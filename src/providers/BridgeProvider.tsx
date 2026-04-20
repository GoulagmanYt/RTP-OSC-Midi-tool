import React, { createContext, useContext, useEffect, useState, useCallback, useRef } from "react";
import * as api from "../api";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { useI18n } from "./LanguageProvider";

interface BridgeContextType {
  config: api.Config | null;
  status: api.BridgeStatus | null;
  metrics: api.BridgeMetrics | null;
  logs: api.LogEvent[];
  audioBackends: string[];
  audioDevices: string[];
  midiInputs: string[];
  midiOutputs: string[];
  isLoading: boolean;
  preflight: api.PreflightReport | null;
  refreshStatus: () => Promise<void>;
  updateConfig: (newConfig: Partial<api.Config>) => Promise<void>;
  saveConfig: () => Promise<void>;
  toggleBridge: () => Promise<void>;
  clearLogs: () => void;
  refreshAudioDevices: (backend?: string | null) => Promise<void>;
  refreshLists: () => Promise<void>;
  runPreflight: () => Promise<api.PreflightReport | null>;
  reloadConfig: () => Promise<void>;
}

const BridgeContext = createContext<BridgeContextType | undefined>(undefined);

export function BridgeProvider({ children }: { children: React.ReactNode }) {
  const [config, setConfig] = useState<api.Config | null>(null);
  const [status, setStatus] = useState<api.BridgeStatus | null>(null);
  const [metrics, setMetrics] = useState<api.BridgeMetrics | null>(null);
  const [logs, setLogs] = useState<api.LogEvent[]>([]);
  const [audioBackends, setAudioBackends] = useState<string[]>([]);
  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  const [midiInputs, setMidiInputs] = useState<string[]>([]);
  const [midiOutputs, setMidiOutputs] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [preflight, setPreflight] = useState<api.PreflightReport | null>(null);
  const hasHydrated = useRef(false);
  const saveTimer = useRef<number | null>(null);
  const lastXrunCount = useRef<number | null>(null);
  const lastDropoutToastMs = useRef(0);
  const { t } = useI18n();

  // Circular log buffer to avoid O(n) array reconstruction on every log
  const LOG_BUFFER_SIZE = 1000;
  const logBufferRef = useRef<api.LogEvent[]>([]);
  const logIndexRef = useRef(0);

  const refreshStatus = useCallback(async () => {
    try {
      const s = await api.getStatus();
      setStatus(s);
    } catch (e) {
      console.error("Failed to get status", e);
    }
  }, []);

  const refreshLists = useCallback(async () => {
    try {
      const [inputs, outputs, backends] = await Promise.all([
        api.listMidiInputs(),
        api.listMidiOutputs(),
        api.listAudioBackends(),
      ]);
      setMidiInputs(inputs);
      setMidiOutputs(outputs);
      setAudioBackends(backends);
    } catch (e) {
      console.error("Failed to refresh lists", e);
    }
  }, []);

  const refreshAudioDevices = useCallback(async (backend?: string | null) => {
    try {
      const devices = await api.listAudioDevices(backend);
      setAudioDevices(devices);
    } catch (e) {
      console.error("Failed to list audio devices", e);
    }
  }, []);

  // Initial Load
  useEffect(() => {
    async function init() {
      try {
        const c = await api.getConfig();
        setConfig(c);
        await Promise.all([
          refreshStatus(),
          refreshLists(),
          c.audioBackend ? refreshAudioDevices(c.audioBackend) : Promise.resolve(),
        ]);
        try {
          const report = await api.preflightCheck();
          setPreflight(report);
        } catch (e) {
          console.warn("Preflight check failed at init", e);
        }

        // Honor persisted auto-start preference after initial state hydration.
        if (c.autoStart) {
          try {
            const bootStatus = await api.startBridge(c);
            setStatus(bootStatus);
          } catch (e) {
            console.error("Auto-start bridge failed", e);
          }
        }
      } catch (e) {
        console.error("Initialization failed", e);
      } finally {
        setIsLoading(false);
      }
    }
    init();

    const unlisten = listen<api.LogEvent>("log", (event) => {
      // Use circular buffer to avoid O(n) array reconstruction
      const buffer = logBufferRef.current;
      buffer[logIndexRef.current] = event.payload;
      logIndexRef.current = (logIndexRef.current + 1) % LOG_BUFFER_SIZE;
      
      // Only update React state with the valid portion of the buffer
      const validLogs: api.LogEvent[] = [];
      for (let i = 0; i < Math.min(buffer.length, LOG_BUFFER_SIZE); i++) {
        const idx = (logIndexRef.current - 1 - i + LOG_BUFFER_SIZE) % LOG_BUFFER_SIZE;
        if (buffer[idx]) {
          validLogs.push(buffer[idx]);
        }
      }
      setLogs(validLogs);
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, [refreshStatus, refreshLists, refreshAudioDevices]);

  useEffect(() => {
    const unlisten = listen<api.BridgeMetrics>("bridge_metrics", (event) => {
      const payload = event.payload;
      setMetrics(payload);
      const current = payload.audioXruns;
      if (current === undefined || current === null) return;
      const previous = lastXrunCount.current;
      lastXrunCount.current = current;
      if (previous === null || current <= previous) return;
      const now = Date.now();
      if (now - lastDropoutToastMs.current < 4000) return;
      lastDropoutToastMs.current = now;
      toast.error(t("toasts.audio.dropouts", { count: current - previous }));
    });

    return () => {
      unlisten.then((f) => f());
    };
  }, [t]);

  const updateConfig = useCallback(async (newConfig: Partial<api.Config>) => {
    if (!config) return;
    const updated = { ...config, ...newConfig };
    const changed = (Object.keys(newConfig) as (keyof api.Config)[]).some(
      (k) => config[k] !== updated[k]
    );
    if (!changed) return;
    setConfig(updated);
  }, [config]);

  const reloadConfig = useCallback(async () => {
    try {
      const fresh = await api.getConfig();
      setConfig(fresh);
      await refreshLists();
      if (fresh.audioBackend) {
        await refreshAudioDevices(fresh.audioBackend);
      }
    } catch (e) {
      console.error("Failed to reload config", e);
    }
  }, [refreshAudioDevices, refreshLists]);

  const runPreflight = useCallback(async () => {
    if (!config) return null;
    try {
      const report = await api.preflightCheck();
      setPreflight(report);
      return report;
    } catch (e) {
      console.error("Preflight check failed", e);
      toast.error(t("toasts.bridge.preflightFailed"));
      return null;
    }
  }, [config, t]);

  const saveConfig = useCallback(async () => {
    if (!config) return;
    try {
      await api.saveConfig(config);
    } catch (e) {
      console.error("Failed to save config", e);
    }
  }, [config]);

  const toggleBridge = useCallback(async () => {
    if (!config || !status) return;
    try {
      if (status.running) {
        await api.stopBridge();
        setStatus((prev) =>
          prev
            ? {
                ...prev,
                running: false,
                rtpActive: false,
                rtpBoundPort: null,
                lastError: null,
                vstLoaded: false,
                audioRunning: false,
                audioLatencyMs: null,
                audioBackend: null,
                audioDevice: null,
                audioSampleRate: null,
                audioBufferSize: null,
                audioRequestedBufferSize: null,
                audioStreamBufferSize: null,
                audioBufferMismatch: null,
                vstMidiCompatible: null,
                audioXruns: null,
                audioLimiterEnabled: null,
              }
            : prev
        );
      } else {
        const report = await runPreflight();
        if (
          report &&
          (!report.midiInOk || !report.midiOutOk || !report.audioBackendOk || !report.rtpPortOk)
        ) {
          toast.error(t("toasts.bridge.preflightBlocked"));
          return;
        }
        setStatus((prev) => (prev ? { ...prev, running: true, lastError: null } : prev));
        const newStatus = await api.startBridge(config);
        setStatus(newStatus);
      }
    } catch (e) {
      console.error("Failed to toggle bridge", e);
      toast.error(t("toasts.bridge.bridgeActionFailed"));
      await refreshStatus();
    }
  }, [config, status, runPreflight, t, refreshStatus]);

  const clearLogs = useCallback(() => {
    logBufferRef.current = [];
    logIndexRef.current = 0;
    setLogs([]);
  }, []);

  // FIX: Remove `refreshStatus` from the dependency array — it's stable (useCallback
  // with no deps) so it never changes, but including it caused this effect to re-run
  // on every render cycle in some environments, triggering unnecessary auto-saves.
  // The save timer already debounces writes; this just eliminates the false triggers.
  useEffect(() => {
    if (!config) return;
    if (!hasHydrated.current) {
      hasHydrated.current = true;
      return;
    }

    if (saveTimer.current) {
      window.clearTimeout(saveTimer.current);
    }

    saveTimer.current = window.setTimeout(async () => {
      try {
        await api.saveConfig(config);
      } catch (e) {
        console.error("Auto-save failed", e);
      }
    }, 350);

    return () => {
      if (saveTimer.current) {
        window.clearTimeout(saveTimer.current);
      }
    };
  }, [config]); // intentionally excludes refreshStatus — it's stable and not needed here

  return (
    <BridgeContext.Provider
      value={{
        config,
        status,
        metrics,
        logs,
        audioBackends,
        audioDevices,
        midiInputs,
        midiOutputs,
        isLoading,
        preflight,
        refreshStatus,
        updateConfig,
        saveConfig,
        toggleBridge,
        clearLogs,
        refreshAudioDevices,
        refreshLists,
        runPreflight,
        reloadConfig,
      }}
    >
      {children}
    </BridgeContext.Provider>
  );
}

export function useBridge() {
  const context = useContext(BridgeContext);
  if (context === undefined) {
    throw new Error("useBridge must be used within a BridgeProvider");
  }
  return context;
}
