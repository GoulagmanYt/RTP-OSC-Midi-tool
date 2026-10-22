// Regression tests for spontaneous worker transitions and stopped snapshots.
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, RuntimeMetrics, RuntimeStatus } from "../api-types";
import { BridgeProvider, useBridge } from "./BridgeProvider";
import { mergeLiveAudioMetrics } from "../pages/audio/liveStatus";

const mocks = vi.hoisted(() => ({
  callbacks: new Map<string, (event: { payload: unknown }) => void>(),
  getConfig: vi.fn(), getStatus: vi.fn(),
  preflight: vi.fn(), start: vi.fn(),
  t: (key: string) => key,
}));
vi.mock("../api", () => ({
  getConfig: mocks.getConfig, getStatus: mocks.getStatus,
  listMidiInputs: vi.fn(async () => []), listMidiOutputs: vi.fn(async () => []),
  listAudioBackends: vi.fn(async () => []), preflightCheck: mocks.preflight, startBridge: mocks.start,
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
  mocks.getStatus.mockReset().mockResolvedValue({ running: true, audioRunning: true, vstWorkerState: "ready" } as RuntimeStatus);
  mocks.start.mockReset();
  mocks.preflight.mockResolvedValue(null);
});

it.each([false, true])("preserves a successful start if the following status query fails (autoStart=%s)", async (autoStart) => {
  mocks.getConfig.mockResolvedValue({ audio: {}, ui: { autoStart } } as AppConfig);
  mocks.getStatus.mockResolvedValueOnce({ running: false, audioRunning: false, vstWorkerState: "disabled" } as RuntimeStatus)
    .mockRejectedValueOnce(new Error("status unavailable"));
  mocks.start.mockResolvedValue({ running: true, audioRunning: true, vstWorkerState: "ready" } as RuntimeStatus);
  mocks.preflight.mockResolvedValue({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true });
  const errorLog = vi.spyOn(console, "error").mockImplementation(() => undefined);
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  if (!autoStart) await act(async () => { await hook.result.current.toggleBridge(); });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "ready" } }));
  expect(hook.result.current.status).toMatchObject({ running: true, audioRunning: true, vstWorkerState: "ready" });
  errorLog.mockRestore();
});

it.each([false, true])("disabled startup metrics cannot hide a successful start (previous session=%s)", async (previousSession) => {
  mocks.getStatus.mockResolvedValueOnce({ running: previousSession, audioRunning: previousSession,
    vstWorkerState: previousSession ? "ready" : "disabled", vstWorkerRestarts: 0 } as RuntimeStatus);
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  if (previousSession) {
    act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "ready", vstWorkerRestarts: 0 } }));
    mocks.getStatus.mockResolvedValueOnce({ running: false, audioRunning: false, vstWorkerState: "disabled", vstWorkerRestarts: 0 } as RuntimeStatus);
    await act(async () => { await hook.result.current.refreshStatus(); });
  }
  mocks.preflight.mockResolvedValue({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true });
  let finish!: (status: RuntimeStatus) => void;
  mocks.start.mockImplementationOnce(() => new Promise<RuntimeStatus>((resolve) => { finish = resolve; }));
  let pending!: Promise<void>;
  await act(async () => { pending = hook.result.current.toggleBridge(); });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "disabled", vstWorkerRestarts: 0 } }));
  mocks.getStatus.mockRejectedValueOnce(new Error("status unavailable"));
  const errorLog = vi.spyOn(console, "error").mockImplementation(() => undefined);
  await act(async () => {
    finish({ running: true, audioRunning: true, vstWorkerState: "ready" } as RuntimeStatus);
    await pending;
  });
  expect(hook.result.current.status).toMatchObject({ running: true, audioRunning: true, vstWorkerState: "ready" });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "ready" } }));
  expect(hook.result.current.status).toMatchObject({ running: true, audioRunning: true, vstWorkerState: "ready" });
  errorLog.mockRestore();
});

it.each(["starting", "disabled"])("a delayed start result cannot overwrite a worker fault from %s even if refresh fails", async (initialWorkerState) => {
  mocks.getStatus.mockResolvedValueOnce({ running: false, audioRunning: false, vstWorkerState: initialWorkerState } as RuntimeStatus);
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  mocks.preflight.mockResolvedValue({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true });
  let finish!: (status: RuntimeStatus) => void;
  mocks.start.mockImplementationOnce(() => new Promise<RuntimeStatus>((resolve) => { finish = resolve; }));
  let pending!: Promise<void>;
  await act(async () => { pending = hook.result.current.toggleBridge(); });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "faulted" } }));
  mocks.getStatus.mockRejectedValueOnce(new Error("status unavailable"));
  const errorLog = vi.spyOn(console, "error").mockImplementation(() => undefined);
  await act(async () => {
    finish({ running: true, audioRunning: true, vstWorkerState: "ready" } as RuntimeStatus);
    await pending;
  });
  expect(hook.result.current.status).toMatchObject({ audioRunning: false, vstWorkerState: "faulted" });
  errorLog.mockRestore();
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

it("a pending status snapshot cannot undo a newer worker fault event", async () => {
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  let finish!: (status: RuntimeStatus) => void;
  mocks.getStatus.mockImplementationOnce(() => new Promise<RuntimeStatus>((resolve) => { finish = resolve; }));
  let pending!: Promise<void>;
  act(() => { pending = hook.result.current.refreshStatus(); });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "faulted" } }));
  await act(async () => {
    finish({ running: true, audioRunning: true, vstWorkerState: "ready", rtpBoundPort: 5010 } as RuntimeStatus);
    await pending;
  });
  expect(hook.result.current.status).toMatchObject({ audioRunning: false, vstWorkerState: "faulted" });
  expect(hook.result.current.status?.rtpBoundPort).toBe(5010);
  hook.unmount();
  expect(mocks.callbacks.size).toBe(0);
});

it("accepts the stopped snapshot even if a queued worker event arrives during refresh", async () => {
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  let finish!: (status: RuntimeStatus) => void;
  mocks.getStatus.mockImplementationOnce(() => new Promise<RuntimeStatus>((resolve) => { finish = resolve; }));
  let pending!: Promise<void>;
  act(() => { pending = hook.result.current.refreshStatus(); });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "faulted" } }));
  await act(async () => {
    finish({ running: false, audioRunning: false, vstWorkerState: "disabled" } as RuntimeStatus);
    await pending;
  });
  expect(hook.result.current.status).toMatchObject({ running: false, audioRunning: false, vstWorkerState: "disabled" });
});

it("unchanged worker metrics do not discard an in-flight status refresh", async () => {
  const hook = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await waitFor(() => expect(hook.result.current.isLoading).toBe(false));
  let finish!: (status: RuntimeStatus) => void;
  mocks.getStatus.mockImplementationOnce(() => new Promise<RuntimeStatus>((resolve) => { finish = resolve; }));
  let pending!: Promise<void>;
  act(() => { pending = hook.result.current.refreshStatus(); });
  act(() => mocks.callbacks.get("runtime:metrics")!({ payload: { vstWorkerState: "ready" } }));
  await act(async () => {
    finish({ running: true, audioRunning: true, vstWorkerState: "ready", rtpBoundPort: 5010 } as RuntimeStatus);
    await pending;
  });
  expect(hook.result.current.status?.rtpBoundPort).toBe(5010);
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
