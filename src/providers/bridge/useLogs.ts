import { useCallback, useRef, useState } from "react";
import type { LogEntry } from "../../api";

const LOG_BUFFER_SIZE = 1000;

export function useLogs() {
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const logBufferRef = useRef<LogEntry[]>([]);
  const logIndexRef = useRef(0);

  const appendLog = useCallback((entry: LogEntry) => {
    const buffer = logBufferRef.current;
    buffer[logIndexRef.current] = entry;
    logIndexRef.current = (logIndexRef.current + 1) % LOG_BUFFER_SIZE;

    const nextLogs: LogEntry[] = [];
    const size = Math.min(buffer.length, LOG_BUFFER_SIZE);
    for (let i = 0; i < size; i += 1) {
      const index = (logIndexRef.current - 1 - i + LOG_BUFFER_SIZE) % LOG_BUFFER_SIZE;
      if (buffer[index]) {
        nextLogs.push(buffer[index]);
      }
    }
    setLogs(nextLogs);
  }, []);

  const clearLogs = useCallback(() => {
    logBufferRef.current = [];
    logIndexRef.current = 0;
    setLogs([]);
  }, []);

  return {
    logs,
    appendLog,
    clearLogs,
  };
}
