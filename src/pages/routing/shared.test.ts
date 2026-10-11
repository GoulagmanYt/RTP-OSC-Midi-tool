import { describe, expect, it } from "vitest";
import { formatNote, displayPortName, clampMidiNumber, clampNullableMidiNumber, clampAudioGain } from "./shared";
import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../../constants";

describe("formatNote", () => {
  it("formats standard MIDI note numbers to name and octave", () => {
    expect(formatNote(0)).toBe("C-1");
    expect(formatNote(12)).toBe("C0");
    expect(formatNote(60)).toBe("C4");
    expect(formatNote(69)).toBe("A4");
    expect(formatNote(127)).toBe("G9");
  });

  it("handles floating point values by flooring", () => {
    expect(formatNote(60.8)).toBe("C4");
  });

  it("returns placeholder for null, undefined, non-finite, and out-of-range notes", () => {
    expect(formatNote(null)).toBe("--");
    expect(formatNote(undefined)).toBe("--");
    expect(formatNote(NaN)).toBe("--");
    expect(formatNote(Infinity)).toBe("--");
    expect(formatNote(-1)).toBe("--");
    expect(formatNote(-100)).toBe("--");
    expect(formatNote(128)).toBe("--");
  });
});

describe("displayPortName", () => {
  const t = (key: string) => `translated:${key}`;

  it("translates virtual and internal ports", () => {
    expect(displayPortName(RTP_VIRTUAL_INPUT, t)).toBe("translated:devices.rtpVirtualInput");
    expect(displayPortName(VST_OUTPUT, t)).toBe("translated:devices.vstInternalOutput");
    expect(displayPortName("External Hardware", t)).toBe("External Hardware");
  });
});

describe("clampMidiNumber", () => {
  it("clamps valid values within range", () => {
    expect(clampMidiNumber(10, 0, 127)).toBe(10);
    expect(clampMidiNumber(0, 0, 127)).toBe(0);
    expect(clampMidiNumber(127, 0, 127)).toBe(127);
    expect(clampMidiNumber(150, 0, 127)).toBe(127);
    expect(clampMidiNumber(-5, 0, 127)).toBe(0);
  });

  it("parses string representations and floors floats", () => {
    expect(clampMidiNumber("60", 0, 127)).toBe(60);
    expect(clampMidiNumber("60.9", 0, 127)).toBe(60);
    expect(clampMidiNumber("-10", 0, 127)).toBe(0);
  });

  it("falls back to default on invalid or empty values", () => {
    expect(clampMidiNumber("", 0, 127, 0)).toBe(0);
    expect(clampMidiNumber(null, 0, 127, 10)).toBe(10);
    expect(clampMidiNumber(undefined, 0, 127, 20)).toBe(20);
    expect(clampMidiNumber("invalid", 0, 127, 5)).toBe(5);
    expect(clampMidiNumber(NaN, 0, 127, 5)).toBe(5);
    expect(clampMidiNumber(Infinity, 0, 127, 5)).toBe(5);
  });
});

describe("clampNullableMidiNumber", () => {
  it("returns null for empty, null, undefined, or non-finite inputs", () => {
    expect(clampNullableMidiNumber("", 1, 16)).toBeNull();
    expect(clampNullableMidiNumber(null, 1, 16)).toBeNull();
    expect(clampNullableMidiNumber(undefined, 1, 16)).toBeNull();
    expect(clampNullableMidiNumber("abc", 1, 16)).toBeNull();
    expect(clampNullableMidiNumber(NaN, 1, 16)).toBeNull();
    expect(clampNullableMidiNumber(Infinity, 1, 16)).toBeNull();
  });

  it("clamps valid numbers within bounds", () => {
    expect(clampNullableMidiNumber(5, 1, 16)).toBe(5);
    expect(clampNullableMidiNumber(0, 1, 16)).toBe(1);
    expect(clampNullableMidiNumber(20, 1, 16)).toBe(16);
    expect(clampNullableMidiNumber("10", 1, 16)).toBe(10);
    expect(clampNullableMidiNumber("10.8", 1, 16)).toBe(10);
  });
});

describe("clampAudioGain", () => {
  it("clamps valid gain within min and max", () => {
    expect(clampAudioGain(0)).toBe(0);
    expect(clampAudioGain(-10, -60, 12)).toBe(-10);
    expect(clampAudioGain(-70, -60, 12)).toBe(-60);
    expect(clampAudioGain(20, -60, 12)).toBe(12);
  });

  it("handles string values and fallback on invalid inputs", () => {
    expect(clampAudioGain("-6.5")).toBe(-6.5);
    expect(clampAudioGain("", -60, 12, 0)).toBe(0);
    expect(clampAudioGain(null, -60, 12, 0)).toBe(0);
    expect(clampAudioGain("foo", -60, 12, -3)).toBe(-3);
    expect(clampAudioGain(NaN, -60, 12, 0)).toBe(0);
  });
});

