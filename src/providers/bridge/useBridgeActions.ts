import { useCallback } from "react";
import type { Dispatch, SetStateAction } from "react";
import { toast } from "sonner";
import * as api from "../../api";
import { mergeConfig } from "./utils";

type BridgeActionsOptions = {
  config: api.AppConfig | null;
  status: api.RuntimeStatus | null;
  setConfig: Dispatch<SetStateAction<api.AppConfig | null>>;
  setStatus: Dispatch<SetStateAction<api.RuntimeStatus | null>>;
  setPreflight: Dispatch<SetStateAction<api.PreflightReport | null>>;
  refreshStatus: () => Promise<void>;
  refreshLists: () => Promise<void>;
  refreshAudioDevices: (backend?: string | null) => Promise<void>;
  t: (key: string, vars?: Record<string, string | number>) => string;
};

export function useBridgeActions({
  config,
  status,
  setConfig,
  setStatus,
  setPreflight,
  refreshStatus,
  refreshLists,
  refreshAudioDevices,
  t,
}: BridgeActionsOptions) {
  const updateConfig = useCallback(
    async (patch: api.DeepPartial<api.AppConfig>) => {
      if (!config) {
        return;
      }
      const next = mergeConfig(config, patch);
      if (JSON.stringify(next) === JSON.stringify(config)) {
        return;
      }
      setConfig(next);
      if (patch.audio?.backend !== undefined) {
        await refreshAudioDevices(next.audio.backend);
      }
    },
    [config, refreshAudioDevices, setConfig]
  );

  const reloadConfig = useCallback(async () => {
    try {
      const fresh = await api.getConfig();
      setConfig(fresh);
      await refreshLists();
      if (fresh.audio.backend) {
        await refreshAudioDevices(fresh.audio.backend);
      }
    } catch (error) {
      console.error("Failed to reload config", error);
    }
  }, [refreshAudioDevices, refreshLists, setConfig]);

  const runPreflight = useCallback(async () => {
    if (!config) {
      return null;
    }
    try {
      const report = await api.preflightCheck();
      setPreflight(report);
      return report;
    } catch (error) {
      console.error("Preflight check failed", error);
      toast.error(t("toasts.bridge.preflightFailed"));
      return null;
    }
  }, [config, setPreflight, t]);

  const saveConfig = useCallback(async () => {
    if (!config) {
      return;
    }
    await api.saveConfig(config);
  }, [config]);

  const toggleBridge = useCallback(async () => {
    if (!config || !status) {
      return;
    }
    try {
      if (status.running) {
        await api.stopBridge();
        await refreshStatus();
        return;
      }

      const report = await runPreflight();
      if (
        report &&
        (!report.midiInOk || !report.midiOutOk || !report.audioBackendOk || !report.rtpPortOk)
      ) {
        toast.error(t("toasts.bridge.preflightBlocked"));
        return;
      }

      setStatus((previous) => (previous ? { ...previous, running: true, lastError: null } : previous));
      const nextStatus = await api.startBridge(config);
      setStatus(nextStatus);
    } catch (error) {
      console.error("Failed to toggle bridge", error);
      toast.error(t("toasts.bridge.bridgeActionFailed"));
      await refreshStatus();
    }
  }, [config, refreshStatus, runPreflight, setStatus, status, t]);

  return {
    updateConfig,
    reloadConfig,
    runPreflight,
    saveConfig,
    toggleBridge,
  };
}
