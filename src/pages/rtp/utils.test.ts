import { describe, expect, it } from "vitest";
import type { RtpParticipantInfo, RtpRemoteEntry, RtpSessionInfo } from "../../api-types";
import {
  buildRemoteConnectionChecker,
  mergeSessionIntoRemotes,
  syncAutoConnectRemotes,
} from "./utils";

describe("syncAutoConnectRemotes", () => {
  it("updates auto-connect remotes when a discovered session matches by name", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "Stage Piano", host: "10.0.0.10", port: 5004, autoConnect: true },
      { id: "2", name: "Drums", host: "10.0.0.20", port: 5006, autoConnect: false },
    ];
    const sessions: RtpSessionInfo[] = [
      { name: "stage piano", host: "192.168.1.50", port: 6000, addresses: ["192.168.1.51"] },
    ];

    const result = syncAutoConnectRemotes(remotes, sessions);

    expect(result.changed).toBe(true);
    expect(result.remotes[0]).toEqual({
      id: "1",
      name: "Stage Piano",
      host: "192.168.1.51",
      port: 6000,
      autoConnect: true,
    });
    expect(result.remotes[1]).toEqual(remotes[1]);
  });
});

describe("mergeSessionIntoRemotes", () => {
  it("updates an existing remote instead of duplicating it", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "Keys", host: "10.0.0.5", port: 5004, autoConnect: false },
    ];
    const session: RtpSessionInfo = {
      name: "Keys",
      host: "192.168.0.8",
      port: 5008,
      addresses: ["192.168.0.9"],
    };

    const result = mergeSessionIntoRemotes(remotes, session, () => "new-id");

    expect(result).toEqual([
      { id: "1", name: "Keys", host: "192.168.0.9", port: 5008, autoConnect: true },
    ]);
  });
});

describe("buildRemoteConnectionChecker", () => {
  it("marks remotes as connected by address or participant name", () => {
    const participants: RtpParticipantInfo[] = [
      { name: "Keys", addr: "192.168.0.9:5008" },
      { name: "Pad", addr: "192.168.0.10:5004" },
    ];
    const isConnected = buildRemoteConnectionChecker(participants);

    expect(
      isConnected({ id: "1", name: "Keys", host: "192.168.0.1", port: 5004, autoConnect: true })
    ).toBe(true);
    expect(
      isConnected({ id: "2", name: "", host: "192.168.0.9", port: 5008, autoConnect: true })
    ).toBe(true);
    expect(
      isConnected({ id: "3", name: "Bass", host: "192.168.0.99", port: 5004, autoConnect: true })
    ).toBe(false);
  });
});
