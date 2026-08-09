import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  closeVstUi,
  listVstParameters,
  listVstPlugins,
  openVstUi,
  pingAudio,
  refreshVstPlugins,
  reloadVst,
  saveConfig as saveConfigApi,
  setAudioLimiter,
  setMasterGain,
  setVstParameter,
} from "../../api";
import type { VstParameter, VstPluginEntry } from "../../api";
import { useI18n } from "../../providers/LanguageProvider";
import { useBridge } from "../../providers/BridgeProvider";
import { mergeLiveAudioMetrics } from "./liveStatus";
import { getErrorMessage } from "../../api-errors";

export function useAudioPageController() {
  const {
    config,
    status: statusSnapshot,
    metrics,
    updateConfig,
    saveConfig: persistConfig,
    audioBackends,
    audioDevices,
    refreshAudioDevices,
    refreshStatus,
  } = useBridge();
  const { t } = useI18n();

  // RuntimeStatus is an on-demand snapshot, while RuntimeMetrics is emitted
  // every second by the bridge. Merge both so the Audio page keeps moving
  // during normal MIDI playback, not only after buttons that refresh status.
  const status = useMemo(
    () => mergeLiveAudioMetrics(statusSnapshot, metrics),
    [statusSnapshot, metrics]
  );

  const [vstUiOpen, setVstUiOpen] = useState(false);
  const [vstParameterDialogOpen, setVstParameterDialogOpen] = useState(false);
  const [vstPlugins, setVstPlugins] = useState<VstPluginEntry[]>([]);
  const [vstPluginsLoading, setVstPluginsLoading] = useState(false);
  const [vstParams, setVstParams] = useState<VstParameter[]>([]);
  const [vstParamsLoading, setVstParamsLoading] = useState(false);
  const [audioReloading, setAudioReloading] = useState(false);
  const audioReloadInFlight = useRef(false);

  useEffect(() => {
    const unlisten = listen("audio:vst-editor-hidden", () => {
      setVstUiOpen(false);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const bridgeRunning = Boolean(status?.running);
  const isVst3 = (config?.audio.vstPath || "").toLowerCase().endsWith(".vst3");

  const activeSampleRate = status?.audioSampleRate ?? config?.audio.sampleRate ?? null;
  const activeBufferSize = status?.audioBufferSize ?? config?.audio.bufferSize ?? null;
  const requestedBufferSize = status?.audioRequestedBufferSize ?? config?.audio.bufferSize ?? null;
  const streamBufferSize = status?.audioStreamBufferSize ?? null;
  const bufferMismatch =
    status?.audioBufferMismatch ??
    (activeBufferSize && requestedBufferSize ? activeBufferSize !== requestedBufferSize : null);
  const vstMidiCompatible = status?.vstMidiCompatible ?? null;

  const selectedVstPlugin = useMemo(
    () => vstPlugins.find((plugin) => plugin.path === (config?.audio.vstPath || "")) ?? null,
    [config?.audio.vstPath, vstPlugins]
  );

  const canOpenSelectedVstUi =
    Boolean(config?.audio.enabled) &&
    bridgeRunning &&
    Boolean(selectedVstPlugin?.supported) &&
    Boolean(selectedVstPlugin?.hasEditor);

  const canOpenVstParameterFallback =
    Boolean(config?.audio.enabled) &&
    bridgeRunning &&
    isVst3 &&
    Boolean(selectedVstPlugin?.supported);

  const currentLatencyMs = useMemo(() => {
    if (status?.audioLatencyMs !== undefined && status?.audioLatencyMs !== null) {
      return status.audioLatencyMs;
    }
    if (!activeBufferSize || !activeSampleRate) return null;
    return Number(((activeBufferSize / activeSampleRate) * 1000).toFixed(2));
  }, [status?.audioLatencyMs, activeBufferSize, activeSampleRate]);

  const updateAudioConfig = useCallback(
    async (patch: Partial<NonNullable<typeof config>["audio"]>) => {
      if (!config) return;
      const previousAudio = config.audio;
      const updated = { ...config, audio: { ...config.audio, ...patch } };
      const restartKeys: Array<keyof typeof patch> = [
        "backend",
        "device",
        "sampleRate",
        "bufferSize",
        "vstPath",
      ];
      const requiresRestart = restartKeys.some((key) => patch[key] !== undefined);
      const shouldRestart = requiresRestart && Boolean(status?.audioRunning) && updated.audio.enabled;
      if (shouldRestart && audioReloadInFlight.current) {
        return;
      }

      if (shouldRestart) {
        audioReloadInFlight.current = true;
        setAudioReloading(true);
      }
      try {
        await updateConfig({ audio: patch });
        if (!shouldRestart) {
          return;
        }
        await saveConfigApi(updated);
        await reloadVst();
        await refreshStatus();
      } catch (e) {
        await updateConfig({ audio: previousAudio });
        try {
          await saveConfigApi({ ...config, audio: previousAudio });
          await reloadVst();
          await refreshStatus();
        } catch (rollbackError) {
          console.error("Failed to restore previous audio runtime", rollbackError);
        }
        const message = getErrorMessage(e);
        toast.error(message || t("toasts.audio.vstReloadFailed"));
      } finally {
        if (shouldRestart) {
          audioReloadInFlight.current = false;
          setAudioReloading(false);
        }
      }
    },
    [config, refreshStatus, status?.audioRunning, t, updateConfig]
  );

  const loadCachedVstPlugins = useCallback(async () => {
    try {
      const plugins = await listVstPlugins();
      setVstPlugins(plugins);
    } catch (e) {
      console.error("Failed to load cached VST plugins", e);
    }
  }, []);

  const refreshVstPluginsList = useCallback(async () => {
    setVstPluginsLoading(true);
    try {
      const plugins = await refreshVstPlugins();
      setVstPlugins(plugins);
    } catch (e) {
      console.error("Failed to list VST plugins", e);
      toast.error(t("toasts.audio.vstListFailed"));
    } finally {
      setVstPluginsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    loadCachedVstPlugins();
  }, [loadCachedVstPlugins]);

  const handleBackendChange = async (value: string) => {
    await updateAudioConfig({ backend: value, device: null });
    await refreshAudioDevices(value);
  };

  const handleAudioToggle = async (checked: boolean) => {
    await updateConfig({ audio: { enabled: checked } });
    await persistConfig();
  };

  const handleGainChange = async (value: number) => {
    await updateConfig({ audio: { gainDb: value } });
    try {
      await setMasterGain(value);
    } catch (e) {
      console.error("Failed to set master gain", e);
    }
  };

  const handleLimiterToggle = async (checked: boolean) => {
    await updateConfig({ audio: { limiterEnabled: checked } });
    try {
      await setAudioLimiter(checked);
    } catch (e) {
      console.error("Failed to set limiter", e);
    }
  };

  const applyVstPath = useCallback(
    async (path: string) => {
      if (!config) return;
      setVstUiOpen(false);
      setVstParameterDialogOpen(false);
      setVstParams([]);
      const updated = { ...config, audio: { ...config.audio, vstPath: path } };
      await updateAudioConfig({ vstPath: path });
      try {
        await saveConfigApi(updated);
      } catch (e) {
        console.error("Failed to save VST path", e);
      }
    },
    [config, updateAudioConfig]
  );

  const handleSelectVst = async (path: string) => {
    if (!path || path === "__empty") return;
    const plugin = vstPlugins.find((entry) => entry.path === path);
    if (plugin && !plugin.supported) {
      toast.error(plugin.unsupportedReason || t("toasts.audio.vstUnsupported"));
      return;
    }
    await applyVstPath(path);
  };

  const handleOpenVstUi = async () => {
    try {
      await openVstUi();
      setVstUiOpen(true);
      toast.success(t("toasts.audio.vstOpened"));
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstOpenFailed"));
      setVstUiOpen(false);
    }
  };

  const handleCloseVstUi = async () => {
    try {
      await closeVstUi();
      setVstUiOpen(false);
      toast.success(t("toasts.audio.vstHidden"));
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstCloseFailed"));
    }
  };

  const handleOpenVstParameters = async () => {
    try {
      setVstParamsLoading(true);
      const params = await listVstParameters();
      setVstParams(params);
      setVstParameterDialogOpen(true);
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstParamOpenFailed"));
      setVstParameterDialogOpen(false);
    } finally {
      setVstParamsLoading(false);
    }
  };

  const handlePingAudio = async () => {
    try {
      await pingAudio();
      toast.success(t("toasts.audio.pingSent"));
      refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.pingFailed"));
    }
  };

  const handleReloadVst = async () => {
    if (audioReloadInFlight.current) return;
    audioReloadInFlight.current = true;
    setAudioReloading(true);
    try {
      await reloadVst();
      toast.success(t("toasts.audio.vstReloaded"));
      refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstReloadFailed"));
    } finally {
      audioReloadInFlight.current = false;
      setAudioReloading(false);
    }
  };

  const handleVstParamChange = async (index: number, value: number) => {
    setVstParams((prev) => prev.map((param) => (param.index === index ? { ...param, value } : param)));
    try {
      await setVstParameter(index, value);
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstParamFailed"));
    }
  };

  const selectedPluginStatusLabel = selectedVstPlugin?.supported
    ? t("audio.vstStatusCompatible")
    : selectedVstPlugin?.unsupportedReason || t("audio.vstStatusUnsupported");
  const selectedPluginKindLabel =
    selectedVstPlugin?.kind === "instrument"
      ? t("audio.vstKindInstrument")
      : selectedVstPlugin?.kind === "effect"
      ? t("audio.vstKindEffect")
      : t("audio.vstKindOther");

  return {
    activeBufferSize,
    activeSampleRate,
    audioBackends,
    audioDevices,
    audioReloading,
    bridgeRunning,
    bufferMismatch,
    canOpenSelectedVstUi,
    canOpenVstParameterFallback,
    config,
    currentLatencyMs,
    isVst3,
    midiMessagesPerSec: metrics?.midiMessagesPerSec ?? null,
    requestedBufferSize,
    selectedPluginKindLabel,
    selectedPluginStatusLabel,
    selectedVstPlugin,
    status,
    streamBufferSize,
    t,
    vstMidiCompatible,
    vstParameterDialogOpen,
    vstParams,
    vstParamsLoading,
    vstPlugins,
    vstPluginsLoading,
    vstUiOpen,
    handleAudioToggle,
    handleBackendChange,
    handleCloseVstUi,
    handleGainChange,
    handleLimiterToggle,
    handleOpenVstParameters,
    handleOpenVstUi,
    handlePingAudio,
    handleReloadVst,
    refreshVstPluginsList,
    persistConfig,
    handleSelectVst,
    updateConfig,
    updateAudioConfig,
    setVstParameterDialogOpen,
    handleVstParamChange,
  };
}
