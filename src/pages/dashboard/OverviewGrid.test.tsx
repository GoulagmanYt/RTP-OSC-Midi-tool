import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import type { AppConfig, LogEntry } from "../../api-types";
import { OverviewGrid } from "./OverviewGrid";

const config = {
  ui: { developerMode: true },
  audio: { enabled: true, gainDb: 0 },
  osc: { enabled: true, targetIp: "127.0.0.1", targetPort: 9000 },
  rtp: { sessionName: "Studio", port: 5004 },
} as AppConfig;

const entry = (level: LogEntry["level"], message: string): LogEntry => ({ level, message, timestamp: "12:00:00" });
const history = [
  entry("debug", "worker IPC private diagnostic"),
  entry("info", "Bridge started"),
  entry("debug", "another private debug entry"),
  entry("warn", "native-driver HRESULT 0x123"),
  entry("warn", "same warning repeated"),
  entry("error", "internal callback queue failed"),
  entry("info", "MIDI input connected"),
  entry("info", "OSC target ready"),
  entry("info", "Older session event"),
];

function overview(developerMode: boolean, logs = history) {
  return <MemoryRouter><OverviewGrid
    config={{ ...config, ui: { ...config.ui, developerMode } }}
    audioDevice="Speakers" limiterEnabled
    logs={logs} status={null} t={(key) => key} onNavigateLogs={vi.fn()}
  /></MemoryRouter>;
}

describe("OverviewGrid recent events", () => {
  it("shows the latest six basic messages in normal mode, including repeated warning levels", () => {
    render(overview(false));
    const events = screen.getByRole("region", { name: "dashboard.quickLogs" });
    const rows = within(events).getAllByRole("listitem");
    expect(rows).toHaveLength(6);
    expect(rows.map((row) => within(row).getByTitle(/.+/).textContent)).toEqual([
      "Bridge started", "native-driver HRESULT 0x123", "same warning repeated",
      "internal callback queue failed", "MIDI input connected", "OSC target ready",
    ]);
    expect(within(events).queryByText(history[0].message)).toBeNull();
    expect(within(events).queryByText(history[2].message)).toBeNull();
    expect(within(events).queryByText("Older session event")).toBeNull();
    expect(within(events).queryByRole("button", { name: /dashboard.viewMore/ })).toBeNull();
  });

  it("hides retained and newly received debug entries when leaving developer mode and preserves detailed history", () => {
    const { rerender } = render(overview(true));
    expect(screen.getByText(history[0].message)).toBeTruthy();

    const nextHistory = [entry("debug", "queued debug after mode switch"), ...history];
    rerender(overview(false, nextHistory));
    for (const log of nextHistory.filter((log) => log.level === "debug")) {
      expect(screen.queryByText(log.message)).toBeNull();
    }
    expect(screen.getByText("Bridge started")).toBeTruthy();
    expect(screen.getByText("native-driver HRESULT 0x123")).toBeTruthy();

    rerender(overview(true, nextHistory));
    expect(screen.getByText(nextHistory[0].message)).toBeTruthy();
    expect(screen.getByText(history[0].message)).toBeTruthy();
    expect(history).toHaveLength(9);
  });

  it("keeps the complete message available in the compact preview", () => {
    render(overview(true));
    expect(screen.getByText(history[0].message).getAttribute("title")).toBe(history[0].message);
    expect(screen.getByRole("list", { name: "dashboard.quickLogs" }).getAttribute("tabindex")).toBe("0");
    expect(screen.getAllByRole("listitem")).toHaveLength(6);
    expect(screen.queryByText("MIDI input connected")).toBeNull();
  });

  it("shows errors with their complete message in normal mode", () => {
    render(overview(false));
    expect(screen.getByText(history[5].message).getAttribute("title")).toBe(history[5].message);
  });

  it("shows an empty overview for a normal session containing only debug logs", () => {
    render(overview(false, [history[0], history[2]]));
    expect(screen.getByText("dashboard.noRecentLogs")).toBeTruthy();
    expect(screen.queryByRole("listitem")).toBeNull();
  });
});
