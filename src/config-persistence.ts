import type { AppConfig } from "./api-types";

let pendingSave: Promise<void> = Promise.resolve();
const automaticSaves = new Set<{ signature: string; cancelled: boolean }>();

/** Preserve invocation order across automatic and explicit configuration saves. */
export function persistConfigSerially<T>(
  config: AppConfig,
  operation: (config: AppConfig) => Promise<T>
): Promise<T> {
  const result = pendingSave.then(() => operation(config));
  pendingSave = result.then(() => undefined, () => undefined);
  return result;
}

/** Cancel queued retries of a rejected snapshot before React can publish its rollback. */
export function persistConfigSave(
  config: AppConfig,
  operation: (config: AppConfig) => Promise<void>,
  shouldSave?: () => boolean
): Promise<boolean> {
  const signature = JSON.stringify(config);
  const automatic = shouldSave ? { signature, cancelled: false } : null;
  if (automatic) automaticSaves.add(automatic);
  return persistConfigSerially(config, async (next) => {
    try {
      if (automatic?.cancelled || (shouldSave && !shouldSave())) return false;
      await operation(next);
      return true;
    } catch (error) {
      for (const queued of automaticSaves) {
        if (queued.signature === signature) queued.cancelled = true;
      }
      throw error;
    } finally {
      if (automatic) automaticSaves.delete(automatic);
    }
  });
}
