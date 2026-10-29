import { useCallback, useEffect, useRef } from "react";
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
  beginStatusUpdate?: () => (status: api.RuntimeStatus) => void;
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
  beginStatusUpdate,
  setPreflight,
  setMetrics,
  refreshStatus,
  refreshLists,
  refreshAudioDevices,
  t,
}: BridgeActionsOptions) {
  const toggleInProgress = useRef(false);
  const mounted = useRef(true);
  const preflightRequestId = useRef(0);
  const reloadRequestId = useRef(0);

  useEffect(() => {
    preflightRequestId.current += 1;
    reloadRequestId.current += 1;
  }, [config]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      preflightRequestId.current += 1;
      reloadRequestId.current += 1;
    };
  }, []);

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
    const requestId = ++reloadRequestId.current;
    try {
      const fresh = accepted ?? await api.getConfig();
      if (!mounted.current || requestId !== reloadRequestId.current) return;
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
    const requestId = ++preflightRequestId.current;
    try {
      const report = await api.preflightCheck(config);
      if (!mounted.current || requestId !== preflightRequestId.current) return null;
      setPreflight(report);
      return report;
    } catch (error) {
      if (!mounted.current || requestId !== preflightRequestId.current) return null;
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
      if (!report) return;
      if (
        (!report.midiInOk || !report.midiOutOk || !report.audioBackendOk || !report.rtpPortOk)
      ) {
        toast.error(t("toasts.bridge.preflightBlocked"));
        return;
      }

      const acceptStatus = beginStatusUpdate?.() ?? setStatus;
      acceptStatus(await api.startBridge(config));
      await refreshStatus();
    } catch (error) {
      console.error("Failed to toggle bridge", error);
      const message = getErrorMessage(error);
      toast.error(message || t("toasts.bridge.bridgeActionFailed"));
      await refreshStatus();
    } finally {
      toggleInProgress.current = false;
    }
  }, [beginStatusUpdate, config, refreshStatus, runPreflight, setMetrics, setStatus, status, t]);

  return {
    updateConfig,
    reloadConfig,
    runPreflight,
    saveConfig,
    toggleBridge,
  };
}
