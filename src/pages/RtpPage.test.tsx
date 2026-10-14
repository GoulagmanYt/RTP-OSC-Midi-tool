import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";
import type { RtpParticipantInfo } from "../api-types";
import RtpPage from "./RtpPage";

const mocks = vi.hoisted(() => ({
  callbacks: new Map<string, (event: { payload: unknown }) => void>(),
  getParticipants: vi.fn(),
  t: (key: string) => key,
}));
vi.mock("../providers/BridgeProvider", () => ({ useBridge: () => ({
  config: null, status: { rtpActive: true }, updateConfig: vi.fn(), saveConfig: vi.fn(),
}) }));
vi.mock("../providers/LanguageProvider", () => ({ useI18n: () => ({ t: mocks.t }) }));
vi.mock("../api", () => ({ getRtpParticipants: mocks.getParticipants, refreshRtpSessions: vi.fn(async () => []) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, callback) => {
  mocks.callbacks.set(name, callback);
  return () => mocks.callbacks.delete(name);
}) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const existing = [{ name: "Existing keyboard", addr: "192.168.1.2:5004" }];
beforeEach(() => {
  vi.clearAllMocks();
  mocks.callbacks.clear();
  mocks.getParticipants.mockResolvedValue(existing);
});

it("loads existing participants on opening the RTP page and returning after navigation", async () => {
  const first = render(<RtpPage />);
  await waitFor(() => expect(screen.getByText("Existing keyboard")).toBeTruthy());
  first.unmount();
  render(<RtpPage />);
  await waitFor(() => expect(screen.getByText("Existing keyboard")).toBeTruthy());
  expect(mocks.getParticipants).toHaveBeenCalledTimes(2);
});

it("preserves a newer participant event when an older snapshot returns later", async () => {
  let resolveSnapshot!: (participants: RtpParticipantInfo[]) => void;
  mocks.getParticipants.mockImplementationOnce(() => new Promise<RtpParticipantInfo[]>((resolve) => { resolveSnapshot = resolve; }));
  render(<RtpPage />);
  await waitFor(() => expect(mocks.getParticipants).toHaveBeenCalledOnce());
  act(() => mocks.callbacks.get("rtp:participants")!({ payload: [{ name: "New keyboard", addr: "192.168.1.3:5004" }] }));
  await act(async () => { resolveSnapshot(existing); });
  expect(screen.getByText("New keyboard")).toBeTruthy();
  expect(screen.queryByText("Existing keyboard")).toBeNull();
});

it("cleans up its subscriptions and ignores a snapshot completing after navigation", async () => {
  let resolveSnapshot!: (participants: RtpParticipantInfo[]) => void;
  mocks.getParticipants.mockImplementationOnce(() => new Promise<RtpParticipantInfo[]>((resolve) => { resolveSnapshot = resolve; }));
  const page = render(<RtpPage />);
  await waitFor(() => expect(mocks.getParticipants).toHaveBeenCalledOnce());
  page.unmount();
  expect(mocks.callbacks.size).toBe(0);
  await act(async () => { resolveSnapshot(existing); });
  expect(screen.queryByText("Existing keyboard")).toBeNull();
});

it("unsubscribes listeners that finish registering after the page has unmounted", async () => {
  let finishSessions!: (unlisten: () => void) => void;
  let finishParticipants!: (unlisten: () => void) => void;
  vi.mocked(listen)
    .mockImplementationOnce(() => new Promise((resolve) => { finishSessions = resolve; }))
    .mockImplementationOnce(() => new Promise((resolve) => { finishParticipants = resolve; }));
  const sessionsCleanup = vi.fn();
  const participantsCleanup = vi.fn();
  const page = render(<RtpPage />);
  page.unmount();
  await act(async () => {
    finishSessions(sessionsCleanup);
    finishParticipants(participantsCleanup);
  });
  expect(sessionsCleanup).toHaveBeenCalledOnce();
  expect(participantsCleanup).toHaveBeenCalledOnce();
  expect(mocks.getParticipants).not.toHaveBeenCalled();
});
