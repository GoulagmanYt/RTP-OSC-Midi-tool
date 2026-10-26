import { describe, expect, it } from "vitest";
import type { AppConfig } from "./api-types";
import { isDeveloperMode, normalizeDeveloperConfig } from "./developer-mode";
import { mergeConfig } from "./providers/bridge/utils";

function configuration(developerMode: boolean, enabled = true): AppConfig {
  return {
    ui: { developerMode },
    logging: { enabled, verbose: true, logAllToFile: true },
    osc: { enabled: true, targetPort: 9000, logMessages: true },
    rtp: { enabled: true, port: 5004, logMessages: true },
    audio: { bufferSize: 512 },
  } as AppConfig;
}

describe("developer capability policy", () => {
  it("requires an explicit developer mode and tolerates loading", () => {
    expect(isDeveloperMode(null)).toBe(false);
    expect(isDeveloperMode(undefined)).toBe(false);
    expect(isDeveloperMode(configuration(false))).toBe(false);
    expect(isDeveloperMode(configuration(true))).toBe(true);
  });

  it.each([true, false])("normalizes imported diagnostics and preserves global logging=%s", (enabled) => {
    const original = configuration(false, enabled);
    const result = normalizeDeveloperConfig(original);
    expect(result.logging).toEqual({ enabled, verbose: false, logAllToFile: false });
    expect(result.osc).toEqual({ ...original.osc, logMessages: false });
    expect(result.rtp).toEqual({ ...original.rtp, logMessages: false });
    expect(result.audio).toBe(original.audio);
    expect(original.logging.verbose).toBe(true);
    expect(normalizeDeveloperConfig(result)).toBe(result);
  });

  it("preserves developer diagnostics", () => {
    const original = configuration(true);
    expect(normalizeDeveloperConfig(original)).toBe(original);
  });

  it("disables all advanced logging together on mode changes and blocks later normal-mode edits", () => {
    const normal = mergeConfig(configuration(true, false), { ui: { developerMode: false } });
    const attempted = mergeConfig(normal, { logging: { verbose: true }, osc: { logMessages: true } });
    expect(attempted.logging).toEqual({ enabled: false, verbose: false, logAllToFile: false });
    expect(attempted.osc.logMessages).toBe(false);
    expect(attempted.rtp.logMessages).toBe(false);
    const developer = mergeConfig(normal, { ui: { developerMode: true }, logging: { verbose: true } });
    expect(developer.logging.verbose).toBe(true);
    expect(developer.logging.enabled).toBe(false);
  });
});
