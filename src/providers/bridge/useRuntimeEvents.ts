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
  const lastAudioHealth = useRef<{
    xruns: number | null;
    midiDrops: number | null;
    lockMisses: number | null;
    emergencyResets: number | null;
    callbackOverBudget: number | null;
  }>({
    xruns: null,
    midiDrops: null,
    lockMisses: null,
    emergencyResets: null,
    callbackOverBudget: null,
  });
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

        const current = {
          xruns: payload.audioXruns ?? null,
          midiDrops: payload.audioMidiDrops ?? null,
          lockMisses: payload.audioLockMisses ?? null,
          emergencyResets: payload.audioEmergencyResets ?? null,
          callbackOverBudget: payload.audioCallbackOverBudgetCount ?? null,
        };
        const previous = lastAudioHealth.current;
        lastAudioHealth.current = current;

        const deltas = {
          xruns: positiveDelta(previous.xruns, current.xruns),
          midiDrops: positiveDelta(previous.midiDrops, current.midiDrops),
          lockMisses: positiveDelta(previous.lockMisses, current.lockMisses),
          emergencyResets: positiveDelta(previous.emergencyResets, current.emergencyResets),
          deadlines: positiveDelta(previous.callbackOverBudget, current.callbackOverBudget),
        };
        const increment =
          deltas.xruns + deltas.midiDrops + deltas.lockMisses + deltas.emergencyResets + deltas.deadlines;
        if (increment === 0) {
          return;
        }
        const now = Date.now();
        if (now - lastDropoutToastMs.current < 4000) {
          return;
        }
        lastDropoutToastMs.current = now;
        toast.error(
          t("toasts.audio.dropouts", {
            count: increment,
            xruns: deltas.xruns,
            deadlines: deltas.deadlines,
            lockMisses: deltas.lockMisses,
            midiDrops: deltas.midiDrops,
            resets: deltas.emergencyResets,
          })
        );
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

function positiveDelta(previous: number | null, current: number | null) {
  if (previous === null || current === null || current <= previous) {
    return 0;
  }
  return current - previous;
}
