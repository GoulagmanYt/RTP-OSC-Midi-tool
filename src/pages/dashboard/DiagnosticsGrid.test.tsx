import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppConfig, RuntimeMetrics, RuntimeStatus } from "../../api-types";
import { DiagnosticsGrid } from "./DiagnosticsGrid";

const config = {
  ui: { developerMode: true }, audio: { enabled: true }, midi: { hotplug: true },
  logging: { enabled: true, verbose: false, logAllToFile: false },
} as AppConfig;
const status = {
  audioRunning: true, vstLoaded: true, vstMidiCompatible: true, audioXruns: 3,
  vstWorkerState: "ready", vstWorkerRestarts: 2,
} as RuntimeStatus;
const metrics = { bridgeQueueDepth: 2, bridgeQueueMaxDepth: 4, bridgeMessagesDropped: 5, rtpMidiDrops: 6 } as RuntimeMetrics;

function diagnostics(logging = config.logging, period = false) {
  const actions = { onHandlePing: vi.fn(), onHandleReloadVst: vi.fn(), onRunPreflight: vi.fn() };
  render(<DiagnosticsGrid
    config={{ ...config, logging }} status={status} metrics={metrics}
    audioBackend="WASAPI" bufferMismatch={false} bufferSamples={512}
    requestedBufferSamples={512} streamBufferSamples={512} sampleRate={48_000}
    dropoutsActive={false} latency={{ ms: 12, source: period ? "bufferPeriod" : "engine" }}
    preflight={null} vstPath="C:\\VST\\Piano.dll" t={(key) => key} {...actions}
  />);
  return actions;
}

describe("developer diagnostic overview", () => {
  it("gives each audio measurement one location while retaining active/requested/stream distinctions", () => {
    diagnostics();
    for (const key of ["dashboard.xruns", "dashboard.sampleRate", "dashboard.bufferActive", "dashboard.bufferRequested", "dashboard.bufferStream", "dashboard.estRoundtrip"]) {
      expect(screen.getAllByText(key)).toHaveLength(1);
    }
    expect(screen.queryByText("dashboard.monitorLatency")).toBeNull();
    expect(screen.getAllByText("512 units.samples")).toHaveLength(3);
    expect(screen.getByText("12.00 units.ms")).toBeTruthy();
  });

  it("shows worker and transport diagnostics including queue pressure and losses", () => {
    diagnostics();
    for (const key of ["dashboard.workerState", "dashboard.workerRestarts", "dashboard.bridgeQueue", "dashboard.bridgeDrops", "dashboard.rtpDrops"]) {
      expect(screen.getByText(key)).toBeTruthy();
    }
    expect(screen.getByText("dashboard.workerStates.ready")).toBeTruthy();
    expect(screen.getByText("2 / 4")).toBeTruthy();
    expect(screen.getByText("5")).toBeTruthy();
    expect(screen.getByText("6")).toBeTruthy();
    expect(screen.queryByText(/undefined/)).toBeNull();
  });

  it.each([
    [{ enabled: false, verbose: true, logAllToFile: true }, "common.disabled"],
    [{ enabled: true, verbose: true, logAllToFile: false }, "dashboard.loggingToFile"],
    [{ enabled: true, verbose: false, logAllToFile: true }, "dashboard.loggingToFile"],
    [{ enabled: true, verbose: false, logAllToFile: false }, "dashboard.loggingWarningsToFile"],
  ])("reports actual file logging policy for %j", (logging, label) => {
    diagnostics(logging);
    expect(screen.getByText(label)).toBeTruthy();
    expect(screen.queryByText("dashboard.loggingConsoleOnly")).toBeNull();
  });

  it("identifies a buffer-period fallback instead of calling it total latency", () => {
    diagnostics(config.logging, true);
    expect(screen.getByText("audio.latencyBufferPeriod")).toBeTruthy();
    expect(screen.getByText("dashboard.bufferPeriodHint")).toBeTruthy();
    expect(screen.queryByText("dashboard.estRoundtrip")).toBeNull();
  });

  it("preserves diagnostic controls", () => {
    const actions = diagnostics();
    fireEvent.click(screen.getByRole("button", { name: "dashboard.ping" }));
    fireEvent.click(screen.getByRole("button", { name: "dashboard.reload" }));
    fireEvent.click(screen.getByRole("button", { name: "dashboard.rerunCheck" }));
    for (const action of Object.values(actions)) expect(action).toHaveBeenCalledOnce();
  });
});
