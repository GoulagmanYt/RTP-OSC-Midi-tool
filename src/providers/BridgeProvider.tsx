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
  const [status, setStatus] = useState<api.RuntimeStatus | null>(null);
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
  const listsRequestId = useRef(0);
  const audioDevicesRequestId = useRef(0);

  const refreshStatus = useCallback(async () => {
    const id = ++statusRequestId.current;
    try {
      const nextStatus = await api.getStatus();
      if (id === statusRequestId.current) {
        setStatus(nextStatus);
      }
    } catch (error) {
      console.error("Failed to get status", error);
    }
  }, []);

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
    setStatus,
    setPreflight,
    setIsLoading,
    setInitializationError,
    refreshStatus,
    refreshLists,
    refreshAudioDevices,
  });

  useRuntimeEvents({
    appendLog,
    setMetrics,
    t,
  });

  const { updateConfig, saveConfig, toggleBridge, runPreflight, reloadConfig } = useBridgeActions({
    config,
    status,
    setConfig,
    setStatus,
    setPreflight,
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
