import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, AudioDeviceEntry } from "../api-types";
import { BridgeProvider, useBridge } from "./BridgeProvider";
const mocks = vi.hoisted(() => ({ devices: vi.fn(), status: vi.fn() }));
vi.mock("../api", () => ({
  getConfig: vi.fn(async () => ({audio: {}, ui: { autoStart: false }} as AppConfig)),
  getStatus: mocks.status, saveConfig: vi.fn(async () => true),
  listMidiInputs: vi.fn(async () => []), listMidiOutputs: vi.fn(async () => []),
  listAudioBackends: vi.fn(async () => []), listAudioDevices: mocks.devices,
  preflightCheck: vi.fn(async () => null),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("./LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
beforeEach(() => { vi.clearAllMocks(); mocks.status.mockResolvedValue({running:false}); });

it("audit: a slow old device query must not replace devices for the current backend", async () => {
  let resolveOld!: (devices: AudioDeviceEntry[]) => void;
  mocks.devices.mockImplementation((backend: string) => backend === "ASIO"
    ? new Promise(resolve => { resolveOld = resolve; })
    : Promise.resolve([{name:"WASAPI device", id:"new"}]));
  const {result} = renderHook(() => useBridge(), {wrapper:BridgeProvider});
  await waitFor(() => expect(result.current.isLoading).toBe(false));
  let old!: Promise<void>;
  await act(async () => { old = result.current.refreshAudioDevices("ASIO"); });
  await act(async () => { await result.current.refreshAudioDevices("WASAPI"); });
  await act(async () => { resolveOld([{name:"ASIO device", id:"old"} as AudioDeviceEntry]); await old; });
  expect(result.current.audioDevices[0].id).toBe("new");
});

it("audit: slow old status query must not overwrite status received after Stop", async () => {
  const {result} = renderHook(() => useBridge(), {wrapper:BridgeProvider});
  await waitFor(() => expect(result.current.isLoading).toBe(false));
  let resolveOld!: (status: unknown) => void;
  mocks.status.mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }))
    .mockResolvedValueOnce({running:false});
  let old!: Promise<void>;
  await act(async () => { old = result.current.refreshStatus(); });
  await act(async () => { await result.current.refreshStatus(); });
  await act(async () => { resolveOld({running:true}); await old; });
  expect(result.current.status?.running).toBe(false);
});
