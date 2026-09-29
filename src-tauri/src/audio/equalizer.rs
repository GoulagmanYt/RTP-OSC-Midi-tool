use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;

use crate::config::VstEqSettings;

const BAND_COUNT: usize = 3;
const COEFFICIENTS_PER_BAND: usize = 5;
const SNAPSHOT_VALUES: usize = 1 + BAND_COUNT * COEFFICIENTS_PER_BAND;
const SMOOTHING_SECONDS: f32 = 0.020;

#[derive(Clone, Copy, Debug, PartialEq)]
struct BiquadCoefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoefficients {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn lerp_towards(&mut self, target: Self, denominator: f32) {
        self.b0 += (target.b0 - self.b0) / denominator;
        self.b1 += (target.b1 - self.b1) / denominator;
        self.b2 += (target.b2 - self.b2) / denominator;
        self.a1 += (target.a1 - self.a1) / denominator;
        self.a2 += (target.a2 - self.a2) / denominator;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EqTarget {
    mix: f32,
    bands: [BiquadCoefficients; BAND_COUNT],
}

impl EqTarget {
    fn from_settings(settings: &VstEqSettings, sample_rate: u32) -> Self {
        let settings = settings.normalized(sample_rate);
        let sample_rate = sample_rate.max(1) as f32;

        Self {
            mix: if settings.enabled { 1.0 } else { 0.0 },
            bands: [
                low_shelf(
                    settings.low_shelf.frequency_hz,
                    settings.low_shelf.gain_db,
                    sample_rate,
                ),
                peaking(
                    settings.mid_peak.frequency_hz,
                    settings.mid_peak.gain_db,
                    settings.mid_peak.q,
                    sample_rate,
                ),
                high_shelf(
                    settings.high_shelf.frequency_hz,
                    settings.high_shelf.gain_db,
                    sample_rate,
                ),
            ],
        }
    }

    fn encode(self) -> [u32; SNAPSHOT_VALUES] {
        let mut values = [0u32; SNAPSHOT_VALUES];
        values[0] = self.mix.to_bits();
        for (band_index, band) in self.bands.iter().enumerate() {
            let offset = 1 + band_index * COEFFICIENTS_PER_BAND;
            values[offset] = band.b0.to_bits();
            values[offset + 1] = band.b1.to_bits();
            values[offset + 2] = band.b2.to_bits();
            values[offset + 3] = band.a1.to_bits();
            values[offset + 4] = band.a2.to_bits();
        }
        values
    }

    fn decode(values: [u32; SNAPSHOT_VALUES]) -> Self {
        let mut bands = [BiquadCoefficients::IDENTITY; BAND_COUNT];
        for (band_index, band) in bands.iter_mut().enumerate() {
            let offset = 1 + band_index * COEFFICIENTS_PER_BAND;
            *band = BiquadCoefficients {
                b0: f32::from_bits(values[offset]),
                b1: f32::from_bits(values[offset + 1]),
                b2: f32::from_bits(values[offset + 2]),
                a1: f32::from_bits(values[offset + 3]),
                a2: f32::from_bits(values[offset + 4]),
            };
        }
        Self {
            mix: f32::from_bits(values[0]),
            bands,
        }
    }
}

/// A single-writer seqlock. The audio thread only performs bounded atomic loads;
/// writers are serialized away from the callback.
pub(super) struct PublishedEq {
    sequence: AtomicU32,
    values: [AtomicU32; SNAPSHOT_VALUES],
    writer: Mutex<()>,
}

impl PublishedEq {
    pub(super) fn new(settings: &VstEqSettings, sample_rate: u32) -> Self {
        let encoded = EqTarget::from_settings(settings, sample_rate).encode();
        Self {
            sequence: AtomicU32::new(0),
            values: std::array::from_fn(|index| AtomicU32::new(encoded[index])),
            writer: Mutex::new(()),
        }
    }

    pub(super) fn publish(&self, settings: &VstEqSettings, sample_rate: u32) {
        let encoded = EqTarget::from_settings(settings, sample_rate).encode();
        let _guard = self.writer.lock();
        let odd = self.sequence.load(Ordering::Relaxed).wrapping_add(1) | 1;
        self.sequence.store(odd, Ordering::Release);
        for (slot, value) in self.values.iter().zip(encoded) {
            slot.store(value, Ordering::Relaxed);
        }
        self.sequence.store(odd.wrapping_add(1), Ordering::Release);
    }

    pub(super) fn load(&self) -> Option<(u32, EqTarget)> {
        let before = self.sequence.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let values = std::array::from_fn(|index| self.values[index].load(Ordering::Relaxed));
        let after = self.sequence.load(Ordering::Acquire);
        (before == after).then(|| (after, EqTarget::decode(values)))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    fn process(&mut self, input: f32, coefficients: BiquadCoefficients) -> f32 {
        let output = coefficients.b0 * input + self.z1;
        self.z1 = coefficients.b1 * input - coefficients.a1 * output + self.z2;
        self.z2 = coefficients.b2 * input - coefficients.a2 * output;
        if self.z1.abs() < 1.0e-20 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1.0e-20 {
            self.z2 = 0.0;
        }
        output
    }
}

pub(super) struct StereoEqualizer {
    revision: u32,
    current: EqTarget,
    target: EqTarget,
    remaining_samples: usize,
    smoothing_samples: usize,
    channels: [[BiquadState; BAND_COUNT]; 2],
}

impl StereoEqualizer {
    pub(super) fn new(revision: u32, target: EqTarget, sample_rate: u32) -> Self {
        Self {
            revision,
            current: target,
            target,
            remaining_samples: 0,
            smoothing_samples: ((sample_rate as f32 * SMOOTHING_SECONDS).round() as usize).max(1),
            channels: [[BiquadState::default(); BAND_COUNT]; 2],
        }
    }

    pub(super) fn begin_block(&mut self, published: &PublishedEq) {
        let Some((revision, target)) = published.load() else {
            return;
        };
        if revision != self.revision {
            self.revision = revision;
            self.target = target;
            self.remaining_samples = self.smoothing_samples;
        }
    }

    pub(super) fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.advance_smoothing();
        (
            self.process_channel(0, left),
            self.process_channel(1, right),
        )
    }

    pub(super) fn process_mono(&mut self, sample: f32) -> f32 {
        self.advance_smoothing();
        self.process_channel(0, sample)
    }

    fn advance_smoothing(&mut self) {
        if self.remaining_samples == 0 {
            return;
        }
        let denominator = self.remaining_samples as f32;
        self.current.mix += (self.target.mix - self.current.mix) / denominator;
        for (current, target) in self.current.bands.iter_mut().zip(self.target.bands) {
            current.lerp_towards(target, denominator);
        }
        self.remaining_samples -= 1;
        if self.remaining_samples == 0 {
            self.current = self.target;
        }
    }

    fn process_channel(&mut self, channel: usize, dry: f32) -> f32 {
        let mut wet = dry;
        for (state, coefficients) in self.channels[channel].iter_mut().zip(self.current.bands) {
            wet = state.process(wet, coefficients);
        }
        dry + (wet - dry) * self.current.mix
    }
}

fn normalize(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> BiquadCoefficients {
    BiquadCoefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

// Coefficients below follow the W3C Audio EQ Cookbook:
// https://www.w3.org/TR/audio-eq-cookbook/
fn peaking(frequency: f32, gain_db: f32, q: f32, sample_rate: f32) -> BiquadCoefficients {
    if gain_db.abs() < f32::EPSILON {
        return BiquadCoefficients::IDENTITY;
    }
    let a = 10.0f32.powf(gain_db / 40.0);
    let omega = std::f32::consts::TAU * frequency / sample_rate;
    let alpha = omega.sin() / (2.0 * q);
    let cosine = omega.cos();
    normalize(
        1.0 + alpha * a,
        -2.0 * cosine,
        1.0 - alpha * a,
        1.0 + alpha / a,
        -2.0 * cosine,
        1.0 - alpha / a,
    )
}

fn low_shelf(frequency: f32, gain_db: f32, sample_rate: f32) -> BiquadCoefficients {
    if gain_db.abs() < f32::EPSILON {
        return BiquadCoefficients::IDENTITY;
    }
    let a = 10.0f32.powf(gain_db / 40.0);
    let omega = std::f32::consts::TAU * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() * std::f32::consts::FRAC_1_SQRT_2;
    let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
    normalize(
        a * ((a + 1.0) - (a - 1.0) * cosine + two_sqrt_a_alpha),
        2.0 * a * ((a - 1.0) - (a + 1.0) * cosine),
        a * ((a + 1.0) - (a - 1.0) * cosine - two_sqrt_a_alpha),
        (a + 1.0) + (a - 1.0) * cosine + two_sqrt_a_alpha,
        -2.0 * ((a - 1.0) + (a + 1.0) * cosine),
        (a + 1.0) + (a - 1.0) * cosine - two_sqrt_a_alpha,
    )
}

fn high_shelf(frequency: f32, gain_db: f32, sample_rate: f32) -> BiquadCoefficients {
    if gain_db.abs() < f32::EPSILON {
        return BiquadCoefficients::IDENTITY;
    }
    let a = 10.0f32.powf(gain_db / 40.0);
    let omega = std::f32::consts::TAU * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() * std::f32::consts::FRAC_1_SQRT_2;
    let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
    normalize(
        a * ((a + 1.0) + (a - 1.0) * cosine + two_sqrt_a_alpha),
        -2.0 * a * ((a - 1.0) + (a + 1.0) * cosine),
        a * ((a + 1.0) + (a - 1.0) * cosine - two_sqrt_a_alpha),
        (a + 1.0) - (a - 1.0) * cosine + two_sqrt_a_alpha,
        2.0 * ((a - 1.0) - (a + 1.0) * cosine),
        (a + 1.0) - (a - 1.0) * cosine - two_sqrt_a_alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn magnitude(coefficients: BiquadCoefficients, frequency: f32, sample_rate: f32) -> f32 {
        let omega = std::f32::consts::TAU * frequency / sample_rate;
        let c1 = omega.cos();
        let s1 = -omega.sin();
        let c2 = (2.0 * omega).cos();
        let s2 = -(2.0 * omega).sin();
        let numerator_re = coefficients.b0 + coefficients.b1 * c1 + coefficients.b2 * c2;
        let numerator_im = coefficients.b1 * s1 + coefficients.b2 * s2;
        let denominator_re = 1.0 + coefficients.a1 * c1 + coefficients.a2 * c2;
        let denominator_im = coefficients.a1 * s1 + coefficients.a2 * s2;
        ((numerator_re * numerator_re + numerator_im * numerator_im)
            / (denominator_re * denominator_re + denominator_im * denominator_im))
            .sqrt()
    }

    #[test]
    fn neutral_settings_are_exactly_flat() {
        let target = EqTarget::from_settings(&VstEqSettings::default(), 48_000);
        assert_eq!(target.bands, [BiquadCoefficients::IDENTITY; BAND_COUNT]);
        assert_eq!(target.mix, 0.0);
    }

    #[test]
    fn low_shelf_cut_reduces_bass_and_preserves_treble() {
        let coefficients = low_shelf(120.0, -6.0, 48_000.0);
        let bass_db = 20.0 * magnitude(coefficients, 30.0, 48_000.0).log10();
        let treble_db = 20.0 * magnitude(coefficients, 5_000.0, 48_000.0).log10();
        assert!((-6.3..=-5.5).contains(&bass_db), "bass response: {bass_db}");
        assert!(treble_db.abs() < 0.05, "treble response: {treble_db}");
    }

    #[test]
    fn invalid_values_are_sanitized_to_finite_coefficients() {
        let defaults = VstEqSettings::default();
        let settings = VstEqSettings {
            enabled: true,
            low_shelf: crate::config::EqShelfSettings {
                frequency_hz: f32::NAN,
                ..defaults.low_shelf
            },
            mid_peak: crate::config::EqPeakSettings {
                q: f32::INFINITY,
                ..defaults.mid_peak
            },
            high_shelf: crate::config::EqShelfSettings {
                gain_db: 1_000.0,
                ..defaults.high_shelf
            },
        };
        let target = EqTarget::from_settings(&settings, 8_000);
        assert!(target
            .encode()
            .into_iter()
            .map(f32::from_bits)
            .all(f32::is_finite));
    }

    #[test]
    fn bypass_returns_the_input_without_channel_crosstalk() {
        let published = PublishedEq::new(&VstEqSettings::default(), 48_000);
        let (revision, target) = published.load().expect("initial EQ snapshot");
        let mut equalizer = StereoEqualizer::new(revision, target, 48_000);
        for _ in 0..64 {
            assert_eq!(equalizer.process_stereo(0.25, -0.75), (0.25, -0.75));
        }
    }

    #[test]
    fn enabled_eq_keeps_stereo_channels_independent_and_mono_finite() {
        let settings = VstEqSettings {
            enabled: true,
            low_shelf: crate::config::EqShelfSettings {
                gain_db: -6.0,
                ..VstEqSettings::default().low_shelf
            },
            ..VstEqSettings::default()
        };
        let published = PublishedEq::new(&settings, 48_000);
        let (revision, target) = published.load().expect("initial EQ snapshot");
        let mut stereo = StereoEqualizer::new(revision, target, 48_000);
        for index in 0..128 {
            let left = if index == 0 { 1.0 } else { 0.0 };
            let (processed_left, processed_right) = stereo.process_stereo(left, 0.0);
            assert!(processed_left.is_finite());
            assert_eq!(processed_right, 0.0);
        }

        let (revision, target) = published.load().expect("initial EQ snapshot");
        let mut mono = StereoEqualizer::new(revision, target, 48_000);
        assert!(mono.process_mono(0.5).is_finite());
    }

    #[test]
    fn published_changes_are_smoothed_to_the_latest_target() {
        let published = PublishedEq::new(&VstEqSettings::default(), 48_000);
        let (revision, target) = published.load().expect("initial EQ snapshot");
        let mut equalizer = StereoEqualizer::new(revision, target, 48_000);
        let enabled = VstEqSettings {
            enabled: true,
            low_shelf: crate::config::EqShelfSettings {
                gain_db: -12.0,
                ..VstEqSettings::default().low_shelf
            },
            ..VstEqSettings::default()
        };
        published.publish(&enabled, 48_000);
        equalizer.begin_block(&published);
        assert_eq!(equalizer.remaining_samples, 960);
        for _ in 0..960 {
            assert!(equalizer.process_mono(0.25).is_finite());
        }
        assert_eq!(equalizer.remaining_samples, 0);
        assert_eq!(equalizer.current, equalizer.target);
    }
}
