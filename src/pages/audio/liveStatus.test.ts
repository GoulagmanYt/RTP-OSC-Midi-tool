import { describe, expect, it } from "vitest";
import type { RuntimeMetrics, RuntimeStatus } from "../../api-types";
import { mergeLiveAudioMetrics } from "./liveStatus";

describe("mergeLiveAudioMetrics", () => {
  it("uses streamed audio counters without losing bridge status fields", () => {
    const status = {
      running: true,
      oscTarget: "127.0.0.1:9000",
      rtpActive: true,
      rtpAdvertisedAddresses: [],
      audioMidiDrops: 0,
      dspProcessMaxUs: 100,
    } satisfies RuntimeStatus;
    const metrics = {
      midiMessagesPerSec: 240,
      oscMessagesPerSec: 0,
      bridgeQueueDepth: 1,
      bridgeQueueMaxDepth: 4,
      bridgeMessagesIn: 500,
      bridgeMessagesOut: 500,
      bridgeMessagesDropped: 0,
      rtpMidiDrops: 0,
      reliablePlaybackMessagesIn: 0,
      reliablePlaybackMessagesOut: 0,
      reliablePlaybackDropped: 0,
      reliablePlaybackMaxLateUs: 0,
      audioMidiDrops: 7,
      dspProcessMaxUs: 321,
    } satisfies RuntimeMetrics;

    expect(mergeLiveAudioMetrics(status, metrics)).toMatchObject({
      running: true,
      oscTarget: "127.0.0.1:9000",
      audioMidiDrops: 7,
      dspProcessMaxUs: 321,
    });
  });
});
