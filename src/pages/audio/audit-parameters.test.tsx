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
