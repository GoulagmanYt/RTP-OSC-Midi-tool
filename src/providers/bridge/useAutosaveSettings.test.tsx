import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppConfig } from "../../api";
import { useAutosaveSettings } from "./useAutosaveSettings";
import { persistConfigReplacement } from "../../config-persistence";

it("publishes saving, saved and failed states without accepting a rejected edit", async () => {
  const state = vi.fn();
  const save = vi.fn().mockResolvedValueOnce(true).mockRejectedValueOnce(new Error("disk unavailable"));
  const { result } = renderHook(() => {
    const [config, setConfig] = useState<AppConfig | null>(initial);
    useAutosaveSettings({ config, setConfig, save, onError: vi.fn(), onStateChange: state });
    return { config, setConfig };
  });
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9002 } } as AppConfig); });
  expect(state).toHaveBeenLastCalledWith("saving");
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(state).toHaveBeenLastCalledWith("saved");
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9003 } } as AppConfig); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(state).toHaveBeenLastCalledWith("error");
  expect(result.current.config?.osc.listenPort).toBe(9002);
});

const initial = { osc: { listenPort: 9001 } } as AppConfig;

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(console, "error").mockImplementation(() => undefined);
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

function mount(save: (config: AppConfig) => Promise<boolean | void>, loadAccepted?: () => Promise<AppConfig>) {
  const onError = vi.fn();
  const hook = renderHook(() => {
    const [config, setConfig] = useState<AppConfig | null>(initial);
    useAutosaveSettings({ config, setConfig, save, onError, loadAccepted });
    return { config, setConfig };
  });
  return { ...hook, onError };
}

it("reports a rejected save and restores the accepted configuration without saving it again", async () => {
  const save = vi.fn().mockRejectedValue(new Error("port occupied"));
  const { result, onError } = mount(save);
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9999 } } as AppConfig); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(result.current.config).toBe(initial);
  expect(onError).toHaveBeenCalledOnce();
  await act(async () => { await vi.advanceTimersByTimeAsync(700); });
  expect(save).toHaveBeenCalledOnce();
});

it("serializes edits and preserves a newer draft when an older save fails", async () => {
  let rejectFirst!: (error: Error) => void;
  const save = vi.fn().mockImplementationOnce(() => new Promise<void>((_, reject) => { rejectFirst = reject; }))
    .mockResolvedValue(undefined);
  const { result } = mount(save);
  const older = { osc: { listenPort: 9999 } } as AppConfig;
  const newer = { osc: { listenPort: 9998 } } as AppConfig;
  await act(async () => { result.current.setConfig(older); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  await act(async () => { result.current.setConfig(newer); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(save).toHaveBeenCalledTimes(1);
  await act(async () => { rejectFirst(new Error("port occupied")); });
  expect(result.current.config).toBe(newer);
  expect(save.mock.calls.map(([config]) => config)).toEqual([older, newer]);
});

it("restores configuration committed by an explicit save rather than an outdated autosave baseline", async () => {
  const committed = { osc: { listenPort: 9002 } } as AppConfig;
  const save = vi.fn().mockRejectedValue(new Error("port occupied"));
  const loadAccepted = vi.fn().mockResolvedValue(committed);
  const { result } = mount(save, loadAccepted);
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9999 } } as AppConfig); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(result.current.config).toBe(committed);
  expect(loadAccepted).toHaveBeenCalledOnce();
});

it("does not restore over an edit made while the accepted configuration is being reloaded", async () => {
  let finishReload!: (config: AppConfig) => void;
  const save = vi.fn().mockRejectedValueOnce(new Error("port occupied")).mockResolvedValue(undefined);
  const loadAccepted = vi.fn(() => new Promise<AppConfig>((resolve) => { finishReload = resolve; }));
  const { result } = mount(save, loadAccepted);
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9999 } } as AppConfig); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  const newer = { osc: { listenPort: 9003 } } as AppConfig;
  await act(async () => { result.current.setConfig(newer); finishReload(initial); });
  expect(result.current.config).toBe(newer);
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(save).toHaveBeenLastCalledWith(newer, expect.any(Function));
});

it("does not treat a skipped native save as an accepted configuration", async () => {
  const save = vi.fn().mockResolvedValueOnce(false).mockRejectedValueOnce(new Error("port occupied"));
  const { result } = mount(save);
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9002 } } as AppConfig); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9999 } } as AppConfig); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(result.current.config).toBe(initial);
});

it("invalidates an old debounce timer when replacement commits before React receives its snapshot", async () => {
  const save = vi.fn().mockResolvedValue(true);
  const { result } = mount(save);
  await act(async () => { result.current.setConfig({ osc: { listenPort: 9002 } } as AppConfig); });
  const replacement = { osc: { listenPort: 9003 } } as AppConfig;
  await persistConfigReplacement(async () => replacement);
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(save).not.toHaveBeenCalled();
  await act(async () => { result.current.setConfig(replacement); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(save).not.toHaveBeenCalled();
  const newer = { osc: { listenPort: 9004 } } as AppConfig;
  await act(async () => { result.current.setConfig(newer); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(save).toHaveBeenCalledWith(newer, expect.any(Function));
});
