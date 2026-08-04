#![allow(deprecated)]

use std::sync::atomic::Ordering;

use cpal::{FromSample, Sample};
use rack::PluginInstance as _;
use smallvec::SmallVec;

use super::runtime_state::AudioCallbackState;

pub(super) fn replay_last_output_or_silence<T: Sample + FromSample<f32>>(
    data: &mut [T],
    device_channels: usize,
    silence: T,
    state: &mut AudioCallbackState,
) {
    if state.last_output.is_empty() || device_channels == 0 {
        data.fill(silence);
        state
            .telemetry
            .meter_left
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        state
            .telemetry
            .meter_right
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    let sample_count = data.len().max(1) as f32;
    for (idx, slot) in data.iter_mut().enumerate() {
        let ramp = 1.0 - (idx as f32 / sample_count);
        let sample = state.last_output.get(idx).copied().unwrap_or(0.0) * ramp;
        *slot = T::from_sample(sample);
        match idx % device_channels {
            0 => peak_l = peak_l.max(sample.abs()),
            1 => peak_r = peak_r.max(sample.abs()),
            _ => {}
        }
    }
    if device_channels == 1 {
        peak_r = peak_l;
    }
    state
        .telemetry
        .meter_left
        .store(peak_l.min(1.0).to_bits(), Ordering::Relaxed);
    state
        .telemetry
        .meter_right
        .store(peak_r.min(1.0).to_bits(), Ordering::Relaxed);
    let rendered = data.len().min(state.last_output.len());
    state.last_output[..rendered].fill(0.0);
    state.recovering_from_silence = true;
}

pub(super) fn process_vst3_plugin(
    plugin: &mut rack::vst3::Vst3Plugin,
    state: &mut AudioCallbackState,
    frames: usize,
    input_channels: usize,
    output_channels: usize,
) -> Result<(), rack::Error> {
    let input_slice = &state.input_silence[..frames];

    match (input_channels, output_channels) {
        (0, 1) => {
            let mut outputs = [&mut state.outputs[0][..frames]];
            plugin.process(&[], &mut outputs, frames)
        }
        (0, 2) => {
            let (left, right) = state.outputs.split_at_mut(1);
            let mut outputs = [&mut left[0][..frames], &mut right[0][..frames]];
            plugin.process(&[], &mut outputs, frames)
        }
        (1, 1) => {
            let inputs = [input_slice];
            let mut outputs = [&mut state.outputs[0][..frames]];
            plugin.process(&inputs, &mut outputs, frames)
        }
        (2, 2) => {
            let inputs = [input_slice, input_slice];
            let (left, right) = state.outputs.split_at_mut(1);
            let mut outputs = [&mut left[0][..frames], &mut right[0][..frames]];
            plugin.process(&inputs, &mut outputs, frames)
        }
        _ => {
            let mut inputs: SmallVec<[&[f32]; 64]> = SmallVec::new();
            for _ in 0..input_channels {
                inputs.push(input_slice);
            }
            let mut outputs: SmallVec<[&mut [f32]; 64]> = SmallVec::new();
            for output in state.outputs.iter_mut().take(output_channels) {
                outputs.push(&mut output[..frames]);
            }
            plugin.process(&inputs, &mut outputs, frames)
        }
    }
}

pub(super) fn limit_sample(sample: f32) -> f32 {
    const LIMIT: f32 = 0.98;
    sample.clamp(-LIMIT, LIMIT)
}
