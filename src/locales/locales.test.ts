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
});
