import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig } from "../../api-types";
import { useAppBootstrap } from "./useAppBootstrap";

const mocks = vi.hoisted(() => ({ getConfig: vi.fn(), preflight: vi.fn(), start: vi.fn() }));
vi.mock("../../api", () => ({
  getConfig: mocks.getConfig, preflightCheck: mocks.preflight, startBridge: mocks.start,
}));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getConfig.mockResolvedValue({ audio: { backend: "auto" }, ui: { autoStart: true } } as AppConfig);
  mocks.preflight.mockResolvedValue({ midiInOk: true });
  mocks.start.mockResolvedValue({ running: true });
});

it.each(["device lists", "preflight"])("does not auto-start after unmount while waiting for %s", async (stage) => {
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  const refreshLists = vi.fn(() => stage === "device lists" ? pending : Promise.resolve());
  if (stage === "preflight") mocks.preflight.mockImplementationOnce(() => pending);
  const options = {
    setConfig: vi.fn(), setStatus: vi.fn(), setPreflight: vi.fn(),
    setIsLoading: vi.fn(), setInitializationError: vi.fn(),
    refreshStatus: vi.fn(async () => undefined), refreshLists,
    refreshAudioDevices: vi.fn(async () => undefined),
  };
  const { unmount } = renderHook(() => useAppBootstrap(options));
  await act(async () => { await Promise.resolve(); });
  expect(refreshLists).toHaveBeenCalledOnce();
  if (stage === "preflight") expect(mocks.preflight).toHaveBeenCalledOnce();
  unmount();
  await act(async () => { finish(); await pending; });
  expect(mocks.start).not.toHaveBeenCalled();
});

it("auto-starts once initialization completes while still mounted", async () => {
  const options = {
    setConfig: vi.fn(), setStatus: vi.fn(), setPreflight: vi.fn(),
    setIsLoading: vi.fn(), setInitializationError: vi.fn(),
    refreshStatus: vi.fn(async () => undefined), refreshLists: vi.fn(async () => undefined),
    refreshAudioDevices: vi.fn(async () => undefined),
  };
  renderHook(() => useAppBootstrap(options));
  await act(async () => { await Promise.resolve(); });
  expect(mocks.start).toHaveBeenCalledOnce();
  expect(options.setStatus).toHaveBeenCalledWith({ running: true });
});
