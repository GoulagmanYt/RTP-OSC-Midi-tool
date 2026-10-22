use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

use parking_lot::Mutex;

use crate::config::VstEqSettings;

const BAND_COUNT: usize = 3;
const COEFFICIENTS_PER_BAND: usize = 5;
const SNAPSHOT_VALUES: usize = 1 + BAND_COUNT * COEFFICIENTS_PER_BAND;
const SMOOTHING_SECONDS: f64 = 0.020;

#[derive(Clone, Copy, Debug, PartialEq)]
// Low corners at high sample rates lose significant digits through coefficient
// cancellation in f32. Keep both coefficients and histories in double precision.
struct BiquadCoefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl BiquadCoefficients {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn lerp_towards(&mut self, target: Self, denominator: f64) {
        self.b0 += (target.b0 - self.b0) / denominator;
        self.b1 += (target.b1 - self.b1) / denominator;
        self.b2 += (target.b2 - self.b2) / denominator;
        self.a1 += (target.a1 - self.a1) / denominator;
        self.a2 += (target.a2 - self.a2) / denominator;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EqTarget {
    mix: f64,
    bands: [BiquadCoefficients; BAND_COUNT],
}

impl EqTarget {
    fn from_settings(settings: &VstEqSettings, sample_rate: u32) -> Self {
        let settings = settings.normalized(sample_rate);
        let sample_rate = f64::from(sample_rate.max(1));

        Self {
            mix: if settings.enabled { 1.0 } else { 0.0 },
            bands: [
                low_shelf(
                    settings.low_shelf.frequency_hz.into(),
                    settings.low_shelf.gain_db.into(),
                    sample_rate,
                ),
                peaking(
                    settings.mid_peak.frequency_hz.into(),
                    settings.mid_peak.gain_db.into(),
                    settings.mid_peak.q.into(),
                    sample_rate,
                ),
                high_shelf(
                    settings.high_shelf.frequency_hz.into(),
                    settings.high_shelf.gain_db.into(),
                    sample_rate,
                ),
            ],
        }
    }

    fn encode(self) -> [u64; SNAPSHOT_VALUES] {
        let mut values = [0u64; SNAPSHOT_VALUES];
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

    fn decode(values: [u64; SNAPSHOT_VALUES]) -> Self {
        let mut bands = [BiquadCoefficients::IDENTITY; BAND_COUNT];
        for (band_index, band) in bands.iter_mut().enumerate() {
            let offset = 1 + band_index * COEFFICIENTS_PER_BAND;
            *band = BiquadCoefficients {
                b0: f64::from_bits(values[offset]),
                b1: f64::from_bits(values[offset + 1]),
                b2: f64::from_bits(values[offset + 2]),
                a1: f64::from_bits(values[offset + 3]),
                a2: f64::from_bits(values[offset + 4]),
            };
        }
        Self {
            mix: f64::from_bits(values[0]),
            bands,
        }
    }
}

/// A single-writer seqlock. The audio thread only performs bounded atomic loads;
/// writers are serialized away from the callback.
pub(super) struct PublishedEq {
    sequence: AtomicU32,
    values: [AtomicU64; SNAPSHOT_VALUES],
    writer: Mutex<()>,
}

impl PublishedEq {
    pub(super) fn new(settings: &VstEqSettings, sample_rate: u32) -> Self {
        let encoded = EqTarget::from_settings(settings, sample_rate).encode();
        Self {
            sequence: AtomicU32::new(0),
            values: std::array::from_fn(|index| AtomicU64::new(encoded[index])),
            writer: Mutex::new(()),
        }
    }

    pub(super) fn publish(&self, settings: &VstEqSettings, sample_rate: u32) {
        let encoded = EqTarget::from_settings(settings, sample_rate).encode();
        let _guard = self.writer.lock();
        let odd = self.sequence.load(Ordering::Relaxed).wrapping_add(1) | 1;
        self.sequence.store(odd, Ordering::Relaxed);
        fence(Ordering::Release);
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
        fence(Ordering::Acquire);
        let after = self.sequence.load(Ordering::Relaxed);
        (before == after).then(|| (after, EqTarget::decode(values)))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct BiquadState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BiquadState {
    fn process(&mut self, input: f64, coefficients: BiquadCoefficients) -> f64 {
        // Direct form I retains sample histories when coefficients move. The
        // transposed form's delays embed the previous coefficients and inject
        // a transient when reused during a frequency/gain ramp.
        let output =
            coefficients.b0 * input + coefficients.b1 * self.x1 + coefficients.b2 * self.x2
                - coefficients.a1 * self.y1
                - coefficients.a2 * self.y2;
        if !output.is_finite() || output.abs() > f64::from(f32::MAX) {
            *self = Self::default();
            return 0.0;
        }
        self.x2 = self.x1;
        self.x1 = if input.abs() < 1.0e-20 { 0.0 } else { input };
        self.y2 = self.y1;
        self.y1 = if output.abs() < 1.0e-20 { 0.0 } else { output };
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
            smoothing_samples: ((f64::from(sample_rate) * SMOOTHING_SECONDS).round() as usize)
                .max(1),
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
        let denominator = self.remaining_samples as f64;
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
        if !dry.is_finite() {
            self.channels[channel].fill(BiquadState::default());
            return 0.0;
        }
        let dry = f64::from(dry);
        let mut wet = dry;
        for (state, coefficients) in self.channels[channel].iter_mut().zip(self.current.bands) {
            wet = state.process(wet, coefficients);
        }
        // Weighted interpolation avoids overflowing wet - dry for finite inputs.
        let output = (dry * (1.0 - self.current.mix) + wet * self.current.mix) as f32;
        if output.is_finite() {
            output
        } else {
            0.0
        }
    }
}

fn normalize(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> BiquadCoefficients {
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
fn peaking(frequency: f64, gain_db: f64, q: f64, sample_rate: f64) -> BiquadCoefficients {
    if gain_db == 0.0 {
        return BiquadCoefficients::IDENTITY;
    }
    let a = 10.0f64.powf(gain_db / 40.0);
    let omega = std::f64::consts::TAU * frequency / sample_rate;
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

fn low_shelf(frequency: f64, gain_db: f64, sample_rate: f64) -> BiquadCoefficients {
    if gain_db == 0.0 {
        return BiquadCoefficients::IDENTITY;
    }
    let a = 10.0f64.powf(gain_db / 40.0);
    let omega = std::f64::consts::TAU * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() * std::f64::consts::FRAC_1_SQRT_2;
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

fn high_shelf(frequency: f64, gain_db: f64, sample_rate: f64) -> BiquadCoefficients {
    if gain_db == 0.0 {
        return BiquadCoefficients::IDENTITY;
    }
    let a = 10.0f64.powf(gain_db / 40.0);
    let omega = std::f64::consts::TAU * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() * std::f64::consts::FRAC_1_SQRT_2;
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

    #[test]
    fn interrupted_publication_keeps_the_previous_target_without_waiting() {
        let published = PublishedEq::new(&VstEqSettings::default(), 48_000);
        let (revision, target) = published.load().unwrap();
        let mut eq = StereoEqualizer::new(revision, target, 48_000);
        let _writer = published.writer.lock();
        published.sequence.store(1, Ordering::Relaxed);
        published.values[0].store(1.0f64.to_bits(), Ordering::Relaxed);
        assert!(published.load().is_none());
        eq.begin_block(&published);
        assert_eq!(eq.target, target);
        assert_eq!(eq.process_stereo(0.25, -0.5), (0.25, -0.5));
    }

    #[test]
    fn concurrent_publishers_never_expose_a_mixed_snapshot() {
        let neutral = VstEqSettings::default();
        let mut boost = neutral.clone();
        boost.enabled = true;
        boost.low_shelf.gain_db = 18.0;
        boost.mid_peak.gain_db = -12.0;
        boost.high_shelf.gain_db = 9.0;
        let mut cut = boost.clone();
        cut.enabled = false;
        cut.low_shelf.gain_db = -18.0;
        cut.mid_peak.frequency_hz = 4_000.0;
        cut.high_shelf.gain_db = -9.0;
        let allowed =
            [&neutral, &boost, &cut].map(|settings| EqTarget::from_settings(settings, 48_000));
        let published = PublishedEq::new(&neutral, 48_000);
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for settings in [&boost, &cut] {
                let published = &published;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..10_000 {
                        published.publish(settings, 48_000);
                    }
                });
            }
            barrier.wait();
            for _ in 0..50_000 {
                if let Some((revision, target)) = published.load() {
                    assert_eq!(revision & 1, 0);
                    assert!(
                        allowed.contains(&target),
                        "torn coefficient snapshot: {target:?}"
                    );
                }
            }
        });
        assert!(allowed.contains(&published.load().unwrap().1));
    }

    #[test]
    fn rapid_changes_remain_stable_and_return_to_neutral_and_bypass() {
        for rate in [8_000, 44_100, 48_000, 96_000] {
            let mut settings = VstEqSettings {
                enabled: true,
                ..Default::default()
            };
            let published = PublishedEq::new(&settings, rate);
            let (revision, target) = published.load().unwrap();
            let mut eq = StereoEqualizer::new(revision, target, rate);
            for block in 0..200 {
                let gain = if block % 2 == 0 { 18.0 } else { -18.0 };
                settings.low_shelf.gain_db = gain;
                settings.low_shelf.frequency_hz = if block % 3 == 0 { 40.0 } else { 500.0 };
                settings.mid_peak.gain_db = -gain;
                settings.mid_peak.frequency_hz = if block % 3 == 0 { 100.0 } else { 10_000.0 };
                settings.mid_peak.q = if block % 2 == 0 { 0.2 } else { 10.0 };
                settings.high_shelf.gain_db = gain;
                settings.high_shelf.frequency_hz = if block % 3 == 0 { 2_000.0 } else { 16_000.0 };
                settings.enabled = block % 5 != 0;
                published.publish(&settings, rate);
                eq.begin_block(&published);
                for n in 0..64 {
                    let input = 0.01
                        * (std::f32::consts::TAU * 440.0 * (block * 64 + n) as f32 / rate as f32)
                            .sin();
                    let (left, right) = eq.process_stereo(input, 0.0);
                    assert!(
                        left.is_finite() && left.abs() < 10.0,
                        "unstable at {rate}: {left}"
                    );
                    assert_eq!(right, 0.0);
                    for c in eq.current.bands {
                        // Jury stability criterion for each interpolated denominator.
                        assert!(
                            c.a2.abs() < 1.0 && 1.0 + c.a1 + c.a2 > 0.0 && 1.0 - c.a1 + c.a2 > 0.0
                        );
                    }
                }
            }
            let neutral = VstEqSettings {
                enabled: true,
                ..Default::default()
            };
            published.publish(&neutral, rate);
            eq.begin_block(&published);
            for _ in 0..eq.smoothing_samples + 2 {
                eq.process_mono(0.0);
            }
            assert_eq!(eq.process_stereo(0.0, 0.0), (0.0, 0.0));
            assert_eq!(eq.process_stereo(0.125, -0.25), (0.125, -0.25));

            settings.enabled = false;
            published.publish(&settings, rate);
            eq.begin_block(&published);
            for _ in 0..eq.smoothing_samples {
                eq.process_mono(0.1);
            }
            assert_eq!(eq.process_stereo(0.125, -0.25), (0.125, -0.25));
        }
    }

    #[test]
    fn bypass_toggle_preserves_a_warm_filter_and_has_a_bounded_ramp() {
        let mut settings = VstEqSettings {
            enabled: true,
            ..Default::default()
        };
        settings.low_shelf.gain_db = 18.0;
        let published = PublishedEq::new(&settings, 48_000);
        let (revision, target) = published.load().unwrap();
        let mut eq = StereoEqualizer::new(revision, target, 48_000);
        for _ in 0..48_000 {
            eq.process_mono(0.1);
        }
        let mut previous = eq.process_mono(0.1);
        for enabled in [false, true, false] {
            settings.enabled = enabled;
            published.publish(&settings, 48_000);
            eq.begin_block(&published);
            for _ in 0..eq.smoothing_samples {
                let output = eq.process_mono(0.1);
                assert!(
                    (output - previous).abs() < 0.001,
                    "bypass step: {previous} -> {output}"
                );
                previous = output;
            }
            let expected = if enabled { 0.794_328_2 } else { 0.1 };
            assert!((previous - expected).abs() < 0.00001);
        }
    }

    #[test]
    fn recreated_filter_uses_new_rate_and_does_not_inherit_the_previous_instrument() {
        let mut settings = VstEqSettings {
            enabled: true,
            ..Default::default()
        };
        settings.mid_peak.gain_db = 18.0;
        settings.mid_peak.frequency_hz = 100.0;
        settings.mid_peak.q = 10.0;
        let mut old = StereoEqualizer::new(0, EqTarget::from_settings(&settings, 96_000), 96_000);
        old.process_stereo(1.0, -1.0);
        let mut new = StereoEqualizer::new(0, EqTarget::from_settings(&settings, 8_000), 8_000);
        assert_eq!(new.process_stereo(0.0, 0.0), (0.0, 0.0));
        assert_eq!(new.smoothing_samples, 160);
        let mut energy_in = 0.0f64;
        let mut energy_out = 0.0f64;
        for n in 0..32_000 {
            let input = 0.01 * (std::f64::consts::TAU * 100.0 * n as f64 / 8_000.0).sin();
            let output = new.process_mono(input as f32);
            if n >= 16_000 {
                energy_in += input * input;
                energy_out += f64::from(output).powi(2);
            }
        }
        assert!((10.0 * (energy_out / energy_in).log10() - 18.0).abs() < 0.01);
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReferenceCase {
        name: String,
        sample_rate: u32,
        settings: VstEqSettings,
        response: Vec<ReferencePoint>,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ReferencePoint {
        frequency_hz: f64,
        db: f64,
    }

    #[test]
    fn measured_impulses_match_independent_analog_reference() {
        // Fixtures come from analog prototypes with BLT prewarping, not from
        // the digital coefficients under test (tests/fixtures/generate_eq_reference.py).
        let cases: Vec<ReferenceCase> =
            serde_json::from_str(include_str!("../../tests/fixtures/eq-response.json")).unwrap();
        for case in cases {
            let mut eq = StereoEqualizer::new(
                0,
                EqTarget::from_settings(&case.settings, case.sample_rate),
                case.sample_rate,
            );
            let mut dft = vec![(0.0f64, 0.0f64); case.response.len()];
            for n in 0..131_072 {
                let (left, right) = eq.process_stereo(if n == 0 { 1.0 } else { 0.0 }, 0.0);
                assert_eq!(right, 0.0, "cross-channel state: {}", case.name);
                assert!(left.is_finite());
                for (point, (re, im)) in case.response.iter().zip(&mut dft) {
                    let angle = std::f64::consts::TAU * point.frequency_hz * n as f64
                        / case.sample_rate as f64;
                    *re += f64::from(left) * angle.cos();
                    *im -= f64::from(left) * angle.sin();
                }
            }
            for (point, (re, im)) in case.response.iter().zip(dft) {
                let measured = 20.0 * re.hypot(im).log10();
                assert!(
                    (measured - point.db).abs() < 0.01,
                    "{} at {} Hz: measured {measured}, expected {}",
                    case.name,
                    point.frequency_hz,
                    point.db
                );
            }
        }
    }

    #[test]
    fn moving_a_peak_on_dc_does_not_create_audio_transients() {
        let settings = VstEqSettings {
            enabled: true,
            ..Default::default()
        };
        let published = PublishedEq::new(&settings, 48_000);
        let (revision, target) = published.load().unwrap();
        let mut eq = StereoEqualizer::new(revision, target, 48_000);
        for _ in 0..4_800 {
            eq.process_mono(0.1);
        }
        let mut changed = settings;
        changed.mid_peak.gain_db = 18.0;
        changed.mid_peak.q = 10.0;
        let mut max_error = 0.0f32;
        for frequency in [100.0, 10_000.0, 100.0] {
            changed.mid_peak.frequency_hz = frequency;
            published.publish(&changed, 48_000);
            eq.begin_block(&published);
            for _ in 0..24_000 {
                max_error = max_error.max((eq.process_mono(0.1) - 0.1).abs());
            }
        }
        // A peaking EQ has unity DC gain at every point of the ramp.
        assert!(max_error < 0.001, "DC transient: {max_error}");
    }

    fn magnitude(coefficients: BiquadCoefficients, frequency: f64, sample_rate: f64) -> f64 {
        let omega = std::f64::consts::TAU * frequency / sample_rate;
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
    fn invalid_plugin_samples_do_not_poison_subsequent_audio() {
        for enabled in [false, true] {
            for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let settings = VstEqSettings {
                    enabled,
                    ..VstEqSettings::default()
                };
                let target = EqTarget::from_settings(&settings, 48_000);
                let mut eq = StereoEqualizer::new(0, target, 48_000);
                assert_eq!(eq.process_stereo(invalid, 0.25), (0.0, 0.25));
                assert_eq!(eq.process_stereo(0.5, 0.25), (0.5, 0.25));
            }
        }
    }

    #[test]
    fn finite_plugin_overflow_does_not_poison_filter_state() {
        let settings = VstEqSettings {
            enabled: true,
            high_shelf: crate::config::EqShelfSettings {
                gain_db: 12.0,
                ..VstEqSettings::default().high_shelf
            },
            ..VstEqSettings::default()
        };
        let mut eq = StereoEqualizer::new(0, EqTarget::from_settings(&settings, 48_000), 48_000);
        assert!(eq.process_mono(f32::MAX).is_finite());
        for _ in 0..1024 {
            assert!(eq.process_mono(0.0).is_finite());
        }
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
            .map(f64::from_bits)
            .all(f64::is_finite));
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
