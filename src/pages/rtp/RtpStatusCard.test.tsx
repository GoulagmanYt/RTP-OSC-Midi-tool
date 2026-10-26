import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import type { AppConfig, RuntimeStatus } from "../../api-types";
import { RtpStatusCard } from "./RtpStatusCard";

const config = { ui: { developerMode: false }, rtp: { sessionName: "Studio", port: 5004 } } as AppConfig;
const status = { rtpActive: true, rtpBoundPort: 5004, rtpAdvertisedHost: "studio.local", rtpAdvertisedAddresses: ["192.168.1.20"] } as RuntimeStatus;
const t = (key: string) => key;

it("shows each connection value once and keeps addresses available without expanded diagnostics", () => {
  render(<RtpStatusCard config={config} status={status} t={t} />);
  expect(screen.getAllByText("Studio")).toHaveLength(1);
  expect(screen.getAllByText("5004/5005")).toHaveLength(1);
  expect(screen.getAllByText("rtp.ready")).toHaveLength(1);
  expect(screen.getByText("192.168.1.20")).toBeTruthy();
  const details = screen.getByText("rtp.networkDetails").closest("details")!;
  expect(details.open).toBe(false);
  fireEvent.click(screen.getByText("rtp.networkDetails"));
  expect(details.open).toBe(true);
  expect(screen.getByText("studio.local")).toBeTruthy();
});

it("distinguishes a bound port from a different configured port", () => {
  render(<RtpStatusCard config={{ ...config, ui: { ...config.ui, developerMode: true } }} status={{ ...status, rtpBoundPort: 6000 }} t={t} />);
  expect(screen.getByText("6000/6001")).toBeTruthy();
  expect(screen.getByText("5004")).toBeTruthy();
  expect(screen.getByText("rtp.networkDetails").closest("details")?.open).toBe(true);
});

it("does not describe configured ports as bound before the server starts", () => {
  render(<RtpStatusCard config={config} status={{ ...status, rtpActive: false }} t={t} />);
  expect(screen.getByText("rtp.connectionPorts")).toBeTruthy();
  expect(screen.queryByText("rtp.boundPorts")).toBeNull();
  expect(screen.getByText("rtp.waiting")).toBeTruthy();
});
