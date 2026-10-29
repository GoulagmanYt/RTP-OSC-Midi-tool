import type { LogEntry } from "../../api-types";

/** Keep diagnostic history intact while selecting a useful, bounded overview. */
export function selectLogPreview(logs: LogEntry[], developerMode: boolean): LogEntry[] {
  const visible = developerMode ? logs : logs.filter((entry) => entry.level !== "debug");
  return visible.slice(0, 6);
}
