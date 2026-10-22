import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RuntimeMetrics } from "../../api";
import { listen } from "@tauri-apps/api/event";
import { useRuntimeEvents } from "./useRuntimeEvents";

const listeners = new Map<string, (event: { payload: unknown }) => void>();
const toastError = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    error: (...args: unknown[]) => toastError(...args),
  },
}));

function metrics(patch: Partial<RuntimeMetrics>): RuntimeMetrics {
  return {
    audioPeakL: null,
    audioPeakR: null,
    audioLatencyMs: null,
    audioXruns: 0,
    audioMidiDrops: 0,
    audioLockMisses: 0,
    audioEmergencyResets: 0,
    audioCallbackMaxUs: 0,
    audioCallbackLastUs: 0,
    audioCallbackOverBudgetCount: 0,
    audioMmcssEnabled: true,
    audioPowerThrottlingDisabled: true,
    midiMessagesPerSec: 0,
    oscMessagesPerSec: 0,
    bridgeQueueDepth: 0,
    bridgeQueueMaxDepth: 0,
    bridgeMessagesIn: 0,
    bridgeMessagesOut: 0,
    bridgeMessagesDropped: 0,
    rtpMidiDrops: 0,
    reliablePlaybackMessagesIn: 0,
    reliablePlaybackMessagesOut: 0,
    reliablePlaybackDropped: 0,
    reliablePlaybackMaxLateUs: 0,
    reliablePlaybackActiveSession: null,
    ...patch,
  };
}

describe("useRuntimeEvents", () => {
  beforeEach(() => {
    listeners.clear();
    toastError.mockClear();
    vi.spyOn(Date, "now").mockReturnValue(10_000);
  });

  it("toasts when audio health counters increase beyond xruns", async () => {
    const setMetrics = vi.fn();
    renderHook(() =>
      useRuntimeEvents({
        appendLog: vi.fn(),
        setMetrics,
        t: (key, vars) => `${key}:${vars?.count ?? ""}`,
      })
    );

    await waitFor(() => expect(listeners.has("runtime:metrics")).toBe(true));
    listeners.get("runtime:metrics")?.({ payload: metrics({}) });
    listeners.get("runtime:metrics")?.({
      payload: metrics({
        audioMidiDrops: 2,
        audioLockMisses: 1,
        audioEmergencyResets: 1,
        audioCallbackOverBudgetCount: 3,
      }),
    });

    expect(toastError).toHaveBeenCalledWith("toasts.audio.dropouts:7");
    expect(setMetrics).toHaveBeenCalledTimes(2);
  });

  it("cleans up subscriptions that finish registering after unmount", async () => {
    let finishLog!: (unlisten: () => void) => void;
    let finishMetrics!: (unlisten: () => void) => void;
    vi.mocked(listen)
      .mockImplementationOnce(() => new Promise((resolve) => { finishLog = resolve; }))
      .mockImplementationOnce(() => new Promise((resolve) => { finishMetrics = resolve; }));
    const unlistenLog = vi.fn();
    const unlistenMetrics = vi.fn();
    const setMetrics = vi.fn();
    const appendLog = vi.fn();
    const { unmount } = renderHook(() => useRuntimeEvents({ appendLog, setMetrics, t: (key) => key }));
    const calls = vi.mocked(listen).mock.calls.slice(-2);
    unmount();
    finishMetrics(unlistenMetrics);
    finishLog(unlistenLog);
    await waitFor(() => expect(unlistenLog).toHaveBeenCalledOnce());
    expect(unlistenMetrics).toHaveBeenCalledOnce();
    calls[0][1]({ event: "log:entry", id: 0, payload: { level: "error", message: "late", timestamp: "00:00:00" } });
    calls[1][1]({ event: "runtime:metrics", id: 0, payload: metrics({ audioXruns: 10 }) });
    expect(appendLog).not.toHaveBeenCalled();
    expect(setMetrics).not.toHaveBeenCalled();
  });
});
