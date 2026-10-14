import { describe, expect, it } from "vitest";
import type { RtpParticipantInfo, RtpRemoteEntry, RtpSessionInfo } from "../../api-types";
import {
  buildRemoteConnectionChecker,
  isSelectedSession,
  mergeSessionIntoRemotes,
  syncAutoConnectRemotes,
} from "./utils";

describe("syncAutoConnectRemotes", () => {
  it("updates auto-connect remotes when an unresolved remote matches a discovered session by name", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "Stage Piano", host: "", port: 5004, autoConnect: true },
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

  it("preserves manually configured host and port even when a discovered session matches by name", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "Stage Piano", host: "10.0.0.10", port: 5004, autoConnect: true },
    ];
    const sessions: RtpSessionInfo[] = [
      { name: "stage piano", host: "192.168.1.50", port: 6000, addresses: ["192.168.1.51"] },
    ];

    const result = syncAutoConnectRemotes(remotes, sessions);

    expect(result.changed).toBe(false);
    expect(result.remotes[0].host).toBe("10.0.0.10");
    expect(result.remotes[0].port).toBe(5004);
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
  it.each([
    ["192.168.0.9", "192.168.0.9:5004"],
    ["192.168.0.9", "192.168.0.9:5005"],
    ["192.168.0.9", "192.168.0.9:5003"],
    ["::1", "[::1]:5005"],
    ["[::1]", "[::1]:5004"],
    ["0:0:0:0:0:0:0:1", "[::1]:5004"],
    ["fe80::1%3", "[fe80::1%3]:5005"],
  ])("recognizes %s via %s without a matching name", (host, addr) => {
    const check = buildRemoteConnectionChecker([{ name: "Device", addr }]);
    expect(check({ id: "1", name: "Alias", host, port: 5004, autoConnect: true })).toBe(true);
    expect(check({ id: "1", name: "Alias", host, port: 5010, autoConnect: true })).toBe(false);
  });

  it("does not confuse different IPv6 scopes or hosts", () => {
    const check = buildRemoteConnectionChecker([{ name: "Device", addr: "[fe80::1%3]:5005" }]);
    for (const host of ["fe80::1%4", "fe80::2%3"]) {
      expect(check({ id: "1", name: "", host, port: 5004, autoConnect: true })).toBe(false);
    }
  });
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

describe("isSelectedSession", () => {
  it("matches bracketed and unbracketed IPv6 addresses", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "", host: "[fe80::1]", port: 5004, autoConnect: true },
    ];
    const session: RtpSessionInfo = {
      name: "Remote",
      host: "fe80::1",
      port: 5004,
      addresses: ["fe80::1"],
    };
    expect(isSelectedSession(remotes, session)).toBe(true);
  });

  it("matches hostnames case-insensitively", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "", host: "MacBook.local", port: 5004, autoConnect: true },
    ];
    const session: RtpSessionInfo = {
      name: "Remote",
      host: "macbook.local",
      port: 5004,
      addresses: ["192.168.1.100"],
    };
    expect(isSelectedSession(remotes, session)).toBe(true);
  });

  it("rejects non-matching ports or hosts", () => {
    const remotes: RtpRemoteEntry[] = [
      { id: "1", name: "Other", host: "192.168.1.10", port: 5004, autoConnect: true },
    ];
    const session: RtpSessionInfo = {
      name: "Remote",
      host: "192.168.1.20",
      port: 5004,
      addresses: ["192.168.1.20"],
    };
    expect(isSelectedSession(remotes, session)).toBe(false);
  });
});

