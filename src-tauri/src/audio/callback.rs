#![allow(deprecated)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use cpal::{
    traits::DeviceTrait, FromSample, OutputCallbackInfo, Sample, SizedSample, Stream, StreamConfig,
};
use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use rtrb::Consumer;
use vst::{buffer::AudioBuffer, plugin::Plugin};

use crate::logger::background_log;

use super::callback_render::replay_last_output_or_silence as replay_output_or_silence;
use super::{
    callback_midi::{process_pending_vst2_midi, process_pending_vst3_midi, send_reset_messages},
    callback_render::{limit_sample, process_vst3_plugin},
    runtime_state::{
        atomic_max, AudioCallbackState, AudioControls, AudioError, AudioTelemetry, MidiPacket,
        ParameterCommand, PluginBackend, DSP_HISTOGRAM_BUCKETS, DSP_HISTOGRAM_BUCKET_US,
    },
    windows_tuning::apply_audio_thread_priority,
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
) -> Result<Stream, AudioError> {
    let mut state = AudioCallbackState::new(
        midi_rx,
        plugin_inputs,
        plugin_outputs,
        max_frames,
        channels,
        Arc::clone(&controls),
        Arc::clone(&telemetry),
        emergency_reset_requested,
        sample_rate,
        parameter_rx,
    );
    let error_telemetry = Arc::clone(&telemetry);
    device
        .build_output_stream(
            cfg,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                audio_callback(data, channels, &mut state, &plugin, info)
            },
            move |err| {
                error_telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                background_log("warn", format!("Audio stream error: {}", err));
            },
            None,
        )
        .map_err(|e| {
            AudioError::Message(format!(
                "Failed to build output stream on '{}': {}",
                device_name, e
            ))
        })
}

fn audio_callback<T: Sample + FromSample<f32>>(
    data: &mut [T],
    device_channels: usize,
    state: &mut AudioCallbackState,
    plugin: &Arc<Mutex<PluginBackend>>,
    _info: &OutputCallbackInfo,
) {
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
        return;
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
        return;
    }

    apply_audio_thread_priority(state);
    if !state.prepare(frames) {
        state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
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
        return;
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
            return;
        }
    };

    if state.needs_emergency_reset {
        send_reset_messages(&mut plugin);
        state.needs_emergency_reset = false;
        state
            .telemetry
            .emergency_reset_count
            .fetch_add(1, Ordering::Relaxed);
    }

    match &mut *plugin {
        PluginBackend::Vst2 { instance } => {
            if state.last_frames != frames {
                // instance.set_block_size(frames as i64); // Removed to prevent VST reset during playback
                state
                    .telemetry
                    .block_size_frames
                    .store(frames as u32, Ordering::Relaxed);
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
                record_dsp_timing(&state.telemetry, dsp_started);
            }));

            if result.is_err() {
                state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                state.record_error();
                replay_output_or_silence(data, device_channels, silence, state);
                finish_callback_timing(state, frames, callback_started);
                return;
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
                }
            }
            if state.last_frames != frames {
                state
                    .telemetry
                    .block_size_frames
                    .store(frames as u32, Ordering::Relaxed);
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
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    state.telemetry.xruns.fetch_add(1, Ordering::Relaxed);
                    state.record_error();
                    replay_output_or_silence(data, device_channels, silence, state);
                    finish_callback_timing(state, frames, callback_started);
                    return;
                }
            }
        }
    }

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
        return;
    }

    let gain_linear = f32::from_bits(state.controls.gain_bits.load(Ordering::Relaxed));
    let limiter_on = state.controls.limiter_enabled.load(Ordering::Relaxed);
    let left = &state.outputs[0];
    let right = if state.plugin_outputs > 1 {
        &state.outputs[1]
    } else {
        &state.outputs[0]
    };

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    debug_assert!(state.last_output.len() >= data.len());
    for (frame_idx, frame) in data.chunks_exact_mut(device_channels).enumerate() {
        let recovery_gain = if state.recovering_from_silence {
            (frame_idx + 1) as f32 / frames as f32
        } else {
            1.0
        };
        let mut l = left[frame_idx] * gain_linear * recovery_gain;
        let mut r = right[frame_idx] * gain_linear * recovery_gain;
        if limiter_on {
            l = limit_sample(l);
            r = limit_sample(r);
        }

        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
        let base = frame_idx * device_channels;
        frame[0] = T::from_sample(l);
        state.last_output[base] = l;
        if device_channels >= 2 {
            frame[1] = T::from_sample(r);
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
    let elapsed_us = started.elapsed().as_micros().min(u32::MAX as u128) as u32;
    state
        .telemetry
        .callback_last_us
        .store(elapsed_us, Ordering::Relaxed);

    atomic_max(&state.telemetry.callback_max_us, elapsed_us);

    if frames > 0 && state.sample_rate > 0 {
        let budget_us = ((frames as u64) * 1_000_000u64 / state.sample_rate as u64) as u32;
        if budget_us > 0 && elapsed_us > budget_us {
            state
                .telemetry
                .callback_over_budget_count
                .fetch_add(1, Ordering::Relaxed);
            state
                .telemetry
                .consecutive_deadline_misses
                .fetch_add(1, Ordering::Relaxed);
        } else {
            state
                .telemetry
                .consecutive_deadline_misses
                .store(0, Ordering::Relaxed);
        }
    }
}
