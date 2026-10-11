import { describe, expect, it } from "vitest";
import { formatNote, displayPortName } from "./shared";
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
