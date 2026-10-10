import type { AppConfig } from "./api-types";

let pendingSave: Promise<void> = Promise.resolve();
let saveGeneration = 0;
const automaticSaves = new Set<{ signature: string; cancelled: boolean }>();
const acceptedReplacements = new WeakSet<AppConfig>();

function enqueueConfigOperation<T>(operation: () => Promise<T>): Promise<T> {
  const result = pendingSave.then(operation);
  pendingSave = result.then(() => undefined, () => undefined);
  return result;
}

export function getConfigSaveGeneration(): number {
  return saveGeneration;
}

export function isAcceptedConfigReplacement(config: AppConfig): boolean {
  return acceptedReplacements.has(config);
}

/** A replacement commits after earlier writes and invalidates their pending autosaves. */
export function persistConfigReplacement(operation: () => Promise<AppConfig>): Promise<AppConfig> {
  return enqueueConfigOperation(async () => {
    const accepted = await operation();
    saveGeneration += 1;
    for (const queued of automaticSaves) queued.cancelled = true;
    acceptedReplacements.add(accepted);
    return accepted;
  });
}

/** Preserve invocation order across automatic and explicit configuration saves. */
export function persistConfigSerially<T>(
  config: AppConfig,
  operation: (config: AppConfig) => Promise<T>
): Promise<T> {
  return enqueueConfigOperation(() => operation(config));
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
