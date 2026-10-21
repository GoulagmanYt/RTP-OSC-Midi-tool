#![allow(deprecated)]

use std::sync::atomic::Ordering;

use cpal::{FromSample, Sample, SizedSample};
use rack::PluginInstance as _;
use smallvec::SmallVec;

use super::runtime_state::{AudioCallbackState, MAX_PLUGIN_CHANNELS};

pub(super) fn replay_last_output_or_silence<T: Sample + SizedSample + FromSample<f32>>(
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
    let frames = data.len() / device_channels;
    for (idx, slot) in data.iter_mut().enumerate() {
        // Every channel in a frame must receive the same gain, and the final
        // frame must reach silence before a subsequent missed callback.
        let ramp = if frames > 1 {
            1.0 - ((idx / device_channels).min(frames - 1) as f32 / (frames - 1) as f32)
        } else {
            0.0
        };
        let sample = state.last_output.get(idx).copied().unwrap_or(0.0) * ramp;
        *slot = device_sample(sample);
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
    state.last_output.fill(0.0);
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
            let mut inputs: SmallVec<[&[f32]; MAX_PLUGIN_CHANNELS]> = SmallVec::new();
            for _ in 0..input_channels {
                inputs.push(input_slice);
            }
            let mut outputs: SmallVec<[&mut [f32]; MAX_PLUGIN_CHANNELS]> = SmallVec::new();
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

pub(super) fn output_sample(sample: f32, gain: f32, limiter: bool) -> f32 {
    let scaled = sample * gain;
    if !scaled.is_finite() {
        return 0.0;
    }
    if limiter {
        limit_sample(scaled)
    } else {
        scaled
    }
}

pub(super) fn device_sample<T: SizedSample + FromSample<f32>>(sample: f32) -> T {
    let sample = if sample.is_finite() { sample } else { 0.0 };
    // dasp's integer conversions require -1 <= sample < 1. In particular,
    // I24::from_sample does not saturate and can construct an invalid value.
    // Floating devices retain their headroom when the limiter is disabled.
    let sample = if T::FORMAT.is_float() {
        sample
    } else {
        sample.clamp(-1.0, f32::from_bits(1.0f32.to_bits() - 1))
    };
    T::from_sample(sample)
}

#[cfg(test)]
mod tests {
    use super::{device_sample, output_sample};

    #[test]
    fn device_conversion_preserves_float_headroom_and_bounds_integer_pcm() {
        for sample in [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0] {
            assert_eq!(device_sample::<f32>(sample), sample);
            assert_eq!(device_sample::<f64>(sample), f64::from(sample));
        }
        assert_eq!(device_sample::<i16>(2.0), i16::MAX);
        assert_eq!(device_sample::<i16>(-2.0), i16::MIN);
        assert_eq!(device_sample::<u16>(2.0), u16::MAX);
        assert_eq!(device_sample::<u16>(-2.0), u16::MIN);
        assert_eq!(device_sample::<u16>(0.0), 32_768);
        assert_eq!(device_sample::<i32>(-2.0), i32::MIN);
        assert!(device_sample::<i32>(2.0) > 2_147_483_000);
        for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(device_sample::<f32>(sample), 0.0);
            assert_eq!(device_sample::<u16>(sample), 32_768);
            assert_eq!(device_sample::<cpal::I24>(sample).inner(), 0);
        }
    }

    #[test]
    fn output_gain_cannot_send_nonfinite_samples_to_the_device() {
        for limiter in [false, true] {
            for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
                assert_eq!(output_sample(sample, 4.0, limiter), 0.0);
            }
            assert_eq!(output_sample(0.25, 2.0, limiter), 0.5);
        }
        assert_eq!(output_sample(1.0, 2.0, false), 2.0);
        assert_eq!(output_sample(1.0, 2.0, true), 0.98);
        assert_eq!(output_sample(-1.0, 2.0, true), -0.98);
    }
}
