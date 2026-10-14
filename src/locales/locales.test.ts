import { describe, expect, it } from "vitest";
import en from "./en";
import fr from "./fr";

function flattenLocale(value: unknown, prefix = ""): Map<string, string> {
  const entries = new Map<string, string>();

  if (typeof value === "string") {
    entries.set(prefix, value);
    return entries;
  }

  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Invalid locale value at ${prefix || "<root>"}`);
  }

  for (const [key, child] of Object.entries(value)) {
    const childPrefix = prefix ? `${prefix}.${key}` : key;
    for (const [childKey, translation] of flattenLocale(child, childPrefix)) {
      entries.set(childKey, translation);
    }
  }

  return entries;
}

describe("locale catalogs", () => {
  it("keeps English and French keys in exact parity", () => {
    const english = flattenLocale(en);
    const french = flattenLocale(fr);

    expect([...french.keys()].sort()).toEqual([...english.keys()].sort());
    expect([...english.values()].every((translation) => translation.trim().length > 0)).toBe(true);
    expect([...french.values()].every((translation) => translation.trim().length > 0)).toBe(true);
  });

  it("contains all required audio.eq translation keys in both languages", () => {
    const requiredEqKeys = [
      "audio.eq.title",
      "audio.eq.description",
      "audio.eq.open",
      "audio.eq.active",
      "audio.eq.enabled",
      "audio.eq.enabledHint",
      "audio.eq.responseGraph",
      "audio.eq.lowBand",
      "audio.eq.midBand",
      "audio.eq.highBand",
      "audio.eq.frequency",
      "audio.eq.gain",
      "audio.eq.q",
      "audio.eq.reset",
    ];

    const english = flattenLocale(en);
    const french = flattenLocale(fr);

    for (const key of requiredEqKeys) {
      expect(english.get(key), `Missing English translation for ${key}`).toBeTruthy();
      expect(french.get(key), `Missing French translation for ${key}`).toBeTruthy();
    }
  });
});

