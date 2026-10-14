// Regression tests for spontaneous worker transitions and stopped snapshots.
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, RuntimeMetrics, RuntimeStatus } from "../api-types";
import { BridgeProvider, useBridge } from "./BridgeProvider";
import { mergeLiveAudioMetrics } from "../pages/audio/liveStatus";

const mocks = vi.hoisted(() => ({
  callbacks: new Map<string, (event: { payload: unknown }) => void>(),
  getConfig: vi.fn(), getStatus: vi.fn(),
  t: (key: string) => key,
}));
vi.mock("../api", () => ({
  getConfig: mocks.getConfig, getStatus: mocks.getStatus,
  listMidiInputs: vi.fn(async () => []), listMidiOutputs: vi.fn(async () => []),
  listAudioBackends: vi.fn(async () => []), preflightCheck: vi.fn(async () => null),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, callback) => {
  mocks.callbacks.set(name, callback);
  return () => mocks.callbacks.delete(name);
}) }));
vi.mock("./LanguageProvider", () => ({ useI18n: () => ({ t: mocks.t }) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

beforeEach(() => {
  vi.clearAllMocks(); mocks.callbacks.clear();
  mocks.getConfig.mockResolvedValue({ audio: {}, ui: { autoStart: false } } as AppConfig);
  mocks.getStatus.mockResolvedValue({ running: true, audioRunning: true, vstWorkerState: "ready" } as RuntimeStatus);
});

it("a worker fault updates the provider and automatic recovery restores running", async () => {
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "faulted" } }));
  expect(hook.result.current.metrics?.vstWorkerState).toBe("faulted");
  expect(hook.result.current.status?.audioRunning).toBe(false);
  expect(hook.result.current.status?.vstWorkerState).toBe("faulted");
  expect(mocks.getStatus).toHaveBeenCalledTimes(1);
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "starting" } }));
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "ready" } }));
  expect(hook.result.current.status?.audioRunning).toBe(true);
  expect(mergeLiveAudioMetrics(hook.result.current.status, hook.result.current.metrics)?.audioRunning).toBe(true);
  hook.unmount();
});

it("late ready metrics cannot revive an explicitly stopped provider", async () => {
  mocks.getStatus.mockResolvedValue({ running: false, audioRunning: false, vstWorkerState: "disabled" } as RuntimeStatus);
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "ready" } }));
  expect(hook.result.current.status?.vstWorkerState).toBe("disabled");
  expect(hook.result.current.status?.audioRunning).toBe(false);
  hook.unmount();
});

it("evidence: old metrics overwrite the fresh stopped audio snapshot", () => {
  const stopped: RuntimeStatus = {
    running: false, audioRunning: false, vstWorkerState: "disabled",
    oscTarget: "127.0.0.1:9000", rtpActive: false, rtpAdvertisedAddresses: [],
  };
  const beforeStop = { vstWorkerState: "ready", audioPeakL: 0.8 } as RuntimeMetrics;
  expect(mergeLiveAudioMetrics(stopped, beforeStop)).toMatchObject({
    running: false, audioRunning: false, vstWorkerState: "disabled",
  });
});
