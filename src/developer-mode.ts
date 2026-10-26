import type { AppConfig } from "./api-types";

/** The same capability check is used by routes, navigation and diagnostics. */
export function isDeveloperMode(config: Pick<AppConfig, "ui"> | null | undefined): boolean {
  return config?.ui?.developerMode === true;
}

/** Normal mode never carries diagnostic logging flags, including imported drafts. */
export function normalizeDeveloperConfig(config: AppConfig): AppConfig {
  if (isDeveloperMode(config)) return config;
  const loggingChanged = config.logging?.verbose || config.logging?.logAllToFile;
  const oscChanged = config.osc?.logMessages;
  const rtpChanged = config.rtp?.logMessages;
  if (!loggingChanged && !oscChanged && !rtpChanged) return config;
  return {
    ...config,
    logging: loggingChanged ? { ...config.logging, verbose: false, logAllToFile: false } : config.logging,
    osc: oscChanged ? { ...config.osc, logMessages: false } : config.osc,
    rtp: rtpChanged ? { ...config.rtp, logMessages: false } : config.rtp,
  };
}
