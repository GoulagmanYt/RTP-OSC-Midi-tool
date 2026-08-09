#![allow(deprecated)]

use super::runtime_state::{timestamp_ms, AudioLifecycleState, AudioRuntime, MidiPacket};
#[cfg(test)]
use super::{
    callback::{midi_to_rack_event, replay_last_output_or_silence, reset_messages_for_channel},
    plugin_host::SimpleHost,
    runtime_state::{
        AudioCallbackState, AudioControls, AudioTelemetry, MAX_PENDING_MIDI,
        MIDI_DRAIN_BUDGET_PER_CALLBACK,
    },
    state_codec::{decode_state, encode_state_chunk, encode_state_params, SavedState},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
};

use parking_lot::Mutex;
#[cfg(test)]
use rack::prelude::MidiEventKind as RackMidiEventKind;
#[cfg(test)]
use rack::prelude::PluginScanner as _;
#[cfg(test)]
use rack::vst3::Vst3Scanner;
#[cfg(test)]
use rack::PluginInstance as _;
use rtrb::Producer;
#[cfg(test)]
use vst::host::PluginLoader;
#[cfg(test)]
use vst::plugin::Plugin;

#[cfg(test)]
use super::stream_config::{buffer_fallback_candidates, choose_buffer_size, max_plugin_block_size};
use crate::logger::background_log;
#[cfg(test)]
use crate::plugin_probe::detect_vst3_channels;

const CRITICAL_MIDI_PUSH_ATTEMPTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MidiSendOutcome {
    Sent,
    DroppedNonCritical,
    CriticalFallbackReset,
    AudioStopped,
}

#[derive(Clone)]
pub struct AudioEngine {
    pub(super) lifecycle_gate: Arc<Mutex<()>>,
    pub(super) lifecycle_state: Arc<AtomicU8>,
    pub(super) runtime: Arc<Mutex<Option<AudioRuntime>>>,
    pub(super) last_vst: Arc<Mutex<Option<PathBuf>>>,
    pub(super) midi_tx: Arc<Mutex<Option<Producer<MidiPacket>>>>,
    pub(super) midi_emergency_reset_requested: Arc<AtomicBool>,
    pub(super) midi_push_log_last_ms: Arc<AtomicU64>,
    pub(super) midi_push_log_accumulator: Arc<AtomicU32>,
    pub(super) midi_drop_log_last_ms: Arc<AtomicU64>,
    pub(super) midi_drop_log_accumulator: Arc<AtomicU32>,
    #[cfg(target_os = "windows")]
    pub(super) worker_enabled: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    pub(super) worker: crate::vst_worker::VstWorkerSupervisor,
}

impl AudioEngine {
    pub fn new() -> Self {
        super::thread_affinity::register_main_ui_thread();
        Self {
            lifecycle_gate: Arc::new(Mutex::new(())),
            lifecycle_state: Arc::new(AtomicU8::new(AudioLifecycleState::Stopped as u8)),
            runtime: Arc::new(Mutex::new(None)),
            last_vst: Arc::new(Mutex::new(None)),
            midi_tx: Arc::new(Mutex::new(None)),
            midi_emergency_reset_requested: Arc::new(AtomicBool::new(false)),
            midi_push_log_last_ms: Arc::new(AtomicU64::new(0)),
            midi_push_log_accumulator: Arc::new(AtomicU32::new(0)),
            midi_drop_log_last_ms: Arc::new(AtomicU64::new(0)),
            midi_drop_log_accumulator: Arc::new(AtomicU32::new(0)),
            #[cfg(target_os = "windows")]
            worker_enabled: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "windows")]
            worker: crate::vst_worker::VstWorkerSupervisor::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.is_ready();
        }
        self.runtime.lock().is_some()
    }

    pub fn is_vst_loaded(&self) -> bool {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.is_ready();
        }
        self.runtime.lock().is_some()
    }

    pub fn send_midi(&self, bytes: &[u8]) {
        let _ = self.send_midi_with_outcome(bytes);
    }

    pub fn send_midi_with_age(&self, bytes: &[u8], age_us: u64) {
        let Some(packet) = MidiPacket::from_bytes_with_age(bytes, age_us) else {
            return;
        };
        let _ = self.send_midi_packet_with_outcome(packet, bytes);
    }

    pub(super) fn send_midi_with_outcome(&self, bytes: &[u8]) -> MidiSendOutcome {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return if self.worker.try_send_midi(bytes) {
                MidiSendOutcome::Sent
            } else {
                MidiSendOutcome::DroppedNonCritical
            };
        }

        let Some(packet) = MidiPacket::from_bytes(bytes) else {
            return MidiSendOutcome::AudioStopped;
        };

        self.send_midi_packet_with_outcome(packet, bytes)
    }

    fn send_midi_packet_with_outcome(
        &self,
        mut packet: MidiPacket,
        bytes: &[u8],
    ) -> MidiSendOutcome {
        let mut attempts = 0usize;
        loop {
            let push_result = {
                let mut guard = self.midi_tx.lock();
                let Some(tx) = guard.as_mut() else {
                    background_log("debug", "send_midi: Audio runtime not started");
                    return MidiSendOutcome::AudioStopped;
                };
                tx.push(packet)
            };

            match push_result {
                Ok(()) => {
                    self.record_midi_push(bytes);
                    return MidiSendOutcome::Sent;
                }
                Err(rtrb::PushError::Full(returned)) => {
                    packet = returned;
                    if !packet.is_critical_release() {
                        self.record_midi_backpressure(bytes);
                        self.record_midi_drop_metric();
                        return MidiSendOutcome::DroppedNonCritical;
                    }
                    if attempts >= CRITICAL_MIDI_PUSH_ATTEMPTS {
                        self.record_midi_backpressure(bytes);
                        self.record_midi_drop_metric();
                        self.midi_emergency_reset_requested
                            .store(true, Ordering::Relaxed);
                        return MidiSendOutcome::CriticalFallbackReset;
                    }
                    attempts += 1;
                    std::thread::yield_now();
                }
            }
        }
    }

    fn record_midi_drop_metric(&self) {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            runtime
                .telemetry
                .midi_drop_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_midi_push(&self, bytes: &[u8]) {
        if !crate::logger::should_log_debug() {
            return;
        }
        let pushed_now = self
            .midi_push_log_accumulator
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let now_ms = timestamp_ms();
        let last_ms = self.midi_push_log_last_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) >= 1000
            && self
                .midi_push_log_last_ms
                .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let pushed_since_last = if pushed_now > 1 {
                self.midi_push_log_accumulator.swap(0, Ordering::Relaxed)
            } else {
                self.midi_push_log_accumulator.store(0, Ordering::Relaxed);
                pushed_now
            };
            background_log(
                "debug",
                format!(
                    "MIDI -> Ring Buffer throughput: {} msg/s (latest={:02X?})",
                    pushed_since_last,
                    &bytes[..bytes.len().min(3)]
                ),
            );
        }
    }

    fn record_midi_backpressure(&self, bytes: &[u8]) {
        if !crate::logger::should_log_debug() {
            return;
        }
        let blocked = self
            .midi_drop_log_accumulator
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let now_ms = timestamp_ms();
        let last_ms = self.midi_drop_log_last_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) >= 1000
            && self
                .midi_drop_log_last_ms
                .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let blocked_since_last = if blocked > 1 {
                self.midi_drop_log_accumulator.swap(0, Ordering::Relaxed)
            } else {
                self.midi_drop_log_accumulator.store(0, Ordering::Relaxed);
                blocked
            };
            background_log(
                "warn",
                format!(
                    "MIDI backpressure before audio callback: {} waits in ~1s (latest={:02X?})",
                    blocked_since_last,
                    &bytes[..bytes.len().min(3)]
                ),
            );
        }
    }

    #[cfg(target_os = "windows")]
    pub(super) fn is_worker_enabled(&self) -> bool {
        self.worker_enabled.load(Ordering::Acquire)
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn is_sforzando_vst3(vst_path: &Path) -> bool {
    let path = vst_path.to_string_lossy().to_lowercase();
    path.ends_with("sforzando.vst3") || path.contains("\\sforzando.vst3")
}

pub(super) fn requires_vst3_destructor_quarantine(vst_path: &Path) -> bool {
    if is_sforzando_vst3(vst_path) {
        return true;
    }
    let path = vst_path.to_string_lossy().to_lowercase();
    path.ends_with(".vst3")
        && (path.contains("\\splice\\") || path.ends_with("splice instrument.vst3"))
}

pub(super) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use rack::prelude::PluginType as RackPluginType;
    use rtrb::RingBuffer;
    use std::{thread, time::Instant};

    fn callback_test_state(
        rx: rtrb::Consumer<MidiPacket>,
        emergency_reset_requested: Arc<AtomicBool>,
    ) -> AudioCallbackState {
        AudioCallbackState::new(
            rx,
            0,
            2,
            1024,
            2,
            Arc::new(AudioControls {
                gain_bits: AtomicU32::new(1.0f32.to_bits()),
                limiter_enabled: AtomicBool::new(false),
            }),
            Arc::new(AudioTelemetry::new()),
            emergency_reset_requested,
            48_000,
            crossbeam_channel::bounded(8).1,
        )
    }

    #[test]
    fn midi_to_rack_event_handles_zero_velocity_note_on() {
        let event = midi_to_rack_event([0x90, 60, 0]).expect("event");
        assert!(matches!(
            event.kind,
            RackMidiEventKind::NoteOff {
                note: 60,
                velocity: 0,
                channel: 0
            }
        ));
    }

    #[test]
    fn state_codec_round_trip_chunk() {
        let payload = vec![1u8, 2, 3, 4, 5];
        let encoded = encode_state_chunk(&payload);
        match decode_state(&encoded) {
            SavedState::Chunk(data) => assert_eq!(data, payload),
            _ => panic!("Expected chunk payload"),
        }
    }

    #[test]
    fn state_codec_round_trip_params() {
        let params = vec![0.0f32, 0.5, 1.0];
        let encoded = encode_state_params(&params);
        match decode_state(&encoded) {
            SavedState::Params(data) => assert_eq!(data, params),
            _ => panic!("Expected params payload"),
        }
    }

    #[test]
    fn state_codec_raw_when_no_header() {
        let payload = vec![9u8, 8, 7];
        match decode_state(&payload) {
            SavedState::Raw(data) => assert_eq!(data, payload),
            _ => panic!("Expected raw payload"),
        }
    }

    #[test]
    fn reset_sequence_includes_sustain_and_panic_controllers() {
        let messages = reset_messages_for_channel(2);
        assert_eq!(
            messages,
            [
                [0xB2, 64, 0],
                [0xB2, 120, 0],
                [0xB2, 121, 0],
                [0xB2, 123, 0],
            ]
        );
    }

    #[test]
    fn emergency_reset_clears_pending_midi() {
        let (_tx, rx) = RingBuffer::new(8);
        let reset_flag = Arc::new(AtomicBool::new(true));
        let mut state = callback_test_state(rx, reset_flag);
        // Pre-fill pending MIDI with note-on events
        for note in 0..10u8 {
            state.pending_midi.push_back(MidiPacket {
                data: [0x90, note, 100],
                len: 3,
                timestamp_us: 0,
            });
        }
        assert_eq!(state.pending_midi.len(), 10);

        // drain_midi should clear pending_midi when emergency_reset_requested is set
        state.drain_midi();

        assert!(
            state.needs_emergency_reset,
            "emergency reset flag should be set"
        );
        assert_eq!(
            state.pending_midi.len(),
            0,
            "pending MIDI must be cleared on emergency reset"
        );
    }

    #[test]
    fn overflow_prefers_dropping_note_on_for_critical_release() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        for _ in 0..MAX_PENDING_MIDI {
            state
                .pending_midi
                .push_back(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("note on"));
        }

        state.enqueue_midi(MidiPacket::from_bytes(&[0x80, 60, 0]).expect("note off"));

        assert_eq!(state.pending_midi.len(), MAX_PENDING_MIDI);
        assert!(state.pending_midi.iter().any(|m| m.is_note_off()));
        assert!(!state.needs_emergency_reset);
    }

    #[test]
    fn stress_audio_ring_buffer_no_loss() {
        const TOTAL: u64 = 500_000;
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(16_384);

        let producer = thread::spawn(move || {
            for i in 0..TOTAL {
                let packet = MidiPacket {
                    data: [0x90, (i % 88) as u8 + 21, 100],
                    len: 3,
                    timestamp_us: i,
                };
                loop {
                    if tx.push(packet).is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        });

        let start = Instant::now();
        let mut received = 0u64;
        let mut next_timestamp = 0u64;
        while received < TOTAL {
            match rx.pop() {
                Ok(packet) => {
                    assert_eq!(
                        packet.timestamp_us, next_timestamp,
                        "audio ring order mismatch"
                    );
                    next_timestamp += 1;
                    received += 1;
                }
                Err(_) => thread::yield_now(),
            }
        }
        let elapsed = start.elapsed();
        producer.join().expect("producer join");
        assert_eq!(received, TOTAL, "audio ring lost midi packets");
        eprintln!(
            "Audio ring stress: {received} packets in {:?} ({:.0} pkt/s)",
            elapsed,
            received as f64 / elapsed.as_secs_f64()
        );
    }

    #[test]
    fn send_midi_drops_non_critical_when_ring_full() {
        let engine = AudioEngine::new();
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(1);
        tx.push(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("first packet"))
            .expect("prefill ring");
        *engine.midi_tx.lock() = Some(tx);

        let outcome = engine.send_midi_with_outcome(&[0x90, 61, 100]);

        assert_eq!(outcome, MidiSendOutcome::DroppedNonCritical);

        let first = rx.pop().expect("first packet still queued");
        assert_eq!(first.data, [0x90, 60, 100]);
        assert!(rx.pop().is_err(), "non-critical note-on must be dropped");
    }

    #[test]
    fn send_midi_preserves_critical_release_or_requests_reset() {
        let engine = AudioEngine::new();
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(1);
        tx.push(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("first packet"))
            .expect("prefill ring");
        *engine.midi_tx.lock() = Some(tx);

        let outcome = engine.send_midi_with_outcome(&[0x80, 60, 0]);

        assert_eq!(outcome, MidiSendOutcome::CriticalFallbackReset);
        assert!(
            engine
                .midi_emergency_reset_requested
                .load(Ordering::Relaxed),
            "critical release overflow should request an emergency reset"
        );

        let first = rx.pop().expect("first packet still queued");
        assert_eq!(first.data, [0x90, 60, 100]);
        assert!(
            rx.pop().is_err(),
            "ring should not receive stale fallback data"
        );
    }

    #[test]
    fn drain_midi_respects_callback_budget() {
        let (mut tx, rx) = RingBuffer::<MidiPacket>::new(MIDI_DRAIN_BUDGET_PER_CALLBACK + 128);
        for i in 0..(MIDI_DRAIN_BUDGET_PER_CALLBACK + 64) {
            tx.push(MidiPacket::from_bytes(&[0x90, (i % 100) as u8, 100]).expect("packet"))
                .expect("push packet");
        }

        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));

        state.drain_midi();

        assert_eq!(state.pending_midi.len(), MIDI_DRAIN_BUDGET_PER_CALLBACK);
        let mut remaining = 0usize;
        while state.midi_rx.pop().is_ok() {
            remaining += 1;
        }
        assert_eq!(remaining, 64);
    }

    #[test]
    fn callback_buffers_are_fixed_after_initialization() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        let input_ptr = state.input_silence.as_ptr();
        let output_ptrs: Vec<_> = state.outputs.iter().map(Vec::as_ptr).collect();
        let pending_capacity = state.pending_midi.capacity();

        assert!(state.prepare(1024));
        assert_eq!(state.input_silence.as_ptr(), input_ptr);
        assert_eq!(
            state.outputs.iter().map(Vec::as_ptr).collect::<Vec<_>>(),
            output_ptrs
        );
        assert_eq!(state.pending_midi.capacity(), pending_capacity);
        assert!(!state.prepare(1025));
    }

    #[test]
    fn choose_buffer_size_respects_requested_when_supported() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(choose_buffer_size(&supported, 128), 128);
        assert_eq!(choose_buffer_size(&supported, 32), 64);
        assert_eq!(choose_buffer_size(&supported, 2048), 1024);
        assert_eq!(choose_buffer_size(&supported, 480), 480);
    }

    #[test]
    fn max_plugin_block_size_covers_supported_range() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(max_plugin_block_size(&supported, 480), 1024);
        assert_eq!(max_plugin_block_size(&supported, 2048), 2048);
    }

    #[test]
    fn buffer_fallback_candidates_are_sorted_around_preferred() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        let c = buffer_fallback_candidates(&supported, 128);
        assert_eq!(c.first().copied(), Some(128));
        assert!(c.contains(&256));
        assert!(c.contains(&480));
        assert!(c.contains(&512));
        assert!(!c.contains(&1536));
    }

    #[test]
    fn replay_last_output_ramps_previous_block_to_silence_on_lock_miss() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        state.last_output = vec![0.25, -0.5, 0.1, -0.2];
        let mut out = vec![0.0f32; 4];

        replay_last_output_or_silence(&mut out, 2, 0.0f32, &mut state);

        assert_eq!(out, vec![0.25, -0.375, 0.05, -0.05]);
        assert!(state.last_output.iter().all(|sample| *sample == 0.0));
        assert!(state.recovering_from_silence);
        assert_eq!(
            f32::from_bits(state.telemetry.meter_left.load(Ordering::Relaxed)),
            0.25
        );
        assert_eq!(
            f32::from_bits(state.telemetry.meter_right.load(Ordering::Relaxed)),
            0.375
        );
    }

    #[test]
    fn replay_last_output_falls_back_to_silence_without_history() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = callback_test_state(rx, Arc::new(AtomicBool::new(false)));
        state.last_output.clear();
        let mut out = vec![1.0f32; 4];

        replay_last_output_or_silence(&mut out, 2, 0.0f32, &mut state);

        assert_eq!(out, vec![0.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            f32::from_bits(state.telemetry.meter_left.load(Ordering::Relaxed)),
            0.0
        );
        assert_eq!(
            f32::from_bits(state.telemetry.meter_right.load(Ordering::Relaxed)),
            0.0
        );
    }

    #[test]
    fn quarantines_known_blocking_vst3_destructors_only() {
        assert!(requires_vst3_destructor_quarantine(Path::new(
            r"C:\Program Files\Common Files\VST3\Splice\Splice INSTRUMENT.vst3"
        )));
        assert!(requires_vst3_destructor_quarantine(Path::new(
            r"C:\VST3\sforzando.vst3"
        )));
        assert!(!requires_vst3_destructor_quarantine(Path::new(
            r"C:\VST3\Other Instrument.vst3"
        )));
        assert!(!requires_vst3_destructor_quarantine(Path::new(
            r"C:\VST2\Splice.dll"
        )));
    }

    #[test]
    #[ignore = "Use the isolated vst_smoke binary for third-party VST2 verification"]
    fn optional_vst2_smoke_test_from_env() {
        let Ok(path) = std::env::var("OSCMIDI_TEST_VST2") else {
            return;
        };

        let vst_path = PathBuf::from(path);
        let host = std::sync::Arc::new(std::sync::Mutex::new(SimpleHost));
        let mut loader =
            PluginLoader::load(&vst_path, host).expect("VST2 smoke: failed to load plugin");
        let mut instance = loader
            .instance()
            .expect("VST2 smoke: failed to instantiate plugin");
        instance.init();
        instance.set_sample_rate(48_000.0);
        instance.set_block_size(128);
        instance.resume();

        let info = instance.get_info();
        assert!(info.outputs > 0, "VST2 smoke: plugin has no output buses");
    }

    #[test]
    #[ignore = "Use the isolated vst_smoke binary for third-party VST3 verification"]
    fn optional_vst3_smoke_test_from_env() {
        let Ok(path) = std::env::var("OSCMIDI_TEST_VST3") else {
            return;
        };

        let vst_path = PathBuf::from(path);
        let scanner = Vst3Scanner::new().expect("VST3 smoke: failed to create scanner");
        let plugins = scanner
            .scan_path(&vst_path)
            .expect("VST3 smoke: failed to scan plugin path");
        let info = plugins
            .into_iter()
            .find(|plugin| {
                matches!(
                    plugin.plugin_type,
                    RackPluginType::Instrument | RackPluginType::Effect
                )
            })
            .expect("VST3 smoke: plugin info not found");
        let mut plugin = scanner
            .load(&info)
            .expect("VST3 smoke: failed to load plugin");
        plugin
            .initialize(48_000.0, 128)
            .expect("VST3 smoke: initialize failed");

        let (input_channels, output_channels) = detect_vst3_channels(&mut plugin, &info, 128)
            .expect("VST3 smoke: channel probe failed");
        eprintln!(
            "VST3 optional smoke channels: inputs={}, outputs={}",
            input_channels, output_channels
        );
    }
}
