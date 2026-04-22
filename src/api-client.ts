import { invoke } from "@tauri-apps/api/core";

import { normalizeCommandError } from "./api-errors";

export async function invokeCommand<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw normalizeCommandError(error);
  }
}
