import { useCallback, useRef } from "react";
import type { Dispatch, SetStateAction } from "react";
import { toast } from "sonner";
import * as api from "../../api";
import { getErrorMessage } from "../../api-errors";
import { mergeConfig } from "./utils";

type BridgeActionsOptions = {
  config: api.AppConfig | null;
  status: api.RuntimeStatus | null;
  setConfig: Dispatch<SetStateAction<api.AppConfig | null>>;
  setStatus: Dispatch<SetStateAction<api.RuntimeStatus | null>>;
  setPreflight: Dispatch<SetStateAction<api.PreflightReport | null>>;
  setMetrics?: Dispatch<SetStateAction<api.RuntimeMetrics | null>>;
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
  setMetrics,
  refreshStatus,
  refreshLists,
  refreshAudioDevices,
  t,
}: BridgeActionsOptions) {
  const toggleInProgress = useRef(false);

  const updateConfig = useCallback(
    async (patch: api.DeepPartial<api.AppConfig>) => {
      if (!config) {
        return;
      }
      setConfig((current) => {
        if (!current) return current;
        const next = mergeConfig(current, patch);
        return JSON.stringify(next) === JSON.stringify(current) ? current : next;
      });
      if (patch.audio?.backend !== undefined) {
        await refreshAudioDevices(patch.audio.backend);
      }
    },
    [config, refreshAudioDevices, setConfig]
  );

  const reloadConfig = useCallback(async (accepted?: api.AppConfig) => {
    try {
      const fresh = accepted ?? await api.getConfig();
      setConfig(fresh);
      await Promise.all([
        refreshStatus(),
        refreshLists(),
        refreshAudioDevices(fresh.audio.backend),
      ]);
    } catch (error) {
      console.error("Failed to reload config", error);
    }
  }, [refreshAudioDevices, refreshLists, refreshStatus, setConfig]);

  const runPreflight = useCallback(async () => {
    if (!config) {
      return null;
    }
    try {
      const report = await api.preflightCheck(config);
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
    await refreshStatus();
  }, [config, refreshStatus]);

  const toggleBridge = useCallback(async () => {
    if (!config || !status || toggleInProgress.current) {
      return;
    }
    toggleInProgress.current = true;
    try {
      if (status.running) {
        await api.stopBridge();
        setMetrics?.(null);
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
      const message = getErrorMessage(error);
      toast.error(message || t("toasts.bridge.bridgeActionFailed"));
      await refreshStatus();
    } finally {
      toggleInProgress.current = false;
    }
  }, [config, refreshStatus, runPreflight, setMetrics, setStatus, status, t]);

  return {
    updateConfig,
    reloadConfig,
    runPreflight,
    saveConfig,
    toggleBridge,
  };
}
