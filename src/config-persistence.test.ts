import { expect, it, vi } from "vitest";
import type { AppConfig } from "./api-types";
import { persistConfigSave, persistConfigSerially } from "./config-persistence";

it("keeps automatic and explicit writes in order and continues after a rejection", async () => {
  let rejectFirst!: (error: Error) => void;
  const first = { version: 1 } as AppConfig;
  const second = { version: 2 } as AppConfig;
  const operation = vi.fn().mockImplementationOnce(() => new Promise<void>((_, reject) => { rejectFirst = reject; }))
    .mockResolvedValue(undefined);
  const older = persistConfigSerially(first, operation);
  const newer = persistConfigSerially(second, operation);
  const rejected = expect(older).rejects.toThrow("failed");
  await Promise.resolve();
  expect(operation).toHaveBeenCalledTimes(1);
  rejectFirst(new Error("failed"));
  await rejected;
  await newer;
  expect(operation.mock.calls.map(([config]) => config)).toEqual([first, second]);
});

it("cancels already queued automatic retries of consecutive rejected snapshots but permits later retries", async () => {
  const first = { version: 11 } as AppConfig;
  const second = { version: 12 } as AppConfig;
  const operation = vi.fn().mockRejectedValueOnce(new Error("first failed"))
    .mockRejectedValueOnce(new Error("second failed")).mockResolvedValue(undefined);
  const rejectedFirst = persistConfigSave(first, operation);
  const rejectedSecond = persistConfigSave(second, operation);
  const automaticFirst = persistConfigSave({ ...first }, operation, () => true);
  const automaticSecond = persistConfigSave({ ...second }, operation, () => true);
  await expect(rejectedFirst).rejects.toThrow("first failed");
  await expect(rejectedSecond).rejects.toThrow("second failed");
  expect(await automaticFirst).toBe(false);
  expect(await automaticSecond).toBe(false);
  expect(operation).toHaveBeenCalledTimes(2);
  expect(await persistConfigSave({ ...first }, operation, () => true)).toBe(true);
  expect(await persistConfigSave({ ...second }, operation, () => true)).toBe(true);
  expect(operation).toHaveBeenCalledTimes(4);
});
