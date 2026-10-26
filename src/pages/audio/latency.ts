import type { AppConfig, RuntimeStatus } from "../../api-types";

export type AudioLatency = { ms: number | null; source: "engine" | "bufferPeriod" };

function nonNegative(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function positive(value: number | null | undefined): value is number {
  return nonNegative(value) && value > 0;
}

/** The native estimate includes driver, plug-in, bridge and render-ahead delays.
 * Without it, report the known buffer period rather than an invented total.
 * Both views pass the same merged runtime snapshot to this selector.
 */
export function getAudioLatency(status: RuntimeStatus | null, config: Pick<AppConfig, "audio"> | null): AudioLatency {
  if (nonNegative(status?.audioLatencyMs)) return { ms: status.audioLatencyMs, source: "engine" };
  if (positive(status?.audioBufferPeriodMs)) return { ms: status.audioBufferPeriodMs, source: "bufferPeriod" };
  const buffer = positive(status?.audioBufferSize) ? status.audioBufferSize : config?.audio.bufferSize;
  const sampleRate = positive(status?.audioSampleRate) ? status.audioSampleRate : config?.audio.sampleRate;
  const ms = positive(buffer) && positive(sampleRate) ? (buffer / sampleRate) * 1000 : null;
  return { ms: nonNegative(ms) ? ms : null, source: "bufferPeriod" };
}
