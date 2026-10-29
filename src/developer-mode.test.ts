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

  it.each([true, false])("enables basic logs and normalizes imported diagnostics (previous logging=%s)", (enabled) => {
    const original = configuration(false, enabled);
    const result = normalizeDeveloperConfig(original);
    expect(result.logging).toEqual({ enabled: true, verbose: false, logAllToFile: false });
    expect(result.osc).toEqual({ ...original.osc, logMessages: false });
    expect(result.rtp).toEqual({ ...original.rtp, logMessages: false });
    expect(result.audio).toBe(original.audio);
    expect(original.logging.verbose).toBe(true);
    expect(normalizeDeveloperConfig(result)).toBe(result);
  });

  it.each([true, false])("preserves developer logging preferences (enabled=%s)", (enabled) => {
    const original = configuration(true, enabled);
    expect(normalizeDeveloperConfig(original)).toBe(original);
  });

  it("disables all advanced logging together on mode changes and blocks later normal-mode edits", () => {
    const normal = mergeConfig(configuration(true, false), { ui: { developerMode: false } });
    const attempted = mergeConfig(normal, { logging: { verbose: true }, osc: { logMessages: true } });
    expect(attempted.logging).toEqual({ enabled: true, verbose: false, logAllToFile: false });
    expect(attempted.osc.logMessages).toBe(false);
    expect(attempted.rtp.logMessages).toBe(false);
    const developer = mergeConfig(normal, { ui: { developerMode: true }, logging: { enabled: false, verbose: true } });
    expect(developer.logging.verbose).toBe(true);
    expect(developer.logging.enabled).toBe(false);
  });

  it("restores basic logging even when all diagnostic flags are already off", () => {
    const config = configuration(false, false);
    config.logging.verbose = false;
    config.logging.logAllToFile = false;
    config.osc.logMessages = false;
    config.rtp.logMessages = false;
    const normalized = normalizeDeveloperConfig(config);
    expect(normalized.logging.enabled).toBe(true);
    expect(config.logging.enabled).toBe(false);
    expect(mergeConfig(normalized, { logging: { enabled: false } }).logging.enabled).toBe(true);
  });
});
