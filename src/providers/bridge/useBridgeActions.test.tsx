import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, PreflightReport, RuntimeStatus } from "../../api-types";
import { useBridgeActions } from "./useBridgeActions";

const mocks = vi.hoisted(() => ({ preflight: vi.fn(), start: vi.fn(), stop: vi.fn(), getConfig: vi.fn(), toastError: vi.fn() }));
vi.mock("../../api", () => ({
  preflightCheck: mocks.preflight, startBridge: mocks.start, stopBridge: mocks.stop, saveConfig: vi.fn(), getConfig: mocks.getConfig,
}));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));
const initial = { midi: { inputDevice: null }, audio: {} } as AppConfig;

function mount(running = false, refreshStatus = vi.fn()) {
  return renderHook(() => {
    const [config, setConfig] = useState<AppConfig | null>(initial);
    const [status, setStatus] = useState<RuntimeStatus | null>({ running } as RuntimeStatus);
    const [preflight, setPreflight] = useState<PreflightReport | null>(null);
    const actions = useBridgeActions({ config, status, setConfig, setStatus, setPreflight,
      refreshStatus, refreshLists: vi.fn(), refreshAudioDevices: vi.fn(), t: (key) => key });
    return { config, status, preflight, ...actions };
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.start.mockResolvedValue({ running: true });
  mocks.preflight.mockImplementation(async (config: AppConfig) => ({
    midiInOk: Boolean(config.midi.inputDevice), midiOutOk: true, audioBackendOk: true, rtpPortOk: true, messages: [],
  }));
});

it("validates and starts the edited configuration immediately before an autosave", async () => {
  const { result } = mount();
  await act(async () => { await result.current.updateConfig({ midi: { inputDevice: "available input" } }); });
  const edited = result.current.config;
  await act(async () => { await result.current.toggleBridge(); });
  expect(mocks.preflight).toHaveBeenCalledWith(edited);
  expect(mocks.start).toHaveBeenCalledWith(edited);
  expect(mocks.toastError).not.toHaveBeenCalled();
});

it("blocks an invalid edited configuration even when the previous one was valid", async () => {
  const { result } = mount();
  await act(async () => { await result.current.updateConfig({ midi: { inputDevice: "available input" } }); });
  await act(async () => { await result.current.updateConfig({ midi: { inputDevice: null } }); });
  await act(async () => { await result.current.toggleBridge(); });
  expect(mocks.preflight).toHaveBeenCalledWith(result.current.config);
  expect(mocks.start).not.toHaveBeenCalled();
  expect(mocks.toastError).toHaveBeenCalledWith("toasts.bridge.preflightBlocked");
  expect(result.current.bridgeTransition).toBeNull();
});

it("does not start the bridge when preflight itself fails", async () => {
  const errorLog = vi.spyOn(console, "error").mockImplementation(() => undefined);
  mocks.preflight.mockRejectedValueOnce(new Error("MIDI enumeration failed"));
  const { result } = mount();
  await act(async () => { await result.current.toggleBridge(); });
  errorLog.mockRestore();
  expect(mocks.start).not.toHaveBeenCalled();
  expect(mocks.toastError).toHaveBeenCalledWith("toasts.bridge.preflightFailed");
  expect(result.current.bridgeTransition).toBeNull();
});

it("does not start an obsolete configuration edited during preflight", async () => {
  let finish!: (report: PreflightReport) => void;
  mocks.preflight.mockImplementationOnce(() => new Promise<PreflightReport>((resolve) => { finish = resolve; }));
  const { result } = mount();
  let pending!: Promise<void>;
  act(() => { pending = result.current.toggleBridge(); });
  await act(async () => { await result.current.updateConfig({ midi: { inputDevice: "new device" } }); });
  await act(async () => {
    finish({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true, messages: [] });
    await pending;
  });
  expect(result.current.config?.midi.inputDevice).toBe("new device");
  expect(mocks.start).not.toHaveBeenCalled();
  expect(result.current.preflight).toBeNull();
  expect(result.current.bridgeTransition).toBeNull();
});

it("a late configuration reload cannot overwrite an accepted replacement", async () => {
  let finish!: (config: AppConfig) => void;
  mocks.getConfig.mockImplementationOnce(() => new Promise<AppConfig>((resolve) => { finish = resolve; }));
  const { result } = mount();
  let old!: Promise<void>;
  act(() => { old = result.current.reloadConfig(); });
  const accepted = { ...initial, midi: { ...initial.midi, inputDevice: "imported device" } };
  await act(async () => { await result.current.reloadConfig(accepted); });
  await act(async () => { finish(initial); await old; });
  expect(result.current.config).toBe(accepted);
});

it("does not start a worker after unmounting during preflight", async () => {
  let finish!: (report: PreflightReport) => void;
  mocks.preflight.mockImplementationOnce(() => new Promise<PreflightReport>((resolve) => { finish = resolve; }));
  const { result, unmount } = mount();
  let pending!: Promise<void>;
  act(() => { pending = result.current.toggleBridge(); });
  unmount();
  await act(async () => {
    finish({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true, messages: [] });
    await pending;
  });
  expect(mocks.start).not.toHaveBeenCalled();
});

it("sends one start on double click while preflight is pending", async () => {
  let finish!: (report: PreflightReport) => void;
  mocks.preflight.mockImplementationOnce(() => new Promise<PreflightReport>((resolve) => { finish = resolve; }));
  const { result } = mount();
  let pending!: Promise<void>;
  act(() => { pending = result.current.toggleBridge(); });
  await act(async () => { await result.current.toggleBridge(); });
  await act(async () => {
    finish({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true, messages: [] });
    await pending;
  });
  expect(mocks.preflight).toHaveBeenCalledOnce();
  expect(mocks.start).toHaveBeenCalledOnce();
});

it("sends one stop on double click and releases the guard after rejection", async () => {
  let fail!: (error: Error) => void;
  mocks.stop.mockImplementationOnce(() => new Promise<void>((_, reject) => { fail = reject; }));
  const errorLog = vi.spyOn(console, "error").mockImplementation(() => undefined);
  const { result } = mount(true);
  let pending!: Promise<void>;
  act(() => { pending = result.current.toggleBridge(); });
  expect(result.current.bridgeTransition).toBe("stopping");
  await act(async () => { await result.current.toggleBridge(); });
  expect(mocks.stop).toHaveBeenCalledOnce();
  await act(async () => { fail(new Error("stop failed")); await pending; });
  expect(result.current.bridgeTransition).toBeNull();
  await act(async () => { await result.current.toggleBridge(); });
  expect(mocks.stop).toHaveBeenCalledTimes(2);
  expect(mocks.toastError).toHaveBeenCalledWith("stop failed");
  errorLog.mockRestore();
});

it("reports starting from preflight through the final status refresh", async () => {
  let finishPreflight!: (report: PreflightReport) => void;
  let finishStart!: (status: RuntimeStatus) => void;
  let finishRefresh!: () => void;
  mocks.preflight.mockImplementationOnce(() => new Promise<PreflightReport>((resolve) => { finishPreflight = resolve; }));
  mocks.start.mockImplementationOnce(() => new Promise<RuntimeStatus>((resolve) => { finishStart = resolve; }));
  const refreshStatus = vi.fn(() => new Promise<void>((resolve) => { finishRefresh = resolve; }));
  const { result } = mount(false, refreshStatus);
  expect(result.current.bridgeTransition).toBeNull();
  let pending!: Promise<void>;
  act(() => { pending = result.current.toggleBridge(); });
  expect(result.current.bridgeTransition).toBe("starting");
  await act(async () => {
    finishPreflight({ midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true, messages: [] });
  });
  expect(result.current.bridgeTransition).toBe("starting");
  await act(async () => { finishStart({ running: true } as RuntimeStatus); });
  expect(result.current.status?.running).toBe(true);
  expect(result.current.bridgeTransition).toBe("starting");
  await act(async () => { finishRefresh(); await pending; });
  expect(result.current.bridgeTransition).toBeNull();
});

it("reports stopping until the final status refresh completes", async () => {
  let finishStop!: () => void;
  let finishRefresh!: () => void;
  mocks.stop.mockImplementationOnce(() => new Promise<void>((resolve) => { finishStop = resolve; }));
  const refreshStatus = vi.fn(() => new Promise<void>((resolve) => { finishRefresh = resolve; }));
  const { result } = mount(true, refreshStatus);
  let pending!: Promise<void>;
  act(() => { pending = result.current.toggleBridge(); });
  expect(result.current.bridgeTransition).toBe("stopping");
  await act(async () => { finishStop(); });
  expect(result.current.bridgeTransition).toBe("stopping");
  await act(async () => { finishRefresh(); await pending; });
  expect(result.current.bridgeTransition).toBeNull();
});
