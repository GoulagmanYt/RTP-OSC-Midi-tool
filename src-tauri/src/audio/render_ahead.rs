//! Preallocated audio exchange between the app-owned DSP and driver threads.

use super::runtime_state::{AudioError, AudioTelemetry};
use cpal::{FromSample, Sample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering},
    Arc, OnceLock,
};
use std::thread::Thread;

#[derive(Clone, Copy)]
pub(super) struct RenderRequest {
    pub(super) frames: usize,
    pub(super) source_end_us: u64,
}

struct AudioBlock {
    samples: Vec<f32>,
    request: RenderRequest,
    generation: u64,
    prime_index: Option<usize>,
}

struct Shared {
    phase: AtomicU8, // paused, priming, active, stopped
    anchor_us: AtomicU64,
    frames: AtomicUsize,
    channels: usize,
    max_frames: usize,
    sample_rate: u32,
    blocks: usize,
    telemetry: Arc<AudioTelemetry>,
    emergency: Arc<AtomicBool>,
    thread: OnceLock<Thread>,
}

#[derive(Clone)]
pub(super) struct RenderControl {
    shared: Arc<Shared>,
}
pub(super) struct RenderProducer {
    ready: Producer<AudioBlock>,
    free: Consumer<AudioBlock>,
    pending: Option<AudioBlock>,
    waiting_for_source: bool,
    shared: Arc<Shared>,
}
pub(super) struct AudioOutput {
    ready: Consumer<AudioBlock>,
    free: Producer<AudioBlock>,
    current: Option<AudioBlock>,
    offset: usize,
    last: Vec<f32>,
    recovering: bool,
    last_endpoint: u64,
    generation: u64,
    shared: Arc<Shared>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn channel(
    frames: usize,
    channels: usize,
    max_frames: usize,
    sample_rate: u32,
    blocks: u32,
    telemetry: Arc<AudioTelemetry>,
    emergency: Arc<AtomicBool>,
) -> Result<(RenderProducer, AudioOutput, RenderControl), AudioError> {
    if !(1..=2).contains(&blocks)
        || frames == 0
        || frames > max_frames
        || channels == 0
        || sample_rate == 0
    {
        return Err(AudioError::Message(
            "Invalid render-ahead configuration".into(),
        ));
    }
    let (ready_tx, ready_rx) = RingBuffer::new(blocks as usize);
    let (mut free_tx, free_rx) = RingBuffer::new(blocks as usize);
    for index in 0..blocks as usize {
        free_tx
            .push(AudioBlock {
                samples: vec![0.0; max_frames * channels],
                request: RenderRequest {
                    frames,
                    source_end_us: 0,
                },
                generation: 0,
                prime_index: Some(index),
            })
            .map_err(|_| AudioError::Message("Cannot initialize audio pool".into()))?;
    }
    let shared = Arc::new(Shared {
        phase: AtomicU8::new(0),
        anchor_us: AtomicU64::new(0),
        frames: AtomicUsize::new(frames),
        channels,
        max_frames,
        sample_rate,
        blocks: blocks as usize,
        telemetry,
        emergency,
        thread: OnceLock::new(),
    });
    let producer = RenderProducer {
        ready: ready_tx,
        free: free_rx,
        pending: None,
        waiting_for_source: false,
        shared: shared.clone(),
    };
    let output = AudioOutput {
        ready: ready_rx,
        free: free_tx,
        current: None,
        offset: 0,
        last: vec![0.0; channels],
        recovering: false,
        last_endpoint: 0,
        generation: 0,
        shared: shared.clone(),
    };
    Ok((producer, output, RenderControl { shared }))
}

impl RenderControl {
    pub(super) fn set_frames(&self, frames: usize) {
        if frames > 0 && frames <= self.shared.max_frames {
            self.shared.frames.store(frames, Ordering::Release);
            self.shared
                .telemetry
                .render_ahead_samples
                .store((frames * self.shared.blocks) as u32, Ordering::Relaxed);
        }
    }
    pub(super) fn prime_at(&self, now_us: u64) {
        self.shared.anchor_us.store(now_us, Ordering::Release);
        self.set_frames(self.shared.frames.load(Ordering::Acquire));
        self.shared.phase.store(1, Ordering::Release);
        self.wake();
    }
    pub(super) fn activate(&self) {
        self.shared.phase.store(2, Ordering::Release);
        self.shared
            .telemetry
            .render_running
            .store(true, Ordering::Release);
    }
    pub(super) fn stop(&self) {
        self.shared.phase.store(3, Ordering::Release);
        self.shared
            .telemetry
            .render_running
            .store(false, Ordering::Release);
        self.wake();
    }
    fn wake(&self) {
        if let Some(thread) = self.shared.thread.get() {
            thread.unpark();
        }
    }
}

impl RenderProducer {
    fn idle_wait(&self, now_us: u64) -> std::time::Duration {
        if self.waiting_for_source {
            let remaining = self.pending.as_ref().map_or(0, |block| {
                block.request.source_end_us.saturating_sub(now_us)
            });
            std::time::Duration::from_micros(remaining.min(2_000))
        } else {
            std::time::Duration::from_millis(2)
        }
    }

    pub(super) fn render_next(
        &mut self,
        render: impl FnOnce(&mut [f32], RenderRequest) -> Option<u64>,
    ) -> bool {
        self.render_next_at(super::runtime_state::monotonic_us(), render)
    }

    fn render_next_at(
        &mut self,
        now_us: u64,
        render: impl FnOnce(&mut [f32], RenderRequest) -> Option<u64>,
    ) -> bool {
        self.waiting_for_source = false;
        let phase = self.shared.phase.load(Ordering::Acquire);
        if !(1..=2).contains(&phase)
            || self.shared.telemetry.callback_gate.is_closed()
            || self.ready.is_full()
        {
            return false;
        }
        let Some(mut block) = self.pending.take().or_else(|| self.free.pop().ok()) else {
            return false;
        };
        if let Some(index) = block.prime_index.take() {
            block.request.frames = self.shared.frames.load(Ordering::Acquire);
            let period =
                block.request.frames as u64 * 1_000_000 / u64::from(self.shared.sample_rate);
            // Every primed window is in the past. No MIDI is requested from the future.
            block.request.source_end_us = self
                .shared
                .anchor_us
                .load(Ordering::Acquire)
                .saturating_sub((self.shared.blocks - index) as u64 * period);
        }
        // Resizing can create requests later than the current source clock.
        // Wait on the DSP thread; never synthesize a future MIDI interval.
        if block.request.source_end_us > now_us {
            self.waiting_for_source = true;
            self.pending = Some(block);
            return false;
        }
        let count = block.request.frames * self.shared.channels;
        let generation = render(&mut block.samples[..count], block.request);
        let Some(generation) = generation else {
            self.pending = Some(block);
            return false;
        };
        if generation
            != self
                .shared
                .telemetry
                .midi_generation
                .load(Ordering::Acquire)
            || self.shared.emergency.load(Ordering::Acquire)
        {
            self.pending = Some(block);
            return false;
        }
        block.generation = generation;
        self.shared
            .telemetry
            .render_queue_frames
            .fetch_add(block.request.frames as u64, Ordering::Relaxed);
        if let Err(rtrb::PushError::Full(block)) = self.ready.push(block) {
            self.shared
                .telemetry
                .render_queue_frames
                .fetch_sub(block.request.frames as u64, Ordering::Relaxed);
            self.pending = Some(block);
            return false;
        }
        true
    }
}

impl AudioOutput {
    fn return_current(&mut self, now_us: u64, written: usize, total: usize) -> bool {
        let Some(mut block) = self.current.take() else {
            return true;
        };
        let frames = self.shared.frames.load(Ordering::Acquire);
        let period = frames as u64 * 1_000_000 / u64::from(self.shared.sample_rate);
        let endpoint = now_us.saturating_sub(
            (total - written) as u64 * 1_000_000 / u64::from(self.shared.sample_rate),
        );
        // Preserve distinct source intervals even when a larger callback returns several old blocks.
        let previous = self.last_endpoint.max(
            self.shared
                .anchor_us
                .load(Ordering::Acquire)
                .saturating_sub(period),
        );
        // A just-created monotonic clock has no negative history to prime.
        // Saturation at zero must not force the first live request one period
        // into the future and consume its entire processing budget.
        let minimum =
            if self.last_endpoint == 0 && self.shared.anchor_us.load(Ordering::Acquire) < period {
                0
            } else {
                previous.saturating_add(period)
            };
        self.last_endpoint = endpoint.max(minimum);
        block.request = RenderRequest {
            frames,
            source_end_us: self.last_endpoint,
        };
        match self.free.push(block) {
            Ok(()) => {
                self.offset = 0;
                if let Some(thread) = self.shared.thread.get() {
                    thread.unpark();
                }
                true
            }
            Err(rtrb::PushError::Full(block)) => {
                // Ownership invariant failure: retain storage; never free a Vec in the driver callback.
                self.current = Some(block);
                self.shared
                    .telemetry
                    .stream_fatal_error
                    .store(14, Ordering::Release);
                false
            }
        }
    }

    pub(super) fn copy_into<T: Sample + SizedSample + FromSample<f32>>(
        &mut self,
        data: &mut [T],
        now_us: u64,
    ) -> usize {
        data.fill(T::from_sample(0.0));
        let shared = self.shared.clone();
        let channels = shared.channels;
        let total = data.len() / channels;
        if shared.phase.load(Ordering::Acquire) != 2 {
            return 0;
        }
        let Some(_admission) = shared.telemetry.callback_gate.try_enter() else {
            self.recovering = true;
            return 0;
        };
        if total > shared.max_frames {
            if shared
                .telemetry
                .stream_fatal_error
                .compare_exchange(0, 13, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                shared
                    .telemetry
                    .stream_recovery_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
            shared.emergency.store(true, Ordering::Release);
            return total;
        }
        if total == 0 {
            return 0;
        }
        let generation = shared.telemetry.midi_generation.load(Ordering::Acquire);
        let resetting = generation != self.generation || shared.emergency.load(Ordering::Acquire);
        self.generation = generation;
        if resetting {
            // A fully consumed block can leave history even when ready is empty.
            // Panic invalidates that fallback as well as queued/partial blocks.
            self.last.fill(0.0);
            self.recovering = true;
        }
        shared.frames.store(total, Ordering::Release);
        shared
            .telemetry
            .render_ahead_samples
            .store((total * shared.blocks) as u32, Ordering::Relaxed);
        let mut written = 0;
        let mut discarded = 0;
        while written < total {
            if self.current.is_none() {
                self.current = self.ready.pop().ok();
                self.offset = 0;
            }
            let Some(block) = self.current.as_ref() else {
                break;
            };
            if block.generation != shared.telemetry.midi_generation.load(Ordering::Acquire)
                || shared.emergency.load(Ordering::Acquire)
            {
                let remaining = block.request.frames - self.offset;
                shared
                    .telemetry
                    .render_queue_frames
                    .fetch_sub(remaining as u64, Ordering::Relaxed);
                self.offset = block.request.frames;
                if !self.return_current(now_us, written, total) {
                    break;
                }
                self.recovering = true;
                self.last.fill(0.0);
                discarded += 1;
                if discarded >= shared.blocks {
                    break;
                }
                continue;
            }
            let take = (block.request.frames - self.offset).min(total - written);
            for frame in 0..take {
                let gain = if self.recovering {
                    (written + frame + 1) as f32 / total as f32
                } else {
                    1.0
                };
                for channel in 0..channels {
                    let sample = block.samples[(self.offset + frame) * channels + channel] * gain;
                    data[(written + frame) * channels + channel] =
                        super::callback_render::device_sample(sample);
                    self.last[channel] = sample;
                }
            }
            self.offset += take;
            written += take;
            shared
                .telemetry
                .render_queue_frames
                .fetch_sub(take as u64, Ordering::Relaxed);
            if self.offset == block.request.frames && !self.return_current(now_us, written, total) {
                break;
            }
        }
        let missing = total - written;
        if missing > 0 {
            for frame in 0..missing {
                let gain = 1.0 - (frame + 1) as f32 / missing as f32;
                for channel in 0..channels {
                    data[(written + frame) * channels + channel] =
                        super::callback_render::device_sample(self.last[channel] * gain);
                }
            }
            self.last.fill(0.0);
            self.recovering = true;
            if resetting || discarded > 0 {
                shared
                    .telemetry
                    .render_reset_silence_frames
                    .fetch_add(missing as u64, Ordering::Relaxed);
            } else {
                shared
                    .telemetry
                    .render_underflows
                    .fetch_add(1, Ordering::Relaxed);
                shared
                    .telemetry
                    .render_missing_frames
                    .fetch_add(missing as u64, Ordering::Relaxed);
            }
        } else {
            self.recovering = false;
        }
        missing
    }
}

/// The stream is dropped before this guard, and this guard before native plugin teardown.
pub(super) struct DspWorkerGuard {
    control: RenderControl,
    primed: std::sync::mpsc::Receiver<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl DspWorkerGuard {
    pub(super) fn spawn(
        mut producer: RenderProducer,
        control: RenderControl,
        mut render: impl FnMut(&mut [f32], RenderRequest) -> Option<u64> + Send + 'static,
        priority: super::runtime_state::DspThreadPriority,
    ) -> Result<Self, AudioError> {
        let (sender, primed) = std::sync::mpsc::sync_channel(1);
        let worker = control.clone();
        let thread = std::thread::Builder::new()
            .name("vst-audio-render".into())
            .spawn(move || {
                let _ = worker.shared.thread.set(std::thread::current());
                let _priority = super::windows_tuning::prioritize_render_thread(
                    priority,
                    &worker.shared.telemetry,
                );
                let mut initial = 0;
                while worker.shared.phase.load(Ordering::Acquire) != 3 {
                    let rendered = producer.render_next(|samples, request| {
                        let started = std::time::Instant::now();
                        worker.shared.telemetry.render_busy_since_qpc.store(
                            crate::vst_worker::protocol::monotonic_qpc(),
                            Ordering::Release,
                        );
                        let generation = render(samples, request);
                        let elapsed = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
                        worker
                            .shared
                            .telemetry
                            .render_busy_since_qpc
                            .store(0, Ordering::Release);
                        if generation.is_some() {
                            worker.shared.telemetry.render_last_completed_qpc.store(
                                crate::vst_worker::protocol::monotonic_qpc(),
                                Ordering::Release,
                            );
                            worker
                                .shared
                                .telemetry
                                .render_block_max_us
                                .fetch_max(elapsed, Ordering::Relaxed);
                            if u64::from(elapsed)
                                > request.frames as u64 * 1_000_000
                                    / u64::from(worker.shared.sample_rate)
                            {
                                worker
                                    .shared
                                    .telemetry
                                    .render_over_budget
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        generation
                    });
                    if rendered {
                        if initial < worker.shared.blocks {
                            initial += 1;
                            if initial == worker.shared.blocks {
                                let _ = sender.send(());
                            }
                        }
                    } else {
                        std::thread::park_timeout(
                            producer.idle_wait(super::runtime_state::monotonic_us()),
                        );
                    }
                }
            })
            .map_err(|error| {
                AudioError::Message(format!("Cannot start audio renderer: {error}"))
            })?;
        Ok(Self {
            control,
            primed,
            thread: Some(thread),
        })
    }

    pub(super) fn prime(&self, frames: Option<u32>) -> Result<(), AudioError> {
        if let Some(frames) = frames {
            self.control.set_frames(frames as usize);
        }
        self.control.prime_at(super::runtime_state::monotonic_us());
        self.primed
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| {
                AudioError::Message(format!(
                    "Audio renderer did not prime within 5 seconds: {error}"
                ))
            })?;
        self.control.activate();
        Ok(())
    }
}

impl Drop for DspWorkerGuard {
    fn drop(&mut self) {
        self.control.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};

    fn endpoints(blocks: u32) -> (RenderProducer, AudioOutput, RenderControl) {
        channel(
            4,
            1,
            8,
            1_000,
            blocks,
            Arc::new(AudioTelemetry::new()),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }

    #[test]
    fn paused_renderer_waits_for_restore_and_guard_joins_before_release() {
        let (mut producer, _output, control) = endpoints(1);
        assert!(!producer.render_next_at(u64::MAX, |_, _| panic!("render before restore")));
        let ownership = Arc::new(());
        let captured = ownership.clone();
        let restored = Arc::new(AtomicBool::new(false));
        let restored_worker = restored.clone();
        let guard = DspWorkerGuard::spawn(
            producer,
            control,
            move |samples, _| {
                assert!(restored_worker.load(Ordering::Acquire));
                assert_eq!(Arc::strong_count(&captured), 2);
                samples.fill(0.0);
                Some(0)
            },
            super::super::runtime_state::DspThreadPriority::Mmcss,
        )
        .unwrap();
        restored.store(true, Ordering::Release);
        guard.prime(None).unwrap();
        drop(guard);
        assert_eq!(
            Arc::strong_count(&ownership),
            1,
            "renderer still owns native state after join"
        );
    }

    #[test]
    fn priming_and_live_requests_never_schedule_future_midi() {
        let (mut producer, mut output, control) = endpoints(2);
        control.prime_at(100_000);
        let mut endpoints = Vec::new();
        for _ in 0..2 {
            producer.render_next_at(1_000_000, |samples, request| {
                endpoints.push(request.source_end_us);
                samples.fill(0.0);
                Some(0)
            });
        }
        assert_eq!(endpoints, [92_000, 96_000]);
        control.activate();
        for now in [100_000, 104_000, 108_000] {
            output.copy_into(&mut [0.0f32; 4], now);
            producer.render_next_at(1_000_000, |_, request| {
                assert_eq!(request.source_end_us, now);
                assert_eq!(request.frames, 4);
                Some(0)
            });
        }
        // Two 4-frame buffers returned by an 8-frame callback must not create
        // duplicate 8-frame source intervals. Native rendering waits for source time.
        output.copy_into(&mut [0.0f32; 8], 112_000);
        assert!(!producer.render_next_at(112_000, |_, _| panic!("future MIDI window")));
        assert_eq!(
            producer.idle_wait(115_950),
            std::time::Duration::from_micros(50)
        );
        assert_eq!(producer.idle_wait(116_000), std::time::Duration::ZERO);
        assert!(producer.render_next_at(116_000, |_, request| {
            assert_eq!(request.source_end_us, 116_000);
            assert_eq!(request.frames, 8);
            Some(0)
        }));
        assert!(!producer.render_next_at(116_000, |_, _| panic!("duplicate MIDI window")));
        assert!(producer.render_next_at(124_000, |_, request| {
            assert_eq!(request.source_end_us, 124_000);
            Some(0)
        }));
    }

    #[test]
    fn a_new_source_clock_does_not_delay_the_first_live_render_by_a_quantum() {
        let (mut producer, mut output, control) = endpoints(1);
        control.prime_at(0);
        assert!(producer.render_next_at(0, |samples, _| {
            samples.fill(0.0);
            Some(0)
        }));
        control.activate();
        output.copy_into(&mut [0.0f32; 4], 1_000);
        assert!(
            producer.render_next_at(1_000, |_, request| {
                assert_eq!(request.source_end_us, 1_000);
                Some(0)
            }),
            "the first live render must have a full callback period to finish"
        );
    }

    #[test]
    fn prepared_output_and_recycling_do_not_allocate() {
        let (mut producer, mut output, control) = endpoints(2);
        control.prime_at(100_000);
        for _ in 0..2 {
            producer.render_next_at(100_000, |samples, _| {
                samples.fill(0.25);
                Some(0)
            });
        }
        control.activate();
        let mut data = [0.0f32; 4];
        let allocations = crate::test_alloc::count_allocations(|| {
            for index in 0..1_000 {
                output.copy_into(&mut data, 100_000 + index * 4_000);
                producer.render_next_at(u64::MAX, |samples, _| {
                    samples.fill(0.25);
                    Some(0)
                });
            }
        });
        assert_eq!(allocations, 0);
        assert_eq!(data, [0.25; 4]);
    }

    #[test]
    fn prepared_block_survives_a_producer_pause_and_empty_output_never_waits() {
        let (mut producer, mut output, control) = endpoints(1);
        control.prime_at(100_000);
        assert!(producer.render_next_at(1_000_000, |samples, _| {
            samples.fill(0.5);
            Some(0)
        }));
        control.activate();
        let mut data = [0.0; 4];
        assert_eq!(output.copy_into(&mut data, 104_000), 0);
        assert_eq!(data, [0.5; 4]);
        // Producer stays paused: the consumer must return silence without waiting.
        assert_eq!(output.copy_into(&mut data, 108_000), 4);
        assert_eq!(data[3], 0.0);
        assert_eq!(
            control
                .shared
                .telemetry
                .render_underflows
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn reset_with_an_empty_ready_queue_never_replays_previous_generation() {
        for emergency in [false, true] {
            let (mut producer, mut output, control) = endpoints(1);
            control.prime_at(100_000);
            assert!(producer.render_next_at(100_000, |samples, _| {
                samples.fill(0.8);
                Some(0)
            }));
            control.activate();
            let mut data = [0.0f32; 4];
            output.copy_into(&mut data, 104_000);
            assert_eq!(data, [0.8; 4]);
            if emergency {
                control.shared.emergency.store(true, Ordering::Release);
            } else {
                control
                    .shared
                    .telemetry
                    .midi_generation
                    .store(1, Ordering::Release);
            }
            // The renderer has not produced another block. Even the fallback
            // ramp must discard the old generation immediately on panic.
            assert_eq!(output.copy_into(&mut data, 108_000), 4);
            assert_eq!(data, [0.0; 4]);
            assert_eq!(output.copy_into(&mut data, 112_000), 4);
            assert_eq!(data, [0.0; 4]);
        }
    }

    #[test]
    fn integer_device_output_saturates_unlimited_plugin_samples() {
        let (mut producer, mut output, control) = endpoints(1);
        control.prime_at(100_000);
        assert!(producer.render_next_at(100_000, |samples, _| {
            // These are valid floating output values with the limiter off.
            samples.copy_from_slice(&[2.0, -2.0, 1.0, 0.5]);
            Some(0)
        }));
        control.activate();
        let mut data = [cpal::I24::new(0).unwrap(); 4];
        output.copy_into(&mut data, 104_000);
        assert_eq!(
            data.map(|sample| sample.inner()),
            [8_388_607, -8_388_608, 8_388_607, 4_194_304]
        );
    }

    #[test]
    fn oversized_driver_callback_requests_recovery_once_and_preserves_first_fault() {
        let (_producer, mut output, control) = endpoints(1);
        control.activate();
        let telemetry = &control.shared.telemetry;
        for _ in 0..2 {
            let mut data = [1.0f32; 9];
            assert_eq!(output.copy_into(&mut data, 100_000), 9);
            assert_eq!(data, [0.0; 9]);
        }
        assert_eq!(telemetry.stream_fatal_error.load(Ordering::Acquire), 13);
        assert_eq!(
            telemetry.stream_recovery_requests.load(Ordering::Relaxed),
            1
        );
        assert!(control.shared.emergency.load(Ordering::Acquire));

        telemetry.stream_fatal_error.store(2, Ordering::Release);
        output.copy_into(&mut [0.0f32; 9], 104_000);
        assert_eq!(telemetry.stream_fatal_error.load(Ordering::Acquire), 2);
        assert_eq!(
            telemetry.stream_recovery_requests.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn partial_blocks_keep_order_and_pool_storage_is_reused() {
        let (mut producer, mut output, control) = endpoints(2);
        control.prime_at(100_000);
        let mut value = 1.0;
        let mut pointers = Vec::new();
        for _ in 0..2 {
            assert!(producer.render_next_at(1_000_000, |samples, _| {
                pointers.push(samples.as_ptr());
                samples.fill(value);
                value += 1.0;
                Some(0)
            }));
        }
        assert!(
            !producer.render_next_at(1_000_000, |_, _| panic!("full ready ring must not render"))
        );
        control.activate();
        let mut first = [0.0; 2];
        output.copy_into(&mut first, 102_000);
        assert_eq!(first, [1.0; 2]);
        let mut second = [0.0; 6];
        assert_eq!(output.copy_into(&mut second, 108_000), 0);
        assert_eq!(second, [1.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
        for _ in 0..2 {
            producer.render_next_at(1_000_000, |samples, _| {
                assert!(pointers.contains(&samples.as_ptr()));
                samples.fill(0.0);
                Some(0)
            });
        }
    }

    #[test]
    fn panic_invalidates_a_partial_block_and_a_render_crossing_the_epoch() {
        let (mut producer, mut output, control) = endpoints(1);
        control.prime_at(100_000);
        producer.render_next_at(1_000_000, |samples, _| {
            samples.fill(0.7);
            Some(0)
        });
        control.activate();
        let mut partial = [0.0; 2];
        output.copy_into(&mut partial, 102_000);
        control
            .shared
            .telemetry
            .midi_generation
            .store(1, Ordering::Release);
        output.copy_into(&mut partial, 104_000);
        assert_eq!(partial, [0.0; 2]);
        assert!(!producer.render_next_at(1_000_000, |samples, _| {
            samples.fill(0.9);
            Some(0)
        }));
        assert!(producer.render_next_at(1_000_000, |samples, _| {
            samples.fill(0.4);
            Some(1)
        }));
        // A returned request retains the frame count paired with its source endpoint.
        let mut fresh = [0.0f32; 2];
        assert_eq!(output.copy_into(&mut fresh, 108_000), 0);
        assert!(fresh.iter().all(|sample| *sample >= 0.0 && *sample <= 0.4));
        assert_eq!(fresh[1], 0.4);
    }

    #[test]
    fn maintenance_holds_prepared_audio_and_excludes_new_rendering() {
        let (mut producer, mut output, control) = endpoints(1);
        control.prime_at(100_000);
        producer.render_next_at(1_000_000, |samples, _| {
            samples.fill(0.3);
            Some(0)
        });
        control.activate();
        assert!(control
            .shared
            .telemetry
            .callback_gate
            .close_and_wait(std::time::Duration::ZERO));
        let mut data = [1.0f32; 4];
        output.copy_into(&mut data, 104_000);
        assert_eq!(data, [0.0; 4]);
        assert!(!producer.render_next_at(1_000_000, |_, _| panic!("maintenance must not call DSP")));
        control.shared.telemetry.callback_gate.reopen();
        assert_eq!(output.copy_into(&mut data, 108_000), 0);
        assert_eq!(data[3], 0.3);
    }
}
