import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, RuntimeStatus } from "../api-types";
import { BridgeProvider, useBridge } from "./BridgeProvider";

const mocks = vi.hoisted(() => ({ getConfig: vi.fn(), getStatus: vi.fn(), saveConfig: vi.fn() }));
vi.mock("../api", () => ({
  getConfig: mocks.getConfig, getStatus: mocks.getStatus, saveConfig: mocks.saveConfig,
  listMidiInputs: vi.fn(async () => []), listMidiOutputs: vi.fn(async () => []),
  listAudioBackends: vi.fn(async () => []), preflightCheck: vi.fn(async () => null),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("./LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

beforeEach(() => { vi.useFakeTimers(); vi.clearAllMocks(); });
afterEach(() => { vi.useRealTimers(); });

it("refreshes actual RTP status after an automatic or explicit accepted save", async () => {
  const initial = { audio: {}, ui: { autoStart: false }, rtp: { enabled: true, port: 5004 } } as AppConfig;
  let nativeStatus = { running: false, rtpActive: true, rtpBoundPort: 5004 } as RuntimeStatus;
  mocks.getConfig.mockResolvedValue(initial);
  mocks.getStatus.mockImplementation(async () => nativeStatus);
  mocks.saveConfig.mockImplementation(async (config: AppConfig) => {
    nativeStatus = { ...nativeStatus, rtpActive: config.rtp.enabled, rtpBoundPort: config.rtp.enabled ? config.rtp.port : null };
    return true;
  });
  const { result } = renderHook(() => useBridge(), { wrapper: BridgeProvider });
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(result.current.status?.rtpActive).toBe(true);
  await act(async () => { await result.current.updateConfig({ rtp: { enabled: false } }); });
  await act(async () => { await vi.advanceTimersByTimeAsync(350); });
  expect(result.current.status?.rtpActive).toBe(false);
  expect(result.current.status?.rtpBoundPort).toBeNull();
  await act(async () => { await result.current.updateConfig({ rtp: { enabled: true, port: 5010 } }); });
  await act(async () => { await result.current.saveConfig(); });
  expect(result.current.status?.rtpActive).toBe(true);
  expect(result.current.status?.rtpBoundPort).toBe(5010);
});
