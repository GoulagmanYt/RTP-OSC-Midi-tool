import type { LogEntry } from "../../api-types";

/** Keep diagnostic history intact while selecting a useful, bounded overview. */
export function selectLogPreview(logs: LogEntry[], developerMode: boolean): LogEntry[] {
  if (developerMode) return logs.slice(0, 3);

  const seen = new Set<LogEntry["level"]>();
  return logs.filter((entry) => {
    if ((entry.level !== "warn" && entry.level !== "error") || seen.has(entry.level)) return false;
    seen.add(entry.level);
    return true;
  });
}
