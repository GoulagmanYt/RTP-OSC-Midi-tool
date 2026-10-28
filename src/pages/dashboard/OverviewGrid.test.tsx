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
  entry("info", "internal buffer configuration"),
  entry("debug", "another private debug entry"),
  entry("warn", "native-driver HRESULT 0x123"),
  entry("warn", "same warning repeated"),
  entry("error", "internal callback queue failed"),
];

function overview(developerMode: boolean, logs = history) {
  return <MemoryRouter><OverviewGrid
    config={{ ...config, ui: { ...config.ui, developerMode } }}
    audioDevice="Speakers" limiterEnabled
    logs={logs} status={null} t={(key) => key} onNavigateLogs={vi.fn()}
  /></MemoryRouter>;
}

describe("OverviewGrid recent events", () => {
  it("summarizes useful incidents before limiting the preview and omits diagnostic content in normal mode", () => {
    render(overview(false));
    const events = screen.getByRole("region", { name: "dashboard.quickLogs" });
    expect(within(events).getByText("dashboard.eventsWarning")).toBeTruthy();
    expect(within(events).getByText("dashboard.eventsError")).toBeTruthy();
    expect(within(events).getAllByRole("listitem")).toHaveLength(2);
    for (const log of history) expect(within(events).queryByText(log.message)).toBeNull();
    expect(within(events).queryByRole("button", { name: /dashboard.viewMore/ })).toBeNull();
  });

  it("hides retained and newly received debug entries when leaving developer mode and preserves detailed history", () => {
    const { rerender } = render(overview(true));
    expect(screen.getByText(history[0].message)).toBeTruthy();

    const nextHistory = [entry("debug", "queued debug after mode switch"), ...history];
    rerender(overview(false, nextHistory));
    for (const log of nextHistory) expect(screen.queryByText(log.message)).toBeNull();
    expect(screen.getByText("dashboard.eventsWarning")).toBeTruthy();
    expect(screen.getByText("dashboard.eventsError")).toBeTruthy();

    rerender(overview(true, nextHistory));
    expect(screen.getByText(nextHistory[0].message)).toBeTruthy();
    expect(screen.getByText(history[0].message)).toBeTruthy();
    expect(history).toHaveLength(6);
  });

  it("keeps the complete message available in the compact preview", () => {
    render(overview(true));
    expect(screen.getByText(history[0].message).getAttribute("title")).toBe(history[0].message);
    expect(screen.getByRole("list", { name: "dashboard.quickLogs" }).getAttribute("tabindex")).toBe("0");
  });

  it("shows an empty overview for a normal session containing only informational and debug logs", () => {
    render(overview(false, history.slice(0, 3)));
    expect(screen.getByText("dashboard.noRecentLogs")).toBeTruthy();
    expect(screen.queryByRole("listitem")).toBeNull();
  });
});
