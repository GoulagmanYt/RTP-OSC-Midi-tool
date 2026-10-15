import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import type { AppConfig, RuntimeStatus } from "../../api-types";
import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../../constants";
import { SignalFlow } from "./SignalFlow";

const t = (key: string) => key;
const config = {
  midi: { inputDevice: "Keystation 49", outputDevice: "loopMIDI", thruEnabled: true },
  rtp: { enabled: true, thruEnabled: true, sessionName: "Studio", port: 5004 },
  osc: { enabled: true, inputEnabled: false, listenIp: "0.0.0.0", listenPort: 9001, targetIp: "127.0.0.1", targetPort: 9000 },
  audio: { enabled: false },
} as AppConfig;

describe("SignalFlow", () => {
  it("links each configured source and destination to its settings", () => {
    render(<MemoryRouter><SignalFlow config={config} status={null} t={t} /></MemoryRouter>);
    expect(screen.getByRole("link", { name: /Keystation 49/ }).getAttribute("href")).toBe("/routing");
    expect(screen.getByRole("link", { name: /loopMIDI/ }).getAttribute("href")).toBe("/routing");
    expect(screen.getByRole("link", { name: /127.0.0.1:9000/ }).getAttribute("href")).toBe("/osc");
    expect(screen.getByRole("link", { name: /dashboard.flow.instrument/ }).getAttribute("href")).toBe("/audio");
  });

  it("keeps configured connections separate from the stopped bridge state", () => {
    render(<MemoryRouter><SignalFlow config={config} status={{ running: false, rtpActive: true } as RuntimeStatus} t={t} /></MemoryRouter>);
    expect(screen.getByText("common.stopped")).toBeTruthy();
    expect(screen.getByText("dashboard.flow.description")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Keystation 49/ }).textContent).toContain("common.enabled");
    expect(screen.getByRole("link", { name: /dashboard.flow.instrument/ }).textContent).toContain("common.disabled");
    expect(screen.queryByText("common.running")).toBeNull();
  });

  it("translates virtual MIDI ports instead of showing internal names", () => {
    const virtualConfig = { ...config, midi: { ...config.midi, inputDevice: RTP_VIRTUAL_INPUT, outputDevice: VST_OUTPUT } };
    render(<MemoryRouter><SignalFlow config={virtualConfig} status={null} t={t} /></MemoryRouter>);
    expect(screen.getByText("devices.rtpVirtualInput")).toBeTruthy();
    expect(screen.getByText("devices.vstInternalOutput")).toBeTruthy();
    expect(screen.queryByText(RTP_VIRTUAL_INPUT)).toBeNull();
  });

  it("shows unknown state and useful empty labels while configuration is unavailable", () => {
    render(<MemoryRouter><SignalFlow config={null} status={null} t={t} /></MemoryRouter>);
    expect(screen.getByText("common.unknown")).toBeTruthy();
    expect(screen.getAllByText("common.none").length).toBeGreaterThan(0);
    expect(screen.queryByText(/undefined|null/)).toBeNull();
    expect(screen.queryByText("common.stopped")).toBeNull();
  });

  it.each(["thruEnabled", "remoteEnabled", "virtualInput"])("shows RTP requested by %s even with its server switch off", (trigger) => {
    const requested = {
      ...config,
      midi: { ...config.midi, inputDevice: trigger === "virtualInput" ? RTP_VIRTUAL_INPUT : config.midi.inputDevice },
      rtp: { ...config.rtp, enabled: false, thruEnabled: trigger === "thruEnabled", remoteEnabled: trigger === "remoteEnabled" },
    };
    render(<MemoryRouter><SignalFlow config={requested} status={null} t={t} /></MemoryRouter>);
    const routes = screen.getAllByRole("link", { name: /RTP-MIDI/ });
    expect(routes[0].textContent).toContain("common.enabled");
    expect(routes[1].textContent).toContain(trigger === "thruEnabled" ? "common.enabled" : "common.disabled");
  });
});
