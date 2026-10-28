import css from "./styles.css?raw";
import { describe, expect, it } from "vitest";

const sources = import.meta.glob<string>(["./**/*.tsx", "!./**/*.test.tsx"], {
  query: "?raw", import: "default", eager: true,
});

describe("UI accessibility policy", () => {
  it("keeps component colors semantic and typography relative", () => {
    const violations = Object.entries(sources).flatMap(([path, text]) => {
      const matches = text.match(/(?:bg|text|border|stroke|fill)-(?:red|green|emerald|amber|slate|sky|purple|blue|white|black)(?:-\d+)?\b|text-\[[\d.]+px\]|#[0-9a-fA-F]{3,8}\b|rgba?\(/g);
      return matches ? [`${path}: ${matches.join(", ")}`] : [];
    });
    expect(violations).toEqual([]);
  });
  it("reserves backdrop blur for floating surfaces", () => {
    expect(css).not.toMatch(/\.(?:navigation|toolbar)-material\s*\{[^}]*backdrop-filter:\s*blur/);
  });
});
