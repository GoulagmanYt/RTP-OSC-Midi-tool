import { useCallback, useEffect, useRef, useState } from "react";
import type { BufferedLogEntry, LogEntry } from "../../api";

const LOG_BUFFER_SIZE = 1000;

export function useLogs() {
  const [logs, setLogs] = useState<BufferedLogEntry[]>([]);
  const logBufferRef = useRef<BufferedLogEntry[]>([]);
  const logIndexRef = useRef(0);
  const sequenceRef = useRef(0);
  const rafIdRef = useRef<number | null>(null);

  const flushLogs = useCallback(() => {
    if (rafIdRef.current !== null && typeof cancelAnimationFrame !== "undefined") {
      cancelAnimationFrame(rafIdRef.current);
      rafIdRef.current = null;
    }
    const buffer = logBufferRef.current;
    const nextLogs: BufferedLogEntry[] = [];
    const size = Math.min(buffer.length, LOG_BUFFER_SIZE);
    for (let i = 0; i < size; i += 1) {
      const index = (logIndexRef.current - 1 - i + LOG_BUFFER_SIZE) % LOG_BUFFER_SIZE;
      if (buffer[index]) {
        nextLogs.push(buffer[index]);
      }
    }
    setLogs(nextLogs);
  }, []);

  const appendLog = useCallback(
    (entry: LogEntry) => {
      const buffer = logBufferRef.current;
      buffer[logIndexRef.current] = { ...entry, sequence: ++sequenceRef.current };
      logIndexRef.current = (logIndexRef.current + 1) % LOG_BUFFER_SIZE;

      if (typeof requestAnimationFrame === "undefined") {
        flushLogs();
        return;
      }

      if (rafIdRef.current === null) {
        rafIdRef.current = requestAnimationFrame(() => {
          rafIdRef.current = null;
          flushLogs();
        });
      }
    },
    [flushLogs]
  );

  const clearLogs = useCallback(() => {
    if (rafIdRef.current !== null && typeof cancelAnimationFrame !== "undefined") {
      cancelAnimationFrame(rafIdRef.current);
      rafIdRef.current = null;
    }
    logBufferRef.current = [];
    logIndexRef.current = 0;
    setLogs([]);
  }, []);

  useEffect(() => {
    return () => {
      if (rafIdRef.current !== null && typeof cancelAnimationFrame !== "undefined") {
        cancelAnimationFrame(rafIdRef.current);
      }
    };
  }, []);

  return {
    logs,
    appendLog,
    clearLogs,
    flushLogs,
  };
}
