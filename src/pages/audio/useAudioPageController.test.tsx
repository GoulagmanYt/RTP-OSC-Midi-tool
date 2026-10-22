import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode } from "react";
import type { AppConfig, RuntimeStatus, VstParameter, VstPluginEntry } from "../../api-types";
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
  refreshVstPlugins: vi.fn(),
  listVstParameters: vi.fn(),
  retestVstPlugin: vi.fn(),
  setVstParameter: vi.fn(),
}));

vi.mock("../../api", () => ({
  saveConfig: mocks.saveConfig,
  reloadVst: mocks.reloadVst,
  listVstPlugins: mocks.listVstPlugins,
  closeVstUi: vi.fn(), listVstParameters: mocks.listVstParameters, openVstUi: vi.fn(),
  openVstFolder: vi.fn(), pingAudio: vi.fn(), refreshVstPlugins: mocks.refreshVstPlugins,
  retestVstPlugin: mocks.retestVstPlugin, setAudioLimiter: mocks.setAudioLimiter, setMasterGain: mocks.setMasterGain,
  setVstEq: vi.fn(), setVstParameter: mocks.setVstParameter,
}));
vi.mock("../../providers/BridgeProvider", () => ({ useBridge: () => mocks.bridge }));
vi.mock("../../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError, success: vi.fn() } }));

const replacement = { id: "B", path: "B.dll", supported: true, hasEditor: true } as VstPluginEntry;
const parameter: VstParameter = { index: 0, id: 42, name: "Cutoff", value: 0.2,
  min: 0, max: 1, default: 0, flags: 0, stepCount: 0, unitId: 0, unit: "" };

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
    mocks.listVstParameters.mockReset().mockResolvedValue([]);
    mocks.retestVstPlugin.mockReset();
    mocks.saveConfig.mockResolvedValue(undefined);
    mocks.reloadVst.mockResolvedValue(undefined);
    mocks.listVstPlugins.mockResolvedValue([replacement]);
    mocks.refreshVstPlugins.mockResolvedValue([replacement]);
    mocks.setVstParameter.mockResolvedValue(undefined);
    mocks.updateConfig.mockResolvedValue(undefined);
    mocks.refreshStatus.mockResolvedValue(undefined);
    mocks.bridge = {
      config: currentConfig(), status: { running: true, audioRunning: true } as RuntimeStatus,
      metrics: null, updateConfig: mocks.updateConfig, refreshStatus: mocks.refreshStatus,
      saveConfig: vi.fn(), refreshAudioDevices: vi.fn(), audioBackends: [], audioDevices: [],
    };
  });

  it("keeps a fresh scan when the initial cached catalogue resolves last", async () => {
    let finishCache!: (plugins: VstPluginEntry[]) => void;
    mocks.listVstPlugins.mockImplementationOnce(() => new Promise<VstPluginEntry[]>((resolve) => { finishCache = resolve; }));
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => { await result.current.refreshVstPluginsList(); });
    await act(async () => { finishCache([]); });
    expect(result.current.vstPlugins).toEqual([replacement]);
    expect(result.current.vstPluginsLoading).toBe(false);
  });

  it("ignores the cached response from the first StrictMode effect setup", async () => {
    let finishFirst!: (plugins: VstPluginEntry[]) => void;
    let finishSecond!: (plugins: VstPluginEntry[]) => void;
    mocks.listVstPlugins
      .mockImplementationOnce(() => new Promise<VstPluginEntry[]>((resolve) => { finishFirst = resolve; }))
      .mockImplementationOnce(() => new Promise<VstPluginEntry[]>((resolve) => { finishSecond = resolve; }));
    const { result } = renderHook(() => useAudioPageController(), { wrapper: StrictMode });
    await act(async () => { finishSecond([replacement]); });
    await act(async () => { finishFirst([]); });
    expect(result.current.vstPlugins).toEqual([replacement]);
  });

  it("keeps the newest scan busy when an older concurrent scan fails", async () => {
    let failOld!: (error: Error) => void;
    let finishNew!: (plugins: VstPluginEntry[]) => void;
    mocks.refreshVstPlugins
      .mockImplementationOnce(() => new Promise<VstPluginEntry[]>((_, reject) => { failOld = reject; }))
      .mockImplementationOnce(() => new Promise<VstPluginEntry[]>((resolve) => { finishNew = resolve; }));
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result } = renderHook(() => useAudioPageController());
    let old!: Promise<void>;
    let recent!: Promise<void>;
    act(() => { old = result.current.refreshVstPluginsList(); recent = result.current.refreshVstPluginsList(); });
    await act(async () => { failOld(new Error("old scan failed")); await old; });
    expect(result.current.vstPluginsLoading).toBe(true);
    expect(mocks.toastError).not.toHaveBeenCalled();
    await act(async () => { finishNew([replacement]); await recent; });
    expect(result.current.vstPlugins).toEqual([replacement]);
    expect(result.current.vstPluginsLoading).toBe(false);
    errorLog.mockRestore();
  });

  it("discards an older scan that resolves after the newest scan", async () => {
    let finishOld!: (plugins: VstPluginEntry[]) => void;
    mocks.refreshVstPlugins.mockImplementationOnce(() => new Promise<VstPluginEntry[]>((resolve) => { finishOld = resolve; }));
    const { result } = renderHook(() => useAudioPageController());
    let old!: Promise<void>;
    act(() => { old = result.current.refreshVstPluginsList(); });
    await act(async () => { await result.current.refreshVstPluginsList(); });
    await act(async () => { finishOld([]); await old; });
    expect(result.current.vstPlugins).toEqual([replacement]);
    expect(mocks.refreshVstPlugins).toHaveBeenCalledTimes(2);
  });

  it("releases the loading flag after the current scan fails", async () => {
    mocks.refreshVstPlugins.mockRejectedValueOnce(new Error("scan failed"));
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => { await result.current.refreshVstPluginsList(); });
    expect(result.current.vstPluginsLoading).toBe(false);
    expect(mocks.toastError).toHaveBeenCalledWith("toasts.audio.vstListFailed");
  });

  it("ignores a scan error delivered after the page unmounts", async () => {
    let fail!: (error: Error) => void;
    mocks.refreshVstPlugins.mockImplementationOnce(() => new Promise<VstPluginEntry[]>((_, reject) => { fail = reject; }));
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result, unmount } = renderHook(() => useAudioPageController());
    let pending!: Promise<void>;
    act(() => { pending = result.current.refreshVstPluginsList(); });
    unmount();
    await act(async () => { fail(new Error("page gone")); await pending; });
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it("keeps a newer retest when an older scan resolves last", async () => {
    let finishScan!: (plugins: VstPluginEntry[]) => void;
    mocks.refreshVstPlugins.mockImplementationOnce(() => new Promise<VstPluginEntry[]>((resolve) => { finishScan = resolve; }));
    const tested = { ...replacement, supported: false, unsupportedReason: "probe failed" };
    mocks.retestVstPlugin.mockResolvedValueOnce(tested);
    const { result } = renderHook(() => useAudioPageController());
    await waitFor(() => expect(result.current.vstPlugins).toEqual([replacement]));
    let pending!: Promise<void>;
    act(() => { pending = result.current.refreshVstPluginsList(); });
    await act(async () => { await result.current.handleRetestVst("B"); });
    await act(async () => { finishScan([replacement]); await pending; });
    expect(result.current.vstPlugins).toEqual([tested]);
    expect(result.current.vstPluginsLoading).toBe(false);
    expect(mocks.retestVstPlugin).toHaveBeenCalledWith("B");
  });

  it("invalidates a pending parameter catalogue across worker loss and recovery", async () => {
    let finish!: (parameters: VstParameter[]) => void;
    mocks.listVstParameters.mockImplementationOnce(() => new Promise<VstParameter[]>((resolve) => { finish = resolve; }));
    const { result, rerender } = renderHook(() => useAudioPageController());
    let pending!: Promise<void>;
    act(() => { pending = result.current.handleOpenVstParameters(); });
    mocks.bridge.status = { running: true, audioRunning: false, vstWorkerState: "faulted" } as RuntimeStatus;
    rerender();
    const loadingAfterLoss = result.current.vstParamsLoading;
    mocks.bridge.status = { running: true, audioRunning: true, vstWorkerState: "ready", vstWorkerRestarts: 1 } as RuntimeStatus;
    rerender();
    await act(async () => { finish([parameter]); await pending; });
    expect(loadingAfterLoss).toBe(false);
    expect(result.current.vstParams).toEqual([]);
    expect(result.current.vstParameterDialogOpen).toBe(false);
    mocks.listVstParameters.mockResolvedValueOnce([parameter]);
    await act(async () => { await result.current.handleOpenVstParameters(); });
    expect(result.current.vstParameterDialogOpen).toBe(true);
    expect(result.current.vstParams).toEqual([parameter]);
  });

  it("does not reopen a parameter catalogue from before an explicit reload", async () => {
    let finish!: (parameters: VstParameter[]) => void;
    mocks.listVstParameters.mockImplementationOnce(() => new Promise<VstParameter[]>((resolve) => { finish = resolve; }));
    const { result } = renderHook(() => useAudioPageController());
    let pending!: Promise<void>;
    act(() => { pending = result.current.handleOpenVstParameters(); });
    await act(async () => { await result.current.handleReloadVst(); });
    await act(async () => { finish([parameter]); await pending; });
    expect(result.current.vstParamsLoading).toBe(false);
    expect(result.current.vstParameterDialogOpen).toBe(false);
    expect(result.current.vstParams).toEqual([]);
    expect(mocks.reloadVst).toHaveBeenCalledOnce();
  });

  it("invalidates parameters when a restart is observed with the worker still ready", async () => {
    mocks.listVstParameters.mockResolvedValueOnce([parameter]);
    mocks.bridge.status = { running: true, audioRunning: true, vstWorkerState: "ready", vstWorkerRestarts: 0 } as RuntimeStatus;
    const { result, rerender } = renderHook(() => useAudioPageController());
    await act(async () => { await result.current.handleOpenVstParameters(); });
    expect(result.current.vstParameterDialogOpen).toBe(true);
    mocks.bridge.metrics = { vstWorkerState: "ready", vstWorkerRestarts: 1 };
    rerender();
    expect(result.current.vstParameterDialogOpen).toBe(false);
    expect(result.current.vstParams).toEqual([]);
  });

  it("disables parameter actions while the worker is faulted", async () => {
    mocks.bridge.status = { running: true, audioRunning: false, vstWorkerState: "faulted" } as RuntimeStatus;
    mocks.bridge.config = { ...currentConfig(), audio: { ...currentConfig().audio, vstPluginId: "B", vstPath: "B.dll" } };
    mocks.listVstParameters.mockRejectedValueOnce(new Error("worker unavailable"));
    const { result } = renderHook(() => useAudioPageController());
    await waitFor(() => expect(result.current.vstPlugins).toEqual([replacement]));
    await act(async () => { await result.current.handleOpenVstParameters(); });
    expect(result.current.canOpenVstParameterFallback).toBe(false);
    expect(result.current.canOpenSelectedVstUi).toBe(false);
    expect(mocks.listVstParameters).not.toHaveBeenCalled();
  });

  it("cancels queued parameter edits and closes the catalogue on worker loss", async () => {
    mocks.listVstParameters.mockResolvedValueOnce([parameter]);
    const frames = new Map<number, FrameRequestCallback>();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => { frames.set(1, callback); return 1; });
    const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => { frames.delete(id); });
    const { result, rerender, unmount } = renderHook(() => useAudioPageController());
    await act(async () => { await result.current.handleOpenVstParameters(); });
    expect(result.current.vstParameterDialogOpen).toBe(true);
    await act(async () => { await result.current.handleVstParamChange(0, 0.9); });
    mocks.bridge.status = { running: true, audioRunning: false, vstWorkerState: "faulted" } as RuntimeStatus;
    rerender();
    const openAfterLoss = result.current.vstParameterDialogOpen;
    const paramsAfterLoss = result.current.vstParams;
    unmount();
    expect(openAfterLoss).toBe(false);
    expect(paramsAfterLoss).toEqual([]);
    expect(cancel).toHaveBeenCalledWith(1);
    expect(frames.size).toBe(0);
    expect(mocks.setVstParameter).not.toHaveBeenCalled();
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

  it("persists config changes and reports live control errors", async () => {
    mocks.setMasterGain.mockRejectedValueOnce(new Error("gain queue full"));
    mocks.setAudioLimiter.mockRejectedValueOnce(new Error("limiter queue full"));
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => {
      await result.current.handleGainChange(-12);
      await result.current.handleLimiterToggle(false);
    });
    expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: { gainDb: -12 } });
    expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: { limiterEnabled: false } });
    expect(mocks.toastError.mock.calls).toEqual([["gain queue full"], ["limiter queue full"]]);
  });

  it("does not call live controls when audio is not running", async () => {
    mocks.bridge.status = { running: true, audioRunning: false } as RuntimeStatus;
    const { result } = renderHook(() => useAudioPageController());
    await act(async () => {
      await result.current.handleGainChange(-6);
      await result.current.handleLimiterToggle(true);
    });
    expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: { gainDb: -6 } });
    expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: { limiterEnabled: true } });
    expect(mocks.setMasterGain).not.toHaveBeenCalled();
    expect(mocks.setAudioLimiter).not.toHaveBeenCalled();
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

  it.each([{ renderAheadBlocks: 2 }, { dspThreadPriority: "timeCritical" as const }])(
    "persists and reloads a renderer setting: %j", async (patch) => {
      const { result } = renderHook(() => useAudioPageController());
      let accepted: boolean | undefined;
      await act(async () => { accepted = await result.current.updateAudioConfig(patch); });
      expect(accepted).toBe(true);
      expect(mocks.updateConfig).toHaveBeenCalledWith({ audio: patch });
      expect(mocks.saveConfig.mock.calls[0][0].audio).toMatchObject(patch);
      expect(mocks.reloadVst).toHaveBeenCalledOnce();
    }
  );

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
