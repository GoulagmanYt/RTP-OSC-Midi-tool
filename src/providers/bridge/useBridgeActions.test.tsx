import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, PreflightReport, RuntimeStatus } from "../../api-types";
import { useBridgeActions } from "./useBridgeActions";

const mocks = vi.hoisted(() => ({ preflight: vi.fn(), start: vi.fn(), toastError: vi.fn() }));
vi.mock("../../api", () => ({
  preflightCheck: mocks.preflight, startBridge: mocks.start, stopBridge: vi.fn(), saveConfig: vi.fn(), getConfig: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));
const initial = { midi: { inputDevice: null }, audio: {} } as AppConfig;

function mount() {
  return renderHook(() => {
    const [config, setConfig] = useState<AppConfig | null>(initial);
    const [status, setStatus] = useState<RuntimeStatus | null>({ running: false } as RuntimeStatus);
    const [preflight, setPreflight] = useState<PreflightReport | null>(null);
    const actions = useBridgeActions({ config, status, setConfig, setStatus, setPreflight,
      refreshStatus: vi.fn(), refreshLists: vi.fn(), refreshAudioDevices: vi.fn(), t: (key) => key });
    return { config, preflight, ...actions };
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
});
