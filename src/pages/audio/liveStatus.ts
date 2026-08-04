import type { RuntimeMetrics, RuntimeStatus } from "../../api-types";

export function mergeLiveAudioMetrics(
  status: RuntimeStatus | null,
  metrics: RuntimeMetrics | null
): RuntimeStatus | null {
  return status && metrics ? { ...status, ...metrics } : status;
}
