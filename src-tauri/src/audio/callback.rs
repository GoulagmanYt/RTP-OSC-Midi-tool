#![allow(deprecated)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use cpal::{
    traits::DeviceTrait, ErrorKind, FromSample, OutputCallbackInfo, Sample, SizedSample, Stream,
    StreamConfig,
};
use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use rtrb::Consumer;
use vst::{buffer::AudioBuffer, plugin::Plugin};

use super::callback_render::replay_last_output_or_silence as replay_output_or_silence;
use super::{
    callback_midi::{process_pending_vst2_midi, process_pending_vst3_midi, send_reset_messages},
    callback_render::{device_sample, output_sample, process_vst3_plugin},
    runtime_state::{
        atomic_max, AudioCallbackState, AudioControls, AudioError, AudioTelemetry, MidiPacket,
        ParameterCommand, PluginBackend, DSP_HISTOGRAM_BUCKETS, DSP_HISTOGRAM_BUCKET_US,
    },
    windows_tuning::apply_audio_driver_thread_priority,
};

#[cfg(test)]
pub(super) use super::callback_midi::{midi_to_rack_event, reset_messages_for_channel};
#[cfg(test)]
pub(super) use super::callback_render::replay_last_output_or_silence;

#[allow(clippy::too_many_arguments)]
pub(super) fn build_output_stream_for_sample<T: Sample + SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    cfg: &StreamConfig,
    channels: usize,
    plugin_inputs: usize,
    plugin_outputs: usize,
    controls: Arc<AudioControls>,
    telemetry: Arc<AudioTelemetry>,
    midi_rx: Consumer<MidiPacket>,
    plugin: Arc<Mutex<PluginBackend>>,
    emergency_reset_requested: Arc<AtomicBool>,
    sample_rate: u32,
    max_frames: usize,
    device_name: &str,
    parameter_rx: Receiver<ParameterCommand>,
    render_ahead_blocks: u32,
    priority: super::runtime_state::DspThreadPriority,
    initial_frames: usize,
) -> Result<(Stream, Option<super::render_ahead::DspWorkerGuard>), AudioError> {
    let state = AudioCallbackState::new(
        midi_rx,
        plugin_inputs,
        plugin_outputs,
        max_frames,
        channels,
        Arc::clone(&controls),
        Arc::clone(&telemetry),
        Arc::clone(&emergency_reset_requested),
        sample_rate,
        parameter_rx,
    );
    let mut state = Some(state);
    let (mut output, renderer) = if render_ahead_blocks > 0 {
        let (producer, output, control) = super::render_ahead::channel(
            initial_frames,
            channels,
            max_frames,
            sample_rate,
            render_ahead_blocks,
            telemetry.clone(),
            emergency_reset_requested.clone(),
        )?;
        let render_plugin = plugin.clone();
        let mut state = state.take().expect("renderer state");
        let worker = super::render_ahead::DspWorkerGuard::spawn(
            producer,
            control,
            move |samples, request| {
                state.render_source_end_us = Some(request.source_end_us);
                render_audio_block(samples, channels, &mut state, &render_plugin)
                    .then_some(state.midi_generation)
            },
            priority,
        )?;
        (Some(output), Some(worker))
    } else {
        (None, None)
    };
    let error_telemetry = Arc::clone(&telemetry);
    let error_reset = Arc::clone(&emergency_reset_requested);
    device
        .build_output_stream(
            *cfg,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                let started = Instant::now();
                apply_audio_driver_thread_priority(&telemetry);
                record_driver_callback(&telemetry, info, data.len() / channels, sample_rate);
                if let Some(output) = output.as_mut() {
                    output.copy_into(data, super::runtime_state::monotonic_us());
                    finish_driver_timing(&telemetry, data.len() / channels, sample_rate, started);
                } else if let Some(state) = state.as_mut() {
                    render_audio_block(data, channels, state, &plugin);
                }
            },
            move |err| match err.kind() {
                ErrorKind::Xrun => {
                    error_telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                }
                ErrorKind::RealtimeDenied => {}
                kind => {
                    if kind == ErrorKind::DeviceChanged {
                        error_telemetry
                            .stream_route_changes
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    error_reset.store(true, Ordering::Release);
                    error_telemetry
                        .stream_recovery_requests
                        .fetch_add(1, Ordering::Relaxed);
                    error_telemetry
                        .stream_fatal_error
                        .compare_exchange(
                            0,
                            stream_error_code(kind),
                            Ordering::AcqRel,
                            Ordering::Relaxed,
                        )
                        .ok();
                }
            },
            None,
        )
        .map(|stream| (stream, renderer))
        .map_err(|e| {
            AudioError::Message(format!(
                "Failed to build output stream on '{}': {}",
                device_name, e
            ))
        })
}

fn stream_error_code(kind: ErrorKind) -> u32 {
    match kind {
        ErrorKind::DeviceBusy => 1,
        ErrorKind::DeviceNotAvailable => 2,
        ErrorKind::HostUnavailable => 3,
        ErrorKind::StreamInvalidated => 4,
        ErrorKind::UnsupportedConfig => 5,
        ErrorKind::ResourceExhausted => 6,
        ErrorKind::PermissionDenied => 7,
        ErrorKind::BackendError => 8,
        ErrorKind::UnsupportedOperation => 9,
        ErrorKind::InvalidInput => 10,
        ErrorKind::Other => 11,
        ErrorKind::DeviceChanged => 12,
        ErrorKind::Xrun | ErrorKind::RealtimeDenied => 0,
        _ => 11,
    }
}

fn record_driver_callback(
    telemetry: &AudioTelemetry,
    info: &OutputCallbackInfo,
    frames: usize,
    sample_rate: u32,
) {
    let now = crate::vst_worker::protocol::monotonic_qpc();
    let previous = telemetry.callback_last_qpc.swap(now, Ordering::Relaxed);
    telemetry.callback_count.fetch_add(1, Ordering::Relaxed);
    if previous != 0 {
        telemetry.callback_interval_max_us.fetch_max(
            crate::vst_worker::protocol::qpc_elapsed_us(previous, now),
            Ordering::Relaxed,
        );
    }
    telemetry
        .block_size_frames
        .store(frames as u32, Ordering::Relaxed);
    if previous != 0
        && frames > 0
        && sample_rate > 0
        && crate::vst_worker::protocol::qpc_elapsed_us(previous, now)
            > frames as u64 * 2_000_000 / u64::from(sample_rate)
    {
        telemetry.callback_gap_count.fetch_add(1, Ordering::Relaxed);
    }
    let timestamp = info.timestamp();
    let backend_latency = timestamp
        .playback
        .checked_duration_since(timestamp.callback)
        .map(|duration| duration.as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or(0);
    telemetry
        .backend_latency_us
        .store(backend_latency, Ordering::Relaxed);
}

fn render_audio_block<T: Sample + SizedSample + FromSample<f32>>(
    data: &mut [T],
    device_channels: usize,
    state: &mut AudioCallbackState,
    plugin: &Arc<Mutex<PluginBackend>>,
) -> bool {
    let callback_started = Instant::now();
    let silence = T::from_sample(0.0f32);
    if device_channels == 0 {
        state
            .telemetry
            .meter_left
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        state
            .telemetry
            .meter_right
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        finish_callback_timing(state, 0, callback_started);
        return true;
    }

    let frames = data.len() / device_channels;
    if frames == 0 {
        state
            .telemetry
            .meter_left
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        state
            .telemetry
            .meter_right
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        finish_callback_timing(state, 0, callback_started);
        return true;
    }

    let telemetry = Arc::clone(&state.telemetry);
    let Some(_admission) = telemetry.callback_gate.try_enter() else {
        // CPAL pause is asynchronous. Keep the clock running with silence while
        // control owns the plugin, without ever attempting its mutex.
        data.fill(silence);
        state.last_output.fill(0.0);
        state.recovering_from_silence = true;
        telemetry
            .meter_left
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        telemetry
            .meter_right
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        finish_callback_timing(state, frames, callback_started);
        return false;
    };
    if !state.prepare(frames) {
        state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
        data.fill(silence);
        state.last_output.fill(0.0);
        state.recovering_from_silence = true;
        state
            .telemetry
            .meter_left
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        state
            .telemetry
            .meter_right
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        finish_callback_timing(state, frames, callback_started);
        return true;
    }
    state.drain_midi();

    let mut plugin = match plugin.try_lock() {
        Some(p) => p,
        None => {
            state
                .telemetry
                .audio_lock_miss_count
                .fetch_add(1, Ordering::Relaxed);
            replay_output_or_silence(data, device_channels, silence, state);
            finish_callback_timing(state, frames, callback_started);
            return true;
        }
    };

    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    if state.render_source_end_us.is_some() && !state.needs_emergency_reset {
        if let PluginBackend::Remote { instance } = &mut *plugin {
            if !instance.buffered_output_ready() {
                // Retain this outer request and all MIDI/parameter commands.
                // Advancing the bridge here would discard the unfinished result.
                return false;
            }
        }
    }

    if state.needs_emergency_reset {
        send_reset_messages(&mut plugin);
    }

    match &mut *plugin {
        PluginBackend::Vst2 { instance, time } => {
            state.drain_parameter_commands();
            let parameters = instance.get_parameter_object();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                for command in &state.parameter_commands {
                    parameters.set_parameter(command.index as i32, command.value);
                }
            }));
            if state.last_frames != frames {
                // instance.set_block_size(frames as i64); // Removed to prevent VST reset during playback

                state.last_frames = frames;
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                process_pending_vst2_midi(state, instance);

                let mut buffer = unsafe {
                    AudioBuffer::from_raw(
                        state.plugin_inputs,
                        state.plugin_outputs,
                        state.input_ptrs.as_ptr(),
                        state.output_ptrs.as_mut_ptr(),
                        frames,
                    )
                };

                let dsp_started = Instant::now();
                instance.process(&mut buffer);
                time.advance(frames);
                record_dsp_timing(&state.telemetry, dsp_started);
            }));

            if result.is_ok() {
                acknowledge_parameters(state);
            }
            if result.is_err() {
                fail_parameters(state);
                state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                state.record_error();
                replay_output_or_silence(data, device_channels, silence, state);
                finish_callback_timing(state, frames, callback_started);
                return true;
            }
        }
        PluginBackend::Vst3 {
            instance,
            input_channels,
            output_channels,
        } => {
            state.drain_parameter_commands();
            for command in &state.parameter_commands {
                if instance
                    .set_parameter_audio(command.index, command.value)
                    .is_err()
                {
                    state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                    state
                        .telemetry
                        .parameter_failed_sequence
                        .fetch_max(command.sequence, Ordering::Release);
                }
            }
            if state.last_frames != frames {
                state.last_frames = frames;
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<(), rack::Error> {
                    process_pending_vst3_midi(state, instance)?;
                    let dsp_started = Instant::now();
                    let result = process_vst3_plugin(
                        instance,
                        state,
                        frames,
                        *input_channels,
                        *output_channels,
                    );
                    record_dsp_timing(&state.telemetry, dsp_started);
                    result
                },
            ));

            match result {
                Ok(Ok(())) => acknowledge_parameters(state),
                Ok(Err(_)) | Err(_) => {
                    fail_parameters(state);
                    state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                    state.record_error();
                    replay_output_or_silence(data, device_channels, silence, state);
                    finish_callback_timing(state, frames, callback_started);
                    return true;
                }
            }
        }
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        PluginBackend::Remote { instance } => {
            state.drain_parameter_commands_limit(instance.parameter_capacity());
            if state.last_frames != frames {
                state.last_frames = frames;
            }
            let dsp_started = Instant::now();
            let healthy = instance.realtime_health();
            state
                .telemetry
                .editor_open
                .store(instance.editor_open(), Ordering::Release);
            state
                .telemetry
                .x86_underruns
                .store(healthy.0, Ordering::Relaxed);
            state
                .telemetry
                .x86_overruns
                .store(healthy.1, Ordering::Relaxed);
            state
                .telemetry
                .x86_alive
                .store(healthy.2, Ordering::Relaxed);
            if !instance.process(
                &mut state.pending_midi,
                &state.parameter_commands,
                &mut state.outputs,
                frames,
                Some(&state.telemetry),
                state.midi_rx.peek().ok().map(|packet| packet.timestamp_us),
                state.render_source_end_us,
            ) {
                state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                replay_output_or_silence(data, device_channels, silence, state);
                finish_callback_timing(state, frames, callback_started);
                return true;
            }
            record_dsp_timing(&state.telemetry, dsp_started);
        }
    }

    if state.needs_emergency_reset {
        let pending = match &*plugin {
            PluginBackend::Vst3 { instance, .. } => instance.midi_reset_pending(),
            _ => false,
        };
        if !pending {
            state.needs_emergency_reset = false;
            state
                .telemetry
                .emergency_reset_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    // Native plugin access is complete; conversion and equalization do not need it.
    drop(plugin);

    if state.plugin_outputs == 0 {
        data.fill(silence);
        state
            .telemetry
            .meter_left
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        state
            .telemetry
            .meter_right
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        finish_callback_timing(state, frames, callback_started);
        return true;
    }

    let gain_linear = f32::from_bits(state.controls.gain_bits.load(Ordering::Relaxed));
    let limiter_on = state.controls.limiter_enabled.load(Ordering::Relaxed);
    state.equalizer.begin_block(&state.controls.equalizer);
    let left = &state.outputs[0];
    let right = if state.plugin_outputs > 1 {
        &state.outputs[1]
    } else {
        &state.outputs[0]
    };

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    debug_assert!(state.last_output.len() >= frames.saturating_mul(device_channels));
    for (frame_idx, frame) in data.chunks_exact_mut(device_channels).enumerate() {
        let recovery_gain = if state.recovering_from_silence {
            (frame_idx + 1) as f32 / frames as f32
        } else {
            1.0
        };
        let (equalized_left, equalized_right) = if state.plugin_outputs > 1 {
            state
                .equalizer
                .process_stereo(left[frame_idx], right[frame_idx])
        } else {
            let mono = state.equalizer.process_mono(left[frame_idx]);
            (mono, mono)
        };
        let l = output_sample(equalized_left, gain_linear * recovery_gain, limiter_on);
        let r = output_sample(equalized_right, gain_linear * recovery_gain, limiter_on);

        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
        let base = frame_idx * device_channels;
        frame[0] = device_sample(l);
        state.last_output[base] = l;
        if device_channels >= 2 {
            frame[1] = device_sample(r);
            state.last_output[base + 1] = r;
            for (ch, frame_sample) in frame.iter_mut().enumerate().take(device_channels).skip(2) {
                *frame_sample = silence;
                state.last_output[base + ch] = 0.0;
            }
        }
    }

    let remainder = data.len() % device_channels;
    if remainder != 0 {
        let start = data.len() - remainder;
        for slot in &mut data[start..] {
            *slot = silence;
        }
        for sample in &mut state.last_output[start..] {
            *sample = 0.0;
        }
    }

    // A later, larger callback must not replay the tail of an older block.
    state.last_output[frames * device_channels..].fill(0.0);

    state
        .telemetry
        .meter_left
        .store(peak_l.min(1.0).to_bits(), Ordering::Relaxed);
    state
        .telemetry
        .meter_right
        .store(peak_r.min(1.0).to_bits(), Ordering::Relaxed);
    state.recovering_from_silence = false;
    finish_callback_timing(state, frames, callback_started);
    true
}

fn record_dsp_timing(telemetry: &AudioTelemetry, started: Instant) {
    let elapsed_us = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
    telemetry
        .dsp_process_last_us
        .store(elapsed_us, Ordering::Relaxed);
    atomic_max(&telemetry.dsp_process_max_us, elapsed_us);
    let bucket =
        (elapsed_us / DSP_HISTOGRAM_BUCKET_US).min((DSP_HISTOGRAM_BUCKETS - 1) as u32) as usize;
    telemetry.dsp_histogram[bucket].fetch_add(1, Ordering::Relaxed);
}

fn finish_callback_timing(state: &AudioCallbackState, frames: usize, started: Instant) {
    if state.render_source_end_us.is_none() {
        finish_driver_timing(&state.telemetry, frames, state.sample_rate, started);
    }
}

fn finish_driver_timing(
    telemetry: &AudioTelemetry,
    frames: usize,
    sample_rate: u32,
    started: Instant,
) {
    let elapsed_us = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
    telemetry
        .callback_last_us
        .store(elapsed_us, Ordering::Relaxed);

    atomic_max(&telemetry.callback_max_us, elapsed_us);

    if frames > 0 && sample_rate > 0 {
        let budget_us = ((frames as u64) * 1_000_000u64 / sample_rate as u64) as u32;
        if budget_us > 0 && elapsed_us > budget_us {
            telemetry
                .callback_over_budget_count
                .fetch_add(1, Ordering::Relaxed);
            telemetry
                .consecutive_deadline_misses
                .fetch_add(1, Ordering::Relaxed);
        } else {
            telemetry
                .consecutive_deadline_misses
                .store(0, Ordering::Relaxed);
        }
    }
}

pub(super) fn acknowledge_parameters(state: &AudioCallbackState) {
    if let Some(sequence) = state.parameter_commands.iter().map(|c| c.sequence).max() {
        state
            .telemetry
            .parameter_applied_sequence
            .fetch_max(sequence, Ordering::Release);
    }
}

pub(super) fn fail_parameters(state: &AudioCallbackState) {
    if let Some(sequence) = state
        .parameter_commands
        .iter()
        .map(|command| command.sequence)
        .max()
    {
        state
            .telemetry
            .parameter_failed_sequence
            .fetch_max(sequence, Ordering::Release);
    }
}
