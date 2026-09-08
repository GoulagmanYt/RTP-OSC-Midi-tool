import type { AudioConfig, VstEqSettings, VstPluginEntry } from "../../api-types";

export const BUNDLED_VST_EQ_KEY = "__bundled_default__";

export const EQ_RANGES = {
  gain: { min: -18, max: 18, step: 0.1 },
  lowFrequency: { min: 40, max: 500, step: 1 },
  midFrequency: { min: 100, max: 10_000, step: 1 },
  highFrequency: { min: 2_000, max: 16_000, step: 1 },
  q: { min: 0.2, max: 10, step: 0.01 },
} as const;

export function createDefaultVstEqSettings(): VstEqSettings {
  return {
    enabled: false,
    lowShelf: { frequencyHz: 120, gainDb: 0 },
    midPeak: { frequencyHz: 1_000, gainDb: 0, q: 1 },
    highShelf: { frequencyHz: 8_000, gainDb: 0 },
  };
}

export function getVstEqKey(audio: AudioConfig, selectedPlugin: VstPluginEntry | null): string {
  return selectedPlugin?.id || audio.vstPluginId || BUNDLED_VST_EQ_KEY;
}

export function frequencyToSlider(frequency: number, minimum: number, maximum: number): number {
  return (Math.log(frequency / minimum) / Math.log(maximum / minimum)) * 1_000;
}

export function sliderToFrequency(value: number, minimum: number, maximum: number): number {
  return Math.round(minimum * Math.pow(maximum / minimum, value / 1_000));
}

type Biquad = { b0: number; b1: number; b2: number; a1: number; a2: number };

const IDENTITY: Biquad = { b0: 1, b1: 0, b2: 0, a1: 0, a2: 0 };

function normalize(b0: number, b1: number, b2: number, a0: number, a1: number, a2: number): Biquad {
  return { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 };
}

function peak(frequency: number, gainDb: number, q: number, sampleRate: number): Biquad {
  if (gainDb === 0) return IDENTITY;
  const a = Math.pow(10, gainDb / 40);
  const omega = (2 * Math.PI * frequency) / sampleRate;
  const alpha = Math.sin(omega) / (2 * q);
  const cosine = Math.cos(omega);
  return normalize(
    1 + alpha * a,
    -2 * cosine,
    1 - alpha * a,
    1 + alpha / a,
    -2 * cosine,
    1 - alpha / a
  );
}

function shelf(frequency: number, gainDb: number, sampleRate: number, high: boolean): Biquad {
  if (gainDb === 0) return IDENTITY;
  const a = Math.pow(10, gainDb / 40);
  const omega = (2 * Math.PI * frequency) / sampleRate;
  const cosine = Math.cos(omega);
  const alpha = Math.sin(omega) / Math.SQRT2;
  const term = 2 * Math.sqrt(a) * alpha;
  if (high) {
    return normalize(
      a * ((a + 1) + (a - 1) * cosine + term),
      -2 * a * ((a - 1) + (a + 1) * cosine),
      a * ((a + 1) + (a - 1) * cosine - term),
      (a + 1) - (a - 1) * cosine + term,
      2 * ((a - 1) - (a + 1) * cosine),
      (a + 1) - (a - 1) * cosine - term
    );
  }
  return normalize(
    a * ((a + 1) - (a - 1) * cosine + term),
    2 * a * ((a - 1) - (a + 1) * cosine),
    a * ((a + 1) - (a - 1) * cosine - term),
    (a + 1) + (a - 1) * cosine + term,
    -2 * ((a - 1) + (a + 1) * cosine),
    (a + 1) + (a - 1) * cosine - term
  );
}

function magnitude(coefficients: Biquad, frequency: number, sampleRate: number): number {
  const omega = (2 * Math.PI * frequency) / sampleRate;
  const c1 = Math.cos(omega);
  const s1 = -Math.sin(omega);
  const c2 = Math.cos(2 * omega);
  const s2 = -Math.sin(2 * omega);
  const numeratorReal = coefficients.b0 + coefficients.b1 * c1 + coefficients.b2 * c2;
  const numeratorImaginary = coefficients.b1 * s1 + coefficients.b2 * s2;
  const denominatorReal = 1 + coefficients.a1 * c1 + coefficients.a2 * c2;
  const denominatorImaginary = coefficients.a1 * s1 + coefficients.a2 * s2;
  return Math.sqrt(
    (numeratorReal * numeratorReal + numeratorImaginary * numeratorImaginary) /
      (denominatorReal * denominatorReal + denominatorImaginary * denominatorImaginary)
  );
}

export function eqResponseDb(settings: VstEqSettings, frequency: number, sampleRate: number): number {
  if (!settings.enabled) return 0;
  const safeRate = Math.max(sampleRate, 1);
  const safeFrequency = Math.min(frequency, safeRate * 0.45);
  const filters = [
    shelf(settings.lowShelf.frequencyHz, settings.lowShelf.gainDb, safeRate, false),
    peak(settings.midPeak.frequencyHz, settings.midPeak.gainDb, settings.midPeak.q, safeRate),
    shelf(settings.highShelf.frequencyHz, settings.highShelf.gainDb, safeRate, true),
  ];
  const response = filters.reduce(
    (product, coefficients) => product * magnitude(coefficients, safeFrequency, safeRate),
    1
  );
  return 20 * Math.log10(Math.max(response, Number.EPSILON));
}
