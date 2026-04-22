import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import type { LogEntry, RuntimeMetrics } from "../../api";

type RuntimeEventsOptions = {
  appendLog: (entry: LogEntry) => void;
  setMetrics: (metrics: RuntimeMetrics) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
};

export function useRuntimeEvents({ appendLog, setMetrics, t }: RuntimeEventsOptions) {
  const lastXrunCount = useRef<number | null>(null);
  const lastDropoutToastMs = useRef(0);

  useEffect(() => {
    let disposed = false;

    const setup = async () => {
      const unlistenLog = await listen<LogEntry>("log:entry", (event) => {
        appendLog(event.payload);
      });

      const unlistenMetrics = await listen<RuntimeMetrics>("runtime:metrics", (event) => {
        const payload = event.payload;
        setMetrics(payload);

        const current = payload.audioXruns;
        if (current === undefined || current === null) {
          return;
        }
        const previous = lastXrunCount.current;
        lastXrunCount.current = current;
        if (previous === null || current <= previous) {
          return;
        }
        const now = Date.now();
        if (now - lastDropoutToastMs.current < 4000) {
          return;
        }
        lastDropoutToastMs.current = now;
        toast.error(t("toasts.audio.dropouts", { count: current - previous }));
      });

      if (disposed) {
        unlistenLog();
        unlistenMetrics();
        return;
      }

      return () => {
        unlistenLog();
        unlistenMetrics();
      };
    };

    let cleanup: (() => void) | undefined;
    setup().then((teardown) => {
      cleanup = teardown;
    });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [appendLog, setMetrics, t]);
}
