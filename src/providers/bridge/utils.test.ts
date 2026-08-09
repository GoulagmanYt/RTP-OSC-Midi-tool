import { describe, expect, it } from "vitest";
import type { AppConfig } from "../../api";
import { mergeConfig } from "./utils";

function makeConfig(): AppConfig {
  return {
    version: 2,
    midi: {
      inputDevice: "Input A",
      outputDevice: "Output A",
      channelFilter: null,
      thruEnabled: true,
      hotplug: true,
      routingProfiles: [],
      routingAssignments: [],
    },
    osc: {
      enabled: true,
      targetIp: "127.0.0.1",
      targetPort: 9000,
      logMessages: true,
    },
    rtp: {
      enabled: true,
      sessionName: "OSCMidi",
      port: 5004,
      remoteEnabled: false,
      remotes: [],
      logMessages: false,
    },
    audio: {
      enabled: true,
      backend: "asio",
      device: "Interface",
      sampleRate: 48000,
      bufferSize: 256,
      gainDb: 0,
      limiterEnabled: false,
      vstPath: null,
    },
    ui: {
      theme: "light",
      themePalette: "light",
      cornerRadius: 12,
      autoStart: false,
      developerMode: false,
    },
    logging: {
      enabled: true,
      verbose: false,
      logAllToFile: false,
    },
  };
}

describe("mergeConfig", () => {
  it("merges nested sections without losing sibling values", () => {
    const current = makeConfig();

    const next = mergeConfig(current, {
      audio: {
        backend: "wasapi",
      },
      logging: {
        verbose: true,
      },
    });

    expect(next.audio.backend).toBe("wasapi");
    expect(next.audio.device).toBe("Interface");
    expect(next.logging.verbose).toBe(true);
    expect(next.logging.enabled).toBe(true);
    expect(next.midi.inputDevice).toBe("Input A");
  });

  it("replaces array fields atomically", () => {
    const current = makeConfig();

    const next = mergeConfig(current, {
      midi: {
        routingProfiles: [
          {
            id: "profile-1",
            name: "Mapped",
            enabled: true,
            channelFilter: 1,
            noteMin: null,
            noteMax: null,
            ccMap: [{ from: 1, to: 74 }],
            programMap: [],
          },
        ],
      },
    });

    expect(next.midi.routingProfiles).toHaveLength(1);
    expect(next.midi.routingProfiles[0]?.ccMap[0]?.to).toBe(74);
  });
});
