import { useCallback, useState } from "react";
import * as api from "../api";
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
  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  const [midiInputs, setMidiInputs] = useState<string[]>([]);
  const [midiOutputs, setMidiOutputs] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [preflight, setPreflight] = useState<api.PreflightReport | null>(null);
  const { logs, appendLog, clearLogs } = useLogs();
  const { t } = useI18n();

  const refreshStatus = useCallback(async () => {
    try {
      const nextStatus = await api.getStatus();
      setStatus(nextStatus);
    } catch (error) {
      console.error("Failed to get status", error);
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
    } catch (error) {
      console.error("Failed to refresh lists", error);
    }
  }, []);

  const refreshAudioDevices = useCallback(async (backend?: string | null) => {
    try {
      const devices = await api.listAudioDevices(backend);
      setAudioDevices(devices);
    } catch (error) {
      console.error("Failed to list audio devices", error);
    }
  }, []);

  useAppBootstrap({
    setConfig,
    setStatus,
    setPreflight,
    setIsLoading,
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

  useAutosaveSettings({
    config,
    save: api.saveConfig,
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
