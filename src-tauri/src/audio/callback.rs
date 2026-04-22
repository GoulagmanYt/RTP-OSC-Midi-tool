#![allow(deprecated)]

use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};

use cpal::{
    traits::DeviceTrait, FromSample, OutputCallbackInfo, Sample, SizedSample, Stream, StreamConfig,
};
use parking_lot::Mutex;
use rtrb::Consumer;
use vst::{buffer::AudioBuffer, plugin::Plugin};

use crate::logger::background_log;

use super::callback_render::replay_last_output_or_silence as replay_output_or_silence;
use super::{
    callback_midi::{process_pending_vst2_midi, process_pending_vst3_midi, send_reset_messages},
    callback_render::{limit_sample, process_vst3_plugin},
    runtime_state::{AudioCallbackState, AudioError, MidiPacket, PluginBackend},
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
    gain_bits: Arc<AtomicU32>,
    limiter_enabled: Arc<AtomicBool>,
    xruns: Arc<AtomicU32>,
    meter_left: Arc<AtomicU32>,
    meter_right: Arc<AtomicU32>,
    block_size_frames: Arc<AtomicU32>,
    midi_rx: Consumer<MidiPacket>,
    plugin: Arc<Mutex<PluginBackend>>,
    midi_drop_count: Arc<AtomicU32>,
    audio_lock_miss_count: Arc<AtomicU32>,
    emergency_reset_count: Arc<AtomicU32>,
    emergency_reset_requested: Arc<AtomicBool>,
    device_name: &str,
) -> Result<Stream, AudioError> {
    let mut state = AudioCallbackState::new(
        midi_rx,
        plugin_inputs,
        plugin_outputs,
        xruns.clone(),
        block_size_frames,
        limiter_enabled,
        midi_drop_count,
        audio_lock_miss_count,
        emergency_reset_count,
        emergency_reset_requested,
    );
    device
        .build_output_stream(
            cfg,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                audio_callback(
                    data,
                    channels,
                    &gain_bits,
                    &meter_left,
                    &meter_right,
                    &mut state,
                    &plugin,
                    info,
                )
            },
            move |err| {
                xruns.fetch_add(1, Ordering::Relaxed);
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

#[allow(clippy::too_many_arguments)]
fn audio_callback<T: Sample + FromSample<f32>>(
    data: &mut [T],
    device_channels: usize,
    gain_bits: &AtomicU32,
    meter_left: &AtomicU32,
    meter_right: &AtomicU32,
    state: &mut AudioCallbackState,
    plugin: &Arc<Mutex<PluginBackend>>,
    _info: &OutputCallbackInfo,
) {
    let silence = T::from_sample(0.0f32);
    if device_channels == 0 {
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    let frames = data.len() / device_channels;
    if frames == 0 {
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    apply_audio_thread_priority(state);
    state.prepare(frames);
    state.drain_midi();

    let mut plugin = match plugin.try_lock() {
        Some(p) => p,
        None => {
            state.audio_lock_miss_count.fetch_add(1, Ordering::Relaxed);
            replay_output_or_silence(
                data,
                device_channels,
                silence,
                state,
                meter_left,
                meter_right,
            );
            return;
        }
    };

    if state.needs_emergency_reset {
        send_reset_messages(&mut plugin);
        state.needs_emergency_reset = false;
        state.emergency_reset_count.fetch_add(1, Ordering::Relaxed);
    }

    match &mut *plugin {
        PluginBackend::Vst2 { instance } => {
            if state.last_frames != frames {
                instance.set_block_size(frames as i64);
                state
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

                instance.process(&mut buffer);
            }));

            if result.is_err() {
                state.xruns.fetch_add(1, Ordering::Relaxed);
                state.record_error();
                replay_output_or_silence(
                    data,
                    device_channels,
                    silence,
                    state,
                    meter_left,
                    meter_right,
                );
                return;
            }
        }
        PluginBackend::Vst3 {
            instance,
            input_channels,
            output_channels,
        } => {
            if state.last_frames != frames {
                state
                    .block_size_frames
                    .store(frames as u32, Ordering::Relaxed);
                state.last_frames = frames;
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<(), rack::Error> {
                    process_pending_vst3_midi(state, instance)?;
                    process_vst3_plugin(instance, state, frames, *input_channels, *output_channels)
                },
            ));

            match result {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    state.xruns.fetch_add(1, Ordering::Relaxed);
                    state.record_error();
                    replay_output_or_silence(
                        data,
                        device_channels,
                        silence,
                        state,
                        meter_left,
                        meter_right,
                    );
                    return;
                }
            }
        }
    }

    if state.plugin_outputs == 0 {
        data.fill(silence);
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    let gain_linear = f32::from_bits(gain_bits.load(Ordering::Relaxed));
    let limiter_on = state.limiter_enabled.load(Ordering::Relaxed);
    let left = &state.outputs[0];
    let right = if state.plugin_outputs > 1 {
        &state.outputs[1]
    } else {
        &state.outputs[0]
    };

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    if state.last_output.len() != data.len() {
        state.last_output.resize(data.len(), 0.0);
    }
    for (frame_idx, frame) in data.chunks_exact_mut(device_channels).enumerate() {
        let mut l = left[frame_idx] * gain_linear;
        let mut r = right[frame_idx] * gain_linear;
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

    meter_left.store(peak_l.min(1.0).to_bits(), Ordering::Relaxed);
    meter_right.store(peak_r.min(1.0).to_bits(), Ordering::Relaxed);
}
