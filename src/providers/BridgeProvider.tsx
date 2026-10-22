import { useCallback, useRef, useState } from "react";
import * as api from "../api";
import { getErrorMessage } from "../api-errors";
import { toast } from "sonner";
import { BridgeContext } from "./bridge/BridgeContext";
import { useAppBootstrap } from "./bridge/useAppBootstrap";
import { useAutosaveSettings } from "./bridge/useAutosaveSettings";
import { useBridgeActions } from "./bridge/useBridgeActions";
import { useLogs } from "./bridge/useLogs";
import { useRuntimeEvents } from "./bridge/useRuntimeEvents";
import { useI18n } from "./LanguageProvider";

export { useBridge } from "./bridge/BridgeContext";

export function BridgeProvider({ children }: { children: React.ReactNode }) {
  const [config, setConfig] = useState<api.AppConfig | null>(null);
  const [status, setStatusSnapshot] = useState<api.RuntimeStatus | null>(null);
  const [metrics, setMetrics] = useState<api.RuntimeMetrics | null>(null);
  const [audioBackends, setAudioBackends] = useState<string[]>([]);
  const [audioDevices, setAudioDevices] = useState<api.AudioDeviceEntry[]>([]);
  const [midiInputs, setMidiInputs] = useState<string[]>([]);
  const [midiOutputs, setMidiOutputs] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [initializationError, setInitializationError] = useState<string | null>(null);
  const [preflight, setPreflight] = useState<api.PreflightReport | null>(null);
  const { logs, appendLog, clearLogs } = useLogs();
  const { t } = useI18n();

  const statusRequestId = useRef(0);
  const workerEventRevision = useRef(0);
  const latestWorkerMetrics = useRef<api.RuntimeMetrics | null>(null);
  const currentStatus = useRef<api.RuntimeStatus | null>(null);
  const listsRequestId = useRef(0);
  const audioDevicesRequestId = useRef(0);

  const applyStatus = useCallback<React.Dispatch<React.SetStateAction<api.RuntimeStatus | null>>>((next) => {
    const resolved = typeof next === "function" ? next(currentStatus.current) : next;
    if (resolved === currentStatus.current) return;
    currentStatus.current = resolved;
    setStatusSnapshot(resolved);
  }, []);

  const setStatus = useCallback<React.Dispatch<React.SetStateAction<api.RuntimeStatus | null>>>((next) => {
    statusRequestId.current += 1;
    applyStatus(next);
  }, [applyStatus]);

  const receiveMetrics = useCallback((next: api.RuntimeMetrics) => {
    const previous = latestWorkerMetrics.current;
    const previousState = previous?.vstWorkerState ?? currentStatus.current?.vstWorkerState;
    const previousRestarts = previous?.vstWorkerRestarts ?? currentStatus.current?.vstWorkerRestarts;
    if (next.vstWorkerState && (next.vstWorkerState !== previousState ||
      next.vstWorkerState !== currentStatus.current?.vstWorkerState ||
      next.vstWorkerRestarts !== previousRestarts)) {
      workerEventRevision.current += 1;
    }
    latestWorkerMetrics.current = next;
    setMetrics(next);
  }, []);

  const beginStatusUpdate = useCallback(() => {
    const id = ++statusRequestId.current;
    const workerRevision = workerEventRevision.current;
    const workerWasDisabled = currentStatus.current?.vstWorkerState === "disabled";
    return (nextStatus: api.RuntimeStatus) => {
      if (id === statusRequestId.current) {
        // Keep newer worker events without discarding the snapshot's routing
        // fields. An explicit stop remains authoritative over queued metrics.
        const worker = latestWorkerMetrics.current;
        applyStatus(workerRevision !== workerEventRevision.current && worker?.vstWorkerState &&
          nextStatus.vstWorkerState !== "disabled" && !(workerWasDisabled && worker.vstWorkerState === "disabled")
          ? { ...nextStatus, audioRunning: worker.vstWorkerState === "ready", vstWorkerState: worker.vstWorkerState }
          : nextStatus);
      }
    };
  }, [applyStatus]);

  const refreshStatus = useCallback(async () => {
    const acceptStatus = beginStatusUpdate();
    try {
      acceptStatus(await api.getStatus());
    } catch (error) {
      console.error("Failed to get status", error);
    }
  }, [beginStatusUpdate]);

  const refreshLists = useCallback(async () => {
    const id = ++listsRequestId.current;
    try {
      const [inputs, outputs, backends] = await Promise.all([
        api.listMidiInputs(),
        api.listMidiOutputs(),
        api.listAudioBackends(),
      ]);
      if (id === listsRequestId.current) {
        setMidiInputs(inputs);
        setMidiOutputs(outputs);
        setAudioBackends(backends);
      }
    } catch (error) {
      console.error("Failed to refresh lists", error);
    }
  }, []);

  const refreshAudioDevices = useCallback(async (backend?: string | null) => {
    const id = ++audioDevicesRequestId.current;
    try {
      const devices = await api.listAudioDevices(backend);
      if (id === audioDevicesRequestId.current) {
        setAudioDevices(devices);
      }
    } catch (error) {
      console.error("Failed to list audio devices", error);
    }
  }, []);

  useAppBootstrap({
    setConfig,
    beginStatusUpdate,
    setPreflight,
    setIsLoading,
    setInitializationError,
    refreshStatus,
    refreshLists,
    refreshAudioDevices,
  });

  useRuntimeEvents({
    appendLog,
    setMetrics: receiveMetrics,
    setStatus: applyStatus,
    t,
  });

  const { updateConfig, saveConfig, toggleBridge, runPreflight, reloadConfig } = useBridgeActions({
    config,
    status,
    setConfig,
    setStatus,
    beginStatusUpdate,
    setPreflight,
    setMetrics,
    refreshStatus,
    refreshLists,
    refreshAudioDevices,
    t,
  });

  const handleAutosaveError = useCallback((error: unknown) => {
    toast.error(getErrorMessage(error) || t("toasts.settings.configSaveFailed"));
  }, [t]);

  const saveAndRefreshStatus = useCallback(async (next: api.AppConfig, shouldSave?: () => boolean) => {
    const saved = await api.saveConfig(next, shouldSave);
    if (saved !== false) await refreshStatus();
    return saved;
  }, [refreshStatus]);

  useAutosaveSettings({
    config,
    save: saveAndRefreshStatus,
    setConfig,
    onError: handleAutosaveError,
    loadAccepted: api.getConfig,
  });

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
        initializationError,
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
