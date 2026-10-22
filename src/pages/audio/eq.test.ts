import { describe, expect, it } from "vitest";
import reference from "../../../src-tauri/tests/fixtures/eq-response.json";
import {
  BUNDLED_VST_EQ_KEY,
  createDefaultVstEqSettings,
  eqResponseDb,
  frequencyToSlider,
  getVstEqKey,
  sliderToFrequency,
} from "./eq";

describe("VST EQ helpers", () => {
  it.each(reference)("matches the independent analog reference: $name", ({ settings, sampleRate, response }) => {
    for (const { frequencyHz, db } of response) {
      expect(eqResponseDb(settings, frequencyHz, sampleRate)).toBeCloseTo(db, 2);
    }
  });

  it("applies the backend gain, frequency and Q limits before plotting", () => {
    const settings = createDefaultVstEqSettings();
    settings.enabled = true;
    settings.midPeak = { frequencyHz: 50_000, gainDb: 90, q: 0 };
    expect(eqResponseDb(settings, 10_000, 48_000)).toBeCloseTo(18, 3);
    settings.midPeak = { frequencyHz: NaN, gainDb: Infinity, q: -Infinity };
    expect(eqResponseDb(settings, 1_000, 48_000)).toBe(0);
  });

  it("evaluates points above the coefficient guard frequency without flattening the curve", () => {
    const settings = createDefaultVstEqSettings();
    settings.enabled = true;
    settings.highShelf = { frequencyHz: 14_400, gainDb: 12 };
    // Shelf midpoint is 6 dB; the response must keep rising towards Nyquist.
    expect(eqResponseDb(settings, 14_400, 32_000)).toBeCloseTo(6, 3);
    expect(eqResponseDb(settings, 15_500, 32_000)).toBeGreaterThan(11);
  });

  it("uses a stable bundled key when no plugin is selected", () => {
    const audio = {
      enabled: true,
      sampleRate: 48_000,
      bufferSize: 256,
      gainDb: 0,
      limiterEnabled: false,
      vstScanPaths: [],
      vstEqByPlugin: {},
    };
    expect(getVstEqKey(audio, null)).toBe(BUNDLED_VST_EQ_KEY);
    expect(
      getVstEqKey(audio, {
        id: "concert-piano",
        name: "Concert Piano",
        path: "piano.vst3",
        format: "VST3",
        kind: "instrument",
        architecture: "x64",
        availableArchitectures: ["x64"],
        status: "compatible",
        supported: true,
        midiCompatible: true,
        hasEditor: true,
      })
    ).toBe("concert-piano");
  });

  it("round-trips logarithmic frequency slider values", () => {
    const slider = frequencyToSlider(1_000, 100, 10_000);
    expect(sliderToFrequency(slider, 100, 10_000)).toBe(1_000);
  });

  it("handles non-finite, negative, and out-of-range frequency values safely", () => {
    expect(frequencyToSlider(-10, 100, 10_000)).toBe(0);
    expect(frequencyToSlider(NaN, 100, 10_000)).toBe(0);
    expect(frequencyToSlider(50, 100, 10_000)).toBe(0);
    expect(frequencyToSlider(20_000, 100, 10_000)).toBe(1_000);

    expect(sliderToFrequency(-50, 100, 10_000)).toBe(100);
    expect(sliderToFrequency(NaN, 100, 10_000)).toBe(100);
    expect(sliderToFrequency(1_500, 100, 10_000)).toBe(10_000);
  });

  it("is flat while bypassed and attenuates bass when enabled", () => {
    const settings = createDefaultVstEqSettings();
    settings.lowShelf.gainDb = -6;
    expect(eqResponseDb(settings, 30, 48_000)).toBe(0);
    settings.enabled = true;
    expect(eqResponseDb(settings, 30, 48_000)).toBeLessThan(-5.5);
    expect(Math.abs(eqResponseDb(settings, 5_000, 48_000))).toBeLessThan(0.1);
  });
});
