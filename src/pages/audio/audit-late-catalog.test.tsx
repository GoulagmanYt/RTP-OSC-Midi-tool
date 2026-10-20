import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppConfig, VstParameter } from "../../api-types";
import { useAudioPageController } from "./useAudioPageController";

const mocks = vi.hoisted(() => ({
  config: {} as AppConfig,
  listParameters: vi.fn(),
  setParameter: vi.fn(),
  saveConfig: vi.fn(),
  activePlugin: "A",
}));
vi.mock("../../api", () => ({
  listVstParameters: mocks.listParameters,
  setVstParameter: mocks.setParameter,
  listVstPlugins: vi.fn(async () => [{ id: "B", path: "B.dll", supported: true }]),
  saveConfig: mocks.saveConfig,
  reloadVst: vi.fn(async () => { mocks.activePlugin = mocks.config.audio.vstPluginId!; }),
  closeVstUi: vi.fn(), openVstUi: vi.fn(), openVstFolder: vi.fn(),
  pingAudio: vi.fn(), refreshVstPlugins: vi.fn(), retestVstPlugin: vi.fn(),
  setAudioLimiter: vi.fn(), setMasterGain: vi.fn(), setVstEq: vi.fn(),
}));
vi.mock("../../providers/BridgeProvider", () => ({ useBridge: () => ({
  config: mocks.config, status: { running: true, audioRunning: true }, metrics: null,
  updateConfig: vi.fn(async () => undefined), refreshStatus: vi.fn(async () => undefined),
  audioBackends: [], audioDevices: [],
}) }));
vi.mock("../../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const oldParameter = {
  index: 0, id: 42, name: "A cutoff", value: 0.2, min: 0, max: 1,
  default: 0, flags: 0, stepCount: 0, unitId: 0, unit: "",
} satisfies VstParameter;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.activePlugin = "A";
  mocks.config = { audio: {
    enabled: true, vstPluginId: "A", vstPath: "A.dll", vstEqByPlugin: {},
  } } as AppConfig;
  mocks.setParameter.mockResolvedValue(undefined);
  mocks.saveConfig.mockImplementation(async (config: AppConfig) => { mocks.config = config; });
});
afterEach(() => vi.restoreAllMocks());

it("does not open the old native parameter catalog while a replacement is loading", async () => {
  let finishSave!: () => void;
  mocks.saveConfig.mockImplementationOnce((config: AppConfig) => new Promise<void>((resolve) => {
    mocks.config = config;
    finishSave = resolve;
  }));
  mocks.listParameters.mockResolvedValueOnce([oldParameter]);
  const { result, rerender } = renderHook(() => useAudioPageController());
  await waitFor(() => expect(result.current.vstPlugins).toHaveLength(1));
  let selection!: Promise<void>;
  await act(async () => { selection = result.current.handleSelectVst("B"); });
  rerender();
  expect(result.current.config?.audio.vstPluginId).toBe("B");
  expect(mocks.activePlugin).toBe("A");
  await act(async () => { await result.current.handleOpenVstParameters(); });
  const openedDuringReplacement = result.current.vstParameterDialogOpen;
  await act(async () => { finishSave(); await selection; });
  expect(openedDuringReplacement).toBe(false);
  expect(result.current.vstParams).toEqual([]);
});

it("audit B15: a catalog from A reopens during switching to B and sends A's index to B", async () => {
  let finish!: (parameters: VstParameter[]) => void;
  let finishSave!: () => void;
  mocks.saveConfig.mockImplementationOnce((config: AppConfig) => new Promise<void>((resolve) => {
    finishSave = () => { mocks.config = config; resolve(); };
  }));
  mocks.listParameters.mockImplementationOnce(() => new Promise<VstParameter[]>((resolve) => { finish = resolve; }));
  const frames: FrameRequestCallback[] = [];
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => frames.push(callback));
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
  const delivered: Array<{ plugin: string; index: number; value: number }> = [];
  mocks.setParameter.mockImplementation(async (index: number, value: number) => {
    delivered.push({ plugin: mocks.activePlugin, index, value });
  });
  const { result, rerender } = renderHook(() => useAudioPageController());
  await waitFor(() => expect(result.current.vstPlugins).toHaveLength(1));
  let pending!: Promise<void>;
  await act(async () => { pending = result.current.handleOpenVstParameters(); });
  expect(result.current.vstParameterDialogOpen).toBe(false);
  let selection!: Promise<void>;
  await act(async () => { selection = result.current.handleSelectVst("B"); });
  expect(result.current.vstParams).toEqual([]);
  await act(async () => { finish([oldParameter]); await pending; });
  await act(async () => { finishSave(); await selection; });
  rerender();
  expect(result.current.config?.audio.vstPluginId).toBe("B");
  // Discards stale parameters response when active plugin has changed
  expect(result.current.vstParameterDialogOpen).toBe(false);
  expect(result.current.vstParams).toEqual([]);
  expect(delivered).toEqual([]);
});

it("control: an unchanged plugin accepts its catalog", async () => {
  mocks.listParameters.mockResolvedValueOnce([oldParameter]);
  const { result } = renderHook(() => useAudioPageController());
  await act(async () => { await result.current.handleOpenVstParameters(); });
  expect(result.current.vstParameterDialogOpen).toBe(true);
  expect(result.current.vstParams).toEqual([oldParameter]);
});

it("a stale request cannot replace a newer catalog for the same plugin", async () => {
  let finish!: (parameters: VstParameter[]) => void;
  mocks.listParameters.mockImplementationOnce(() => new Promise<VstParameter[]>((resolve) => { finish = resolve; }));
  const newer = { ...oldParameter, name: "New catalog", value: 0.8 };
  mocks.listParameters.mockResolvedValueOnce([newer]);
  const { result } = renderHook(() => useAudioPageController());
  let pending!: Promise<void>;
  await act(async () => { pending = result.current.handleOpenVstParameters(); });
  await act(async () => { await result.current.handleOpenVstParameters(); });
  await act(async () => { finish([oldParameter]); await pending; });
  expect(result.current.vstParams).toEqual([newer]);
  expect(result.current.vstParameterDialogOpen).toBe(true);
});

it("switching away and back still invalidates the old catalog request", async () => {
  let finish!: (parameters: VstParameter[]) => void;
  mocks.listParameters.mockImplementationOnce(() => new Promise<VstParameter[]>((resolve) => { finish = resolve; }));
  const { result, rerender } = renderHook(() => useAudioPageController());
  let pending!: Promise<void>;
  await act(async () => { pending = result.current.handleOpenVstParameters(); });
  mocks.config = { ...mocks.config, audio: { ...mocks.config.audio, vstPluginId: "B", vstPath: "B.dll" } };
  rerender();
  mocks.config = { ...mocks.config, audio: { ...mocks.config.audio, vstPluginId: "A", vstPath: "A.dll" } };
  rerender();
  await act(async () => { finish([oldParameter]); await pending; });
  expect(result.current.vstParams).toEqual([]);
  expect(result.current.vstParameterDialogOpen).toBe(false);
});
