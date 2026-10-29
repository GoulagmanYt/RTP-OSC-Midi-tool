import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { useAudioPageController } from "./useAudioPageController";
import type { AppConfig } from "../../api-types";
const mocks = vi.hoisted(() => ({setParameter:vi.fn(), active:"A", update:vi.fn(), config:{} as AppConfig}));
vi.mock("../../api", () => ({
  closeVstUi:vi.fn(), listVstParameters:vi.fn(), openVstUi:vi.fn(), openVstFolder:vi.fn(),
  pingAudio:vi.fn(), refreshVstPlugins:vi.fn(), retestVstPlugin:vi.fn(),
  setAudioLimiter:vi.fn(), setMasterGain:vi.fn(), setVstEq:vi.fn(), setVstParameter:mocks.setParameter,
  listVstPlugins:vi.fn(async () => [{id:"B",path:"B.dll",supported:true}]),
  saveConfig:vi.fn(async () => true), reloadVst:vi.fn(async () => {mocks.active="B";}),
}));
vi.mock("../../providers/BridgeProvider", () => ({useBridge:() => ({
  config:mocks.config, status:{running:true,audioRunning:true}, metrics:null,
  updateConfig:mocks.update, refreshStatus:vi.fn(async () => {}), audioBackends:[], audioDevices:[],
})}));
vi.mock("../../providers/LanguageProvider", () => ({useI18n:() => ({t:(key:string)=>key})}));
vi.mock("@tauri-apps/api/event", () => ({listen:vi.fn(async () => () => undefined)}));
vi.mock("sonner", () => ({toast:{error:vi.fn(),success:vi.fn()}}));
let frames: Map<number, FrameRequestCallback>;
beforeEach(() => {
  vi.clearAllMocks(); mocks.active="A";
  mocks.config={audio:{enabled:true,vstPluginId:"A",vstPath:"A.dll",vstEqByPlugin:{}}} as AppConfig;
  mocks.update.mockResolvedValue(undefined);
  mocks.setParameter.mockResolvedValue(undefined);
  frames=new Map(); let next=0;
  vi.spyOn(window,"requestAnimationFrame").mockImplementation(callback => {frames.set(++next,callback);return next;});
  vi.spyOn(window,"cancelAnimationFrame").mockImplementation(id => {frames.delete(id);});
});
afterEach(()=>vi.restoreAllMocks());

async function runFrame() {
  await act(async () => {
    const callbacks = Array.from(frames.values());
    frames.clear();
    for (const callback of callbacks) callback(0);
  });
}

it("serializes parameter sends and coalesces to the latest pending value", async () => {
  let finish!: () => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  const { result } = renderHook(() => useAudioPageController());
  await act(async () => { await result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  for (const value of [0.3, 0.6, 0.9]) {
    await act(async () => { await result.current.handleVstParamChange(7, value); });
    await runFrame();
  }
  await act(async () => { await result.current.handleVstParamChange(8, 0.5); });
  await runFrame();
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1]]);
  await act(async () => { finish(); });
  await runFrame();
  await runFrame();
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1], [7, 0.9], [8, 0.5]]);
});

it("continues sending pending values after an earlier parameter request fails", async () => {
  let fail!: (error: Error) => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((_, reject) => { fail = reject; }));
  const { result } = renderHook(() => useAudioPageController());
  await act(async () => { await result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  await act(async () => { await result.current.handleVstParamChange(7, 0.9); });
  await act(async () => { fail(new Error("worker timeout")); });
  await runFrame();
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1], [7, 0.9]]);
});

it("flushes the last pending value after unmount while a request is in flight", async () => {
  let finish!: () => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  const { result, unmount } = renderHook(() => useAudioPageController());
  await act(async () => { await result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  await act(async () => { await result.current.handleVstParamChange(7, 0.9); });
  unmount();
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1]]);
  await act(async () => { finish(); });
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1], [7, 0.9]]);
  expect(frames.size).toBe(0);
});

it("discards pending values when the plugin changes during an in-flight request", async () => {
  let finish!: () => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  const { result } = renderHook(() => useAudioPageController());
  await waitFor(() => expect(result.current.vstPlugins).toHaveLength(1));
  await act(async () => { await result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  await act(async () => { await result.current.handleVstParamChange(7, 0.9); });
  await runFrame();
  await act(async () => { await result.current.handleSelectVst("B"); });
  await act(async () => { finish(); });
  await runFrame();
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1]]);
});

it("keeps the newest parameter value across navigation away and back", async () => {
  let finish!: () => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  const first = renderHook(() => useAudioPageController());
  await act(async () => { await first.result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  await act(async () => { await first.result.current.handleVstParamChange(7, 0.9); });
  first.unmount();
  const second = renderHook(() => useAudioPageController());
  await act(async () => { await second.result.current.handleVstParamChange(7, 0.7); });
  await runFrame();
  const callsWhileBlocked = mocks.setParameter.mock.calls.map((args) => [...args]);
  await act(async () => { finish(); });
  await runFrame();
  expect(callsWhileBlocked).toEqual([[7, 0.1]]);
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1], [7, 0.7]]);
});

it("preserves the last pending edit when reopening the same plugin without a new edit", async () => {
  let finish!: () => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  const first = renderHook(() => useAudioPageController());
  await act(async () => { await first.result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  await act(async () => { await first.result.current.handleVstParamChange(7, 0.9); });
  first.unmount();
  renderHook(() => useAudioPageController());
  await act(async () => { finish(); });
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1], [7, 0.9]]);
});

it("discards old pending edits when reopening a different plugin", async () => {
  let finish!: () => void;
  mocks.setParameter.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  const first = renderHook(() => useAudioPageController());
  await act(async () => { await first.result.current.handleVstParamChange(7, 0.1); });
  await runFrame();
  await act(async () => { await first.result.current.handleVstParamChange(7, 0.9); });
  first.unmount();
  mocks.config = { ...mocks.config, audio: { ...mocks.config.audio, vstPluginId: "B", vstPath: "B.dll" } };
  renderHook(() => useAudioPageController());
  await act(async () => { finish(); });
  expect(mocks.setParameter.mock.calls).toEqual([[7, 0.1]]);
});

it("audit: parameter queued for A must not be sent to B after plugin replacement", async () => {
  const delivered:string[]=[];
  mocks.setParameter.mockImplementation(async () => {delivered.push(mocks.active);});
  const {result}=renderHook(()=>useAudioPageController());
  await waitFor(()=>expect(result.current.vstPlugins).toHaveLength(1));
  await act(async ()=>{await result.current.handleVstParamChange(7,0.9);});
  await act(async ()=>{await result.current.handleSelectVst("B");});
  await act(async ()=>{for(const callback of frames.values()) callback(0);});
  expect(delivered).not.toContain("B");
});
it("audit: navigating away must not discard the user's last parameter change", async () => {
  const {result,unmount}=renderHook(()=>useAudioPageController());
  await waitFor(()=>expect(result.current.vstPlugins).toHaveLength(1));
  await act(async ()=>{await result.current.handleVstParamChange(7,0.9);});
  unmount();
  await act(async ()=>{for(const callback of frames.values()) callback(0);});
  expect(mocks.setParameter).toHaveBeenCalledWith(7,0.9);
  expect(mocks.setParameter).toHaveBeenCalledTimes(1);
});
