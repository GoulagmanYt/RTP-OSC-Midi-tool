import { describe, expect, it } from "vitest";
import {
  BUNDLED_VST_EQ_KEY,
  createDefaultVstEqSettings,
  eqResponseDb,
  frequencyToSlider,
  getVstEqKey,
  sliderToFrequency,
} from "./eq";

describe("VST EQ helpers", () => {
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

  it("is flat while bypassed and attenuates bass when enabled", () => {
    const settings = createDefaultVstEqSettings();
    settings.lowShelf.gainDb = -6;
    expect(eqResponseDb(settings, 30, 48_000)).toBe(0);
    settings.enabled = true;
    expect(eqResponseDb(settings, 30, 48_000)).toBeLessThan(-5.5);
    expect(Math.abs(eqResponseDb(settings, 5_000, 48_000))).toBeLessThan(0.1);
  });
});
