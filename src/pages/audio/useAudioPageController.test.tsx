import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig, RuntimeStatus, VstPluginEntry } from "../../api-types";
import { useAudioPageController } from "./useAudioPageController";

const mocks = vi.hoisted(() => ({
  bridge: {} as Record<string, unknown>,
  saveConfig: vi.fn(),
  reloadVst: vi.fn(),
  listVstPlugins: vi.fn(),
  updateConfig: vi.fn(),
  refreshStatus: vi.fn(),
  toastError: vi.fn(),
  setMasterGain: vi.fn(),
  setAudioLimiter: vi.fn(),
}));

vi.mock("../../api", () => ({
  saveConfig: mocks.saveConfig,
  reloadVst: mocks.reloadVst,
  listVstPlugins: mocks.listVstPlugins,
  closeVstUi: vi.fn(), listVstParameters: vi.fn(), openVstUi: vi.fn(),
  openVstFolder: vi.fn(), pingAudio: vi.fn(), refreshVstPlugins: vi.fn(),
  retestVstPlugin: vi.fn(), setAudioLimiter: mocks.setAudioLimiter, setMasterGain: mocks.setMasterGain,
  setVstEq: vi.fn(), setVstParameter: vi.fn(),
}));
vi.mock("../../providers/BridgeProvider", () => ({ useBridge: () => mocks.bridge }));
vi.mock("../../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError, success: vi.fn() } }));

const replacement = { id: "B", path: "B.dll", supported: true, hasEditor: true } as VstPluginEntry;

function currentConfig(): AppConfig {
  return {
    version: 4,
    midi: { thruEnabled: false, hotplug: true, routingProfiles: [], routingAssignments: [] },
    osc: { inputEnabled: false, listenIp: "127.0.0.1", listenPort: 9001, enabled: false, targetIp: "127.0.0.1", targetPort: 9000, logMessages: false },
    rtp: { thruEnabled: false, enabled: false, sessionName: "OSCMidi", port: 5004, remoteEnabled: false, remotes: [], logMessages: false },
    ui: { theme: "dark", themePalette: "default", cornerRadius: 8, autoStart: false, developerMode: false },
    logging: { enabled: true, verbose: false, logAllToFile: false },
    audio: {
      enabled: true, gameMode: true, sampleRate: 48_000, bufferSize: 512,
      gainDb: 0, limiterEnabled: true, vstPluginId: "A", vstPath: "A.dll",
      vstScanPaths: [], vstEqByPlugin: {},
    },
  };
}

describe("audio configuration transactions", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
    mocks.saveConfig.mockResolvedValue(undefined);
    mocks.reloadVst.mockResolvedValue(undefined);
    mocks.listVstPlugins.mockResolvedValue([replacement]);
    mocks.updateConfig.mockResolvedValue(undefined);
    mocks.refreshStatus.mockResolvedValue(undefined);
    mocks.bridge = {
      config: currentConfig(), status: { running: true, audioRunning: true } as RuntimeStatus,
      metrics: null, updateConfig: mocks.updateConfig, refreshStatus: mocks.refreshStatus,
      saveConfig: vi.fn(), refreshAudioDevices: vi.fn(), audioBackends: [], audioDevices: [],
    };
  });

  it("does not load devices from a backend rejected by the native transaction", async () => {
    mocks.bridge.config = { ...currentConfig(), audio: { ...currentConfig().audio, backend: "WASAPI" } };
    mocks.saveConfig.mockRejectedValueOnce(new Error("ASIO failed"));
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => { await result.current.handleBackendChange("ASIO"); });
    expect(mocks.updateConfig).toHaveBeenLastCalledWith({ audio: (mocks.bridge.config as AppConfig).audio });
    expect(mocks.bridge.refreshAudioDevices).not.toHaveBeenCalledWith("ASIO");
    expect(mocks.toastError).toHaveBeenCalledWith("ASIO failed");
  });

  it("reports rejected live controls without persisting their unaccepted values", async () => {
    mocks.setMasterGain.mockRejectedValueOnce(new Error("gain queue full"));
    mocks.setAudioLimiter.mockRejectedValueOnce(new Error("limiter queue full"));
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => {
      await result.current.handleGainChange(-12);
      await result.current.handleLimiterToggle(false);
    });
    expect(mocks.updateConfig).not.toHaveBeenCalled();
    expect(mocks.toastError.mock.calls).toEqual([["gain queue full"], ["limiter queue full"]]);
  });

  it("saves the new audio enabled value rather than the previous render", async () => {
    let finishSave!: () => void;
    mocks.saveConfig.mockImplementationOnce(() => new Promise<void>((resolve) => { finishSave = resolve; }));
    const { result } = renderHook(() => useAudioPageController());
    let pending!: Promise<void>;
    await act(async () => { pending = result.current.handleAudioToggle(false); });
    expect(mocks.saveConfig.mock.calls[0][0].audio.enabled).toBe(false);
    expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: { enabled: false } });
    await act(async () => { finishSave(); await pending; });
    expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: { enabled: false } });
    expect(mocks.refreshStatus).toHaveBeenCalledOnce();
    expect(mocks.bridge.saveConfig).not.toHaveBeenCalled();
  });

  it("keeps the audio switch unchanged and reports a rejected native transition", async () => {
    mocks.saveConfig.mockRejectedValueOnce(new Error("audio start failed"));
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => { await result.current.handleAudioToggle(false); });
    expect(mocks.updateConfig.mock.calls).toEqual([
      [{ audio: { enabled: false } }], [{ audio: { enabled: true } }],
    ]);
    expect(mocks.toastError).toHaveBeenCalledWith("audio start failed");
    expect(result.current.audioReloading).toBe(false);
  });

  it("restores a failed game-mode change without reloading the VST", async () => {
    mocks.saveConfig.mockRejectedValueOnce(new Error("disk failed"));
    const { result } = renderHook(() => useAudioPageController());
    let accepted: boolean | undefined;
    await act(async () => { accepted = await result.current.updateAudioConfig({ gameMode: false }); });
    expect(accepted).toBe(false);
    expect(mocks.reloadVst).not.toHaveBeenCalled();
    expect(mocks.saveConfig.mock.calls.map(([config]) => config.audio.gameMode)).toEqual([false, true]);
    expect(mocks.updateConfig).toHaveBeenLastCalledWith({ audio: currentConfig().audio });
  });

  it("keeps an accepted change when refreshing status fails", async () => {
    mocks.refreshStatus.mockRejectedValueOnce(new Error("status unavailable"));
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result } = renderHook(() => useAudioPageController());
    let accepted: boolean | undefined;
    await act(async () => { accepted = await result.current.updateAudioConfig({ gameMode: false }); });
    expect(accepted).toBe(true);
    expect(mocks.saveConfig).toHaveBeenCalledTimes(1);
    expect(mocks.updateConfig).toHaveBeenCalledTimes(1);
    expect(mocks.reloadVst).not.toHaveBeenCalled();
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it("does not save a rejected VST selection again after rolling back", async () => {
    mocks.reloadVst.mockRejectedValueOnce(new Error("B load failed"));
    const { result } = renderHook(() => useAudioPageController());
    await waitFor(() => expect(result.current.vstPlugins).toEqual([replacement]));
    await act(async () => { await result.current.handleSelectVst("B"); });
    expect(mocks.saveConfig.mock.calls.map(([config]) => config.audio.vstPluginId)).toEqual(["B", "A"]);
    expect(mocks.reloadVst).toHaveBeenCalledTimes(2);
    expect(mocks.updateConfig).toHaveBeenLastCalledWith({ audio: currentConfig().audio });
  });

  it("persists a VST selection once when audio is stopped", async () => {
    mocks.bridge.status = { running: true, audioRunning: false } as RuntimeStatus;
    const { result } = renderHook(() => useAudioPageController());
    await waitFor(() => expect(result.current.vstPlugins).toEqual([replacement]));
    await act(async () => { await result.current.handleSelectVst("B"); });
    expect(mocks.saveConfig).toHaveBeenCalledTimes(1);
    expect(mocks.saveConfig.mock.calls[0][0].audio.vstPluginId).toBe("B");
    expect(mocks.reloadVst).not.toHaveBeenCalled();
  });

  it("rejects overlapping changes while configuration is being applied", async () => {
    let finishSave: (() => void) | undefined;
    mocks.saveConfig.mockImplementationOnce(() => new Promise<void>((resolve) => { finishSave = resolve; }));
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => {
      const first = result.current.updateAudioConfig({ gameMode: false });
      await Promise.resolve();
      const second = await result.current.updateAudioConfig({ vstPluginId: "B" });
      expect(second).toBe(false);
      finishSave?.();
      expect(await first).toBe(true);
    });
    expect(mocks.saveConfig).toHaveBeenCalledTimes(1);
    expect(mocks.updateConfig).toHaveBeenCalledTimes(1);
  });

  it("tracks actual editor state after a deferred open and native close", async () => {
    const { result, rerender } = renderHook(() => useAudioPageController());
    expect(result.current.vstUiOpen).toBe(false);
    mocks.bridge.status = { realtime: { worker: { editorOpen: true } } } as RuntimeStatus;
    rerender();
    await waitFor(() => expect(result.current.vstUiOpen).toBe(true));
    mocks.bridge.status = { realtime: { worker: { editorOpen: false } } } as RuntimeStatus;
    rerender();
    await waitFor(() => expect(result.current.vstUiOpen).toBe(false));
  });

  it("keeps the actual editor open when a VST selection fails before reloading", async () => {
    mocks.bridge.status = {
      running: true, audioRunning: true, realtime: { worker: { editorOpen: true } },
    } as RuntimeStatus;
    mocks.saveConfig.mockRejectedValueOnce(new Error("disk failed"));
    const { result } = renderHook(() => useAudioPageController());
    await waitFor(() => expect(result.current.vstPlugins).toEqual([replacement]));
    expect(result.current.vstUiOpen).toBe(true);
    await act(async () => { await result.current.handleSelectVst("B"); });
    expect(mocks.reloadVst).not.toHaveBeenCalled();
    expect(result.current.vstUiOpen).toBe(true);
  });
});
