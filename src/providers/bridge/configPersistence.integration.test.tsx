import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import * as api from "../../api";
import type { BridgeContextType } from "./BridgeContext";
import { useAudioPageController } from "../../pages/audio/useAudioPageController";
import { useAutosaveSettings } from "./useAutosaveSettings";
import { useBridgeActions } from "./useBridgeActions";

const mocks = vi.hoisted(() => ({ bridge: {} as BridgeContextType, invoke: vi.fn() }));
vi.mock("../../api-client", () => ({ invokeCommand: mocks.invoke }));
vi.mock("../BridgeProvider", () => ({ useBridge: () => mocks.bridge }));
vi.mock("../LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
const initial = {
  midi: { inputDevice: "available" }, audio: { enabled: true, vstEqByPlugin: {} }, osc: { listenPort: 9001 },
} as api.AppConfig;
const onError = vi.fn();

function mount() {
  return renderHook(() => {
    const [config, setConfig] = useState<api.AppConfig | null>(initial);
    const [status, setStatus] = useState<api.RuntimeStatus | null>({ running: true, audioRunning: true } as api.RuntimeStatus);
    const actions = useBridgeActions({ config, status, setConfig, setStatus, setPreflight: vi.fn(),
      refreshStatus: vi.fn(), refreshLists: vi.fn(), refreshAudioDevices: vi.fn(), t: (key) => key });
    useAutosaveSettings({ config, setConfig, save: api.saveConfig, onError });
    mocks.bridge = { config, status, metrics: null, ...actions, audioBackends: [], audioDevices: [],
      refreshAudioDevices: vi.fn(), refreshStatus: vi.fn() } as unknown as BridgeContextType;
    return { config, ...actions, audio: useAudioPageController() };
  });
}

beforeEach(() => { vi.useFakeTimers(); vi.clearAllMocks(); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it("never queues an enabled autosave behind a slow audio disable transition", async () => {
  let finishSave!: () => void;
  const persisted: api.AppConfig[] = [];
  mocks.invoke.mockImplementation(async (command: string, args?: { config: api.AppConfig }) => {
    if (command !== "save_config") return [];
    persisted.push(args!.config);
    if (persisted.length === 1) await new Promise<void>((resolve) => { finishSave = resolve; });
  });
  const { result } = mount();
  await act(async () => { await result.current.updateConfig({ osc: { listenPort: 9002 } }); });
  let toggle!: Promise<void>;
  await act(async () => { toggle = result.current.audio.handleAudioToggle(false); });
  await act(async () => { await vi.advanceTimersByTimeAsync(700); });
  expect(persisted.map((config) => config.audio.enabled)).toEqual([false]);
  await act(async () => { finishSave(); await toggle; });
  expect(persisted.map((config) => config.audio.enabled)).toEqual([false, false]);
  expect(result.current.config?.audio.enabled).toBe(false);
});

it("skips an outdated autosave when it finally reaches the native command queue", async () => {
  let finishSave!: () => void;
  const persisted: api.AppConfig[] = [];
  mocks.invoke.mockImplementation(async (command: string, args?: { config: api.AppConfig }) => {
    if (command !== "save_config") return [];
    persisted.push(args!.config);
    if (persisted.length === 1) await new Promise<void>((resolve) => { finishSave = resolve; });
  });
  const { result } = mount();
  const explicit = api.saveConfig(initial);
  await act(async () => { await result.current.updateConfig({ osc: { listenPort: 9002 } }); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  await act(async () => { await result.current.updateConfig({ osc: { listenPort: 9003 } }); });
  await act(async () => { finishSave(); await explicit; });
  expect(persisted.map((config) => config.osc.listenPort)).toEqual([9001]);
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(persisted.map((config) => config.osc.listenPort)).toEqual([9001, 9003]);
});

it("starts the bridge only after an older configuration save has completed", async () => {
  let finishSave!: () => void;
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "save_config") await new Promise<void>((resolve) => { finishSave = resolve; });
    return { running: true };
  });
  const saved = api.saveConfig(initial);
  const next = { ...initial, audio: { ...initial.audio, enabled: false } };
  const started = api.startBridge(next);
  await Promise.resolve();
  expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["save_config"]);
  finishSave();
  await saved;
  expect(await started).toEqual({ running: true });
  expect(mocks.invoke).toHaveBeenLastCalledWith("start_bridge", { config: next });
});

it("keeps native audio enabled after a slow disable is rejected and the UI rolls back", async () => {
  let rejectSave!: (error: Error) => void;
  let nativeConfig = initial;
  let saves = 0;
  mocks.invoke.mockImplementation(async (command: string, args?: { config: api.AppConfig }) => {
    if (command !== "save_config") return [];
    saves += 1;
    if (saves === 1) await new Promise<void>((_, reject) => { rejectSave = reject; });
    nativeConfig = args!.config;
  });
  const { result } = mount();
  let toggle!: Promise<void>;
  await act(async () => { toggle = result.current.audio.handleAudioToggle(false); });
  await act(async () => { await vi.advanceTimersByTimeAsync(700); });
  await act(async () => { rejectSave(new Error("native audio transition rejected")); await toggle; });
  expect(result.current.config?.audio.enabled).toBe(true);
  expect(nativeConfig.audio.enabled).toBe(true);
});
