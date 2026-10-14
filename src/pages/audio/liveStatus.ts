import type { RuntimeMetrics, RuntimeStatus } from "../../api-types";

export function mergeLiveAudioMetrics(
  status: RuntimeStatus | null,
  metrics: RuntimeMetrics | null
): RuntimeStatus | null {
  if (!status) return null;
  if (!metrics) return status;
  if (!status.audioRunning && status.vstWorkerState === "disabled") {
    return status;
  }
  return {
    ...status,
    ...metrics,
    audioRunning: status.audioRunning && metrics.vstWorkerState === "ready",
  };
}
