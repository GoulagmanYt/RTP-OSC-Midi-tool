import type { AppConfig, DeepPartial } from "../../api";

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function mergeValue<T>(current: T, patch: DeepPartial<T>): T {
  if (patch === undefined) {
    return current;
  }
  if (Array.isArray(patch)) {
    return patch as T;
  }
  if (isPlainObject(current) && isPlainObject(patch)) {
    const next: Record<string, unknown> = { ...current };
    for (const [key, value] of Object.entries(patch)) {
      next[key] = mergeValue((current as Record<string, unknown>)[key], value as never);
    }
    return next as T;
  }
  return patch as T;
}

export function mergeConfig(config: AppConfig, patch: DeepPartial<AppConfig>): AppConfig {
  return mergeValue(config, patch);
}
