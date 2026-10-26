import { describe, expect, it } from "vitest";
import type { AppConfig, RuntimeStatus } from "../../api-types";
import { getAudioLatency } from "./latency";
import { mergeLiveAudioMetrics } from "./liveStatus";
import type { RuntimeMetrics } from "../../api-types";

const config = { audio: { bufferSize: 512, sampleRate: 48_000 } } as AppConfig;

describe("shared audio latency", () => {
  it.each([0, 21.333, 42])("preserves the native total estimate %s including zero", (ms) => {
    expect(getAudioLatency({ audioLatencyMs: ms } as RuntimeStatus, config)).toEqual({ ms, source: "engine" });
  });

  it("uses the live estimate on both pages instead of an older status value", () => {
    const status = { audioRunning: true, vstWorkerState: "ready", audioLatencyMs: 30 } as RuntimeStatus;
    const live = { vstWorkerState: "ready", audioLatencyMs: 12 } as RuntimeMetrics;
    expect(getAudioLatency(mergeLiveAudioMetrics(status, live), config)).toEqual({ ms: 12, source: "engine" });
  });

  it.each([null, undefined, NaN, Infinity, -1])("uses a clearly identified native period when total is %s", (audioLatencyMs) => {
    const status = { audioLatencyMs, audioBufferPeriodMs: 5 } as RuntimeStatus;
    expect(getAudioLatency(status, config)).toEqual({ ms: 5, source: "bufferPeriod" });
  });

  it("calculates only one active buffer period rather than doubling requested samples", () => {
    const status = { audioBufferSize: 256, audioRequestedBufferSize: 512, audioSampleRate: 48_000 } as RuntimeStatus;
    const latency = getAudioLatency(status, config);
    expect(latency.source).toBe("bufferPeriod");
    expect(latency.ms).toBeCloseTo(256 / 48);
  });

  it("uses configuration as a period estimate before an active stream exists", () => {
    expect(getAudioLatency(null, config).ms).toBeCloseTo(512 / 48);
    expect(getAudioLatency(null, config).source).toBe("bufferPeriod");
  });

  it.each([0, -1, NaN, Infinity])("never produces a non-finite or negative result from invalid sample rate %s", (sampleRate) => {
    expect(getAudioLatency(null, { audio: { ...config.audio, sampleRate } })).toEqual({ ms: null, source: "bufferPeriod" });
  });
});
