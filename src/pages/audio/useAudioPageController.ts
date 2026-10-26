import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  closeVstUi,
  listVstParameters,
  listVstPlugins,
  openVstUi,
  openVstFolder,
  pingAudio,
  refreshVstPlugins,
  retestVstPlugin,
  reloadVst,
  saveConfig as saveConfigApi,
  setAudioLimiter,
  setMasterGain,
  setVstEq,
  setVstParameter,
} from "../../api";
import type { VstEqSettings, VstParameter, VstPluginEntry } from "../../api";
import { useI18n } from "../../providers/LanguageProvider";
import { useBridge } from "../../providers/BridgeProvider";
import { mergeLiveAudioMetrics } from "./liveStatus";
import { getAudioLatency } from "./latency";
import { getErrorMessage } from "../../api-errors";
import { BUNDLED_VST_EQ_KEY, createDefaultVstEqSettings, getVstEqKey } from "./eq";

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
  const actualEditorOpen = status?.realtime?.worker.editorOpen;

  const [vstParameterDialogOpen, setVstParameterDialogOpen] = useState(false);
  const [vstEqDialogOpen, setVstEqDialogOpen] = useState(false);
  const [vstPlugins, setVstPlugins] = useState<VstPluginEntry[]>([]);
  const [vstPluginsLoading, setVstPluginsLoading] = useState(false);
  const [vstParams, setVstParams] = useState<VstParameter[]>([]);
  const [vstParamsLoading, setVstParamsLoading] = useState(false);
  const [audioReloading, setAudioReloading] = useState(false);
  const audioReloadInFlight = useRef(false);
  const pendingVstEq = useRef<VstEqSettings | null>(null);
  const vstEqFrame = useRef<number | null>(null);
  const vstEqSendInFlight = useRef(false);
  const pendingVstParams = useRef<Map<number, number>>(new Map());
  const vstParamFrame = useRef<number | null>(null);
  const vstEqControllerMounted = useRef(true);
  const flushVstEqRef = useRef<() => void>(() => undefined);
  const flushVstParamsRef = useRef<() => void>(() => undefined);
  const activePluginIdRef = useRef<string | null>(config?.audio.vstPluginId ?? config?.audio.vstPath ?? null);
  const parameterRequestId = useRef(0);
  const vstPluginsRequestId = useRef(0);

  const invalidateVstParameters = useCallback(() => {
    parameterRequestId.current += 1;
    setVstParamsLoading(false);
    setVstParameterDialogOpen(false);
    setVstParams([]);
    pendingVstParams.current.clear();
    if (vstParamFrame.current !== null) {
      window.cancelAnimationFrame(vstParamFrame.current);
      vstParamFrame.current = null;
    }
  }, []);

  useEffect(() => {
    activePluginIdRef.current = config?.audio.vstPluginId ?? config?.audio.vstPath ?? null;
    invalidateVstParameters();
  }, [config?.audio.vstPluginId, config?.audio.vstPath, status?.audioRunning,
    status?.vstWorkerState, status?.vstWorkerRestarts, invalidateVstParameters]);

  useEffect(() => {
    let disposed = false;
    const unlisten = listen("audio:vst-editor-hidden", () => {
      if (!disposed) {
        setVstUiOpen(false);
      }
    });
    return () => {
      disposed = true;
      unlisten.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  const bridgeRunning = Boolean(status?.running);

  const activeSampleRate = status?.audioSampleRate ?? config?.audio.sampleRate ?? null;
  const activeBufferSize = status?.audioBufferSize ?? config?.audio.bufferSize ?? null;
  const requestedBufferSize = status?.audioRequestedBufferSize ?? config?.audio.bufferSize ?? null;
  const streamBufferSize = status?.audioStreamBufferSize ?? null;
  const bufferMismatch =
    status?.audioBufferMismatch ??
    (activeBufferSize && requestedBufferSize ? activeBufferSize !== requestedBufferSize : null);
  const vstMidiCompatible = status?.vstMidiCompatible ?? null;

  const selectedVstPlugin = useMemo(
    () =>
      vstPlugins.find((plugin) => plugin.id === config?.audio.vstPluginId) ??
      vstPlugins.find((plugin) => plugin.path === (config?.audio.vstPath || "")) ??
      null,
    [config?.audio, vstPlugins]
  );
  const selectedVstEqKey = config ? getVstEqKey(config.audio, selectedVstPlugin) : BUNDLED_VST_EQ_KEY;
  const selectedVstEq =
    config?.audio.vstEqByPlugin?.[selectedVstEqKey] ?? createDefaultVstEqSettings();
  const selectedVstEqName = selectedVstPlugin?.name ?? t("dashboard.bundledVstDefault");

  const flushVstEq = useCallback(() => {
    vstEqFrame.current = null;
    if (vstEqSendInFlight.current || !pendingVstEq.current) return;
    const next = pendingVstEq.current;
    pendingVstEq.current = null;
    vstEqSendInFlight.current = true;
    void setVstEq(next)
      .catch((error) => {
        console.error("Failed to set VST EQ", error);
        toast.error(getErrorMessage(error) || t("toasts.audio.eqFailed"));
      })
      .finally(() => {
        vstEqSendInFlight.current = false;
        if (vstEqControllerMounted.current && pendingVstEq.current && vstEqFrame.current === null) {
          vstEqFrame.current = window.requestAnimationFrame(() => flushVstEqRef.current());
        }
      });
  }, [t]);

  useEffect(() => {
    flushVstEqRef.current = flushVstEq;
  }, [flushVstEq]);

  const queueVstEq = useCallback(
    (settings: VstEqSettings) => {
      pendingVstEq.current = settings;
      if (!vstEqSendInFlight.current && vstEqFrame.current === null) {
        vstEqFrame.current = window.requestAnimationFrame(() => flushVstEqRef.current());
      }
    },
    []
  );

  useEffect(() => {
    vstEqControllerMounted.current = true;
    return () => {
      vstEqControllerMounted.current = false;
      vstPluginsRequestId.current += 1;
      parameterRequestId.current += 1;
      if (vstEqFrame.current !== null) {
        window.cancelAnimationFrame(vstEqFrame.current);
        vstEqFrame.current = null;
      }
      if (vstParamFrame.current !== null) {
        window.cancelAnimationFrame(vstParamFrame.current);
        vstParamFrame.current = null;
      }
      flushVstParamsRef.current();
    };
  }, []);

  const canOpenSelectedVstUi =
    !audioReloading &&
    Boolean(status?.audioRunning) &&
    Boolean(config?.audio.enabled) &&
    bridgeRunning &&
    Boolean(selectedVstPlugin?.supported) &&
    Boolean(selectedVstPlugin?.hasEditor);

  const canOpenVstParameterFallback =
    !audioReloading &&
    Boolean(status?.audioRunning) &&
    Boolean(config?.audio.enabled) &&
    bridgeRunning &&
    Boolean(selectedVstPlugin?.supported);

  const latency = getAudioLatency(status, config);
  const currentLatencyMs = latency.ms;

  const updateAudioConfig = useCallback(
    async (patch: Partial<NonNullable<typeof config>["audio"]>): Promise<boolean> => {
      if (!config || audioReloadInFlight.current) return false;
      const previousAudio = config.audio;
      const updated = { ...config, audio: { ...config.audio, ...patch } };
      const restartKeys: Array<keyof typeof patch> = [
        "backend",
        "device",
        "sampleRate",
        "bufferSize",
        "renderAheadBlocks",
        "dspThreadPriority",
        "vstPluginId",
        "vstPath",
      ];
      const requiresRestart = restartKeys.some((key) => patch[key] !== undefined);
      const shouldRestart = requiresRestart && Boolean(status?.audioRunning) && updated.audio.enabled;
      const persistImmediately = requiresRestart || patch.gameMode !== undefined;
      let reloadAttempted = false;
      audioReloadInFlight.current = true;
      if (persistImmediately) setAudioReloading(true);
      try {
        await updateConfig({ audio: patch });
        if (persistImmediately) await saveConfigApi(updated);
        if (shouldRestart) {
          reloadAttempted = true;
          invalidateVstParameters();
          await reloadVst();
        }
        if (patch.gameMode) setVstUiOpen(false);
      } catch (e) {
        try {
          await updateConfig({ audio: previousAudio });
          if (persistImmediately) await saveConfigApi(config);
          if (reloadAttempted) await reloadVst();
        } catch (rollbackError) {
          console.error("Failed to restore previous audio runtime", rollbackError);
        }
        const message = getErrorMessage(e);
        toast.error(message || t("toasts.audio.vstReloadFailed"));
        return false;
      } finally {
        audioReloadInFlight.current = false;
        if (persistImmediately) setAudioReloading(false);
      }
      // A failed status query must not undo a configuration already accepted by the runtime.
      if (persistImmediately) {
        try {
          await refreshStatus();
        } catch (error) {
          console.error("Failed to refresh audio status", error);
        }
      }
      return true;
    },
    [config, refreshStatus, status?.audioRunning, t, updateConfig, invalidateVstParameters]
  );

  const loadCachedVstPlugins = useCallback(async () => {
    const requestId = ++vstPluginsRequestId.current;
    try {
      const plugins = await listVstPlugins();
      if (vstEqControllerMounted.current && requestId === vstPluginsRequestId.current) {
        setVstPlugins(plugins);
      }
    } catch (e) {
      console.error("Failed to load cached VST plugins", e);
    }
  }, []);

  const refreshVstPluginsList = useCallback(async () => {
    const requestId = ++vstPluginsRequestId.current;
    setVstPluginsLoading(true);
    try {
      const plugins = await refreshVstPlugins();
      if (vstEqControllerMounted.current && requestId === vstPluginsRequestId.current) {
        setVstPlugins(plugins);
      }
    } catch (e) {
      if (!vstEqControllerMounted.current || requestId !== vstPluginsRequestId.current) return;
      console.error("Failed to list VST plugins", e);
      toast.error(t("toasts.audio.vstListFailed"));
    } finally {
      if (vstEqControllerMounted.current && requestId === vstPluginsRequestId.current) {
        setVstPluginsLoading(false);
      }
    }
  }, [t]);

  useEffect(() => {
    loadCachedVstPlugins();
  }, [loadCachedVstPlugins]);

  const handleBackendChange = async (value: string) => {
    if (await updateAudioConfig({ backend: value, device: null, deviceId: null })) {
      await refreshAudioDevices(value);
    }
  };

  const handleAudioToggle = async (checked: boolean) => {
    if (!config || audioReloadInFlight.current) return;
    audioReloadInFlight.current = true;
    setAudioReloading(true);
    try {
      const updated = { ...config, audio: { ...config.audio, enabled: checked } };
      await updateConfig({ audio: { enabled: checked } });
      await saveConfigApi(updated);
      await refreshStatus();
    } catch (error) {
      await updateConfig({ audio: { enabled: config.audio.enabled } });
      toast.error(getErrorMessage(error) || t("toasts.settings.configSaveFailed"));
    } finally {
      audioReloadInFlight.current = false;
      setAudioReloading(false);
    }
  };

  const handleGainChange = async (value: number) => {
    try {
      await updateConfig({ audio: { gainDb: value } });
      if (status?.audioRunning) {
        await setMasterGain(value);
      }
    } catch (e) {
      console.error("Failed to set master gain", e);
      toast.error(getErrorMessage(e) || t("toasts.settings.configSaveFailed"));
    }
  };

  const handleLimiterToggle = async (checked: boolean) => {
    try {
      await updateConfig({ audio: { limiterEnabled: checked } });
      if (status?.audioRunning) {
        await setAudioLimiter(checked);
      }
    } catch (e) {
      console.error("Failed to set limiter", e);
      toast.error(getErrorMessage(e) || t("toasts.settings.configSaveFailed"));
    }
  };

  const applyVstPlugin = useCallback(
    async (plugin: VstPluginEntry) => {
      if (!config) return;
      activePluginIdRef.current = plugin.id;
      invalidateVstParameters();
      setVstUiOpen(false);
      setVstEqDialogOpen(false);
      pendingVstEq.current = null;
      if (vstEqFrame.current !== null) {
        window.cancelAnimationFrame(vstEqFrame.current);
        vstEqFrame.current = null;
      }
      await updateAudioConfig({ vstPluginId: plugin.id, vstPath: plugin.path });
    },
    [config, updateAudioConfig, invalidateVstParameters]
  );

  const handleSelectVst = async (pluginId: string) => {
    if (!pluginId || pluginId === "__empty") return;
    const plugin = vstPlugins.find((entry) => entry.id === pluginId);
    if (!plugin) return;
    if (plugin && !plugin.supported) {
      toast.error(plugin.unsupportedReason || t("toasts.audio.vstUnsupported"));
      return;
    }
    await applyVstPlugin(plugin);
  };

  const handleRetestVst = async (pluginId: string) => {
    const requestId = ++vstPluginsRequestId.current;
    setVstPluginsLoading(true);
    try {
      const tested = await retestVstPlugin(pluginId);
      if (!vstEqControllerMounted.current || requestId !== vstPluginsRequestId.current) return;
      setVstPlugins((entries) => entries.map((entry) => (entry.id === pluginId ? tested : entry)));
    } catch (e) {
      if (!vstEqControllerMounted.current || requestId !== vstPluginsRequestId.current) return;
      toast.error(getErrorMessage(e) || t("toasts.audio.vstListFailed"));
    } finally {
      if (vstEqControllerMounted.current && requestId === vstPluginsRequestId.current) setVstPluginsLoading(false);
    }
  };

  const handleOpenVstFolder = async (pluginId: string) => {
    try {
      await openVstFolder(pluginId);
    } catch (e) {
      toast.error(getErrorMessage(e));
    }
  };

  const handleOpenVstUi = async () => {
    try {
      await openVstUi();
      setVstUiOpen(!(config?.audio.gameMode ?? true));
      toast.success(t((config?.audio.gameMode ?? true) ? "audio.deferredOperations" : "toasts.audio.vstOpened"));
      await refreshStatus();
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
    if (audioReloadInFlight.current || !status?.audioRunning) return;
    const requestedPluginId = config?.audio.vstPluginId ?? config?.audio.vstPath ?? null;
    const requestId = ++parameterRequestId.current;
    try {
      setVstParamsLoading(true);
      const params = await listVstParameters();
      if (!vstEqControllerMounted.current || requestId !== parameterRequestId.current || activePluginIdRef.current !== requestedPluginId) {
        return;
      }
      setVstParams(params);
      setVstParameterDialogOpen(true);
    } catch (e) {
      if (!vstEqControllerMounted.current || requestId !== parameterRequestId.current) return;
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstParamOpenFailed"));
      setVstParameterDialogOpen(false);
    } finally {
      if (vstEqControllerMounted.current && requestId === parameterRequestId.current) setVstParamsLoading(false);
    }
  };

  const handlePingAudio = async () => {
    try {
      await pingAudio();
      toast.success(t("toasts.audio.pingSent"));
      await refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.pingFailed"));
    }
  };

  const handleReloadVst = async () => {
    if (audioReloadInFlight.current) return;
    audioReloadInFlight.current = true;
    setAudioReloading(true);
    invalidateVstParameters();
    try {
      await reloadVst();
      toast.success(t("toasts.audio.vstReloaded"));
      await refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstReloadFailed"));
    } finally {
      audioReloadInFlight.current = false;
      setAudioReloading(false);
    }
  };

  const flushVstParams = useCallback(() => {
    vstParamFrame.current = null;
    if (pendingVstParams.current.size === 0) return;
    const entries = Array.from(pendingVstParams.current.entries());
    pendingVstParams.current.clear();
    for (const [idx, val] of entries) {
      setVstParameter(idx, val).catch((e) => {
        const message = getErrorMessage(e);
        toast.error(message || t("toasts.audio.vstParamFailed"));
      });
    }
  }, [t]);

  useEffect(() => {
    flushVstParamsRef.current = flushVstParams;
  }, [flushVstParams]);

  const handleVstParamChange = useCallback(async (index: number, value: number) => {
    setVstParams((prev) => prev.map((param) => (param.index === index ? { ...param, value } : param)));
    pendingVstParams.current.set(index, value);
    if (vstParamFrame.current === null) {
      vstParamFrame.current = window.requestAnimationFrame(flushVstParams);
    }
  }, [flushVstParams]);

  const handleVstEqChange = (settings: VstEqSettings) => {
    if (!config) return;
    void updateConfig({
      audio: {
        vstPluginId: config.audio.vstPluginId ?? selectedVstPlugin?.id,
        vstEqByPlugin: {
          ...config.audio.vstEqByPlugin,
          [selectedVstEqKey]: settings,
        },
      },
    });
    queueVstEq(settings);
  };

  const handleVstEqReset = () => {
    if (!config) return;
    const profiles = { ...config.audio.vstEqByPlugin };
    delete profiles[selectedVstEqKey];
    void updateConfig({
      audio: {
        vstPluginId: config.audio.vstPluginId ?? selectedVstPlugin?.id,
        vstEqByPlugin: profiles,
      },
    });
    queueVstEq(createDefaultVstEqSettings());
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
    latencyIsBufferPeriod: latency.source === "bufferPeriod",
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
    vstEqDialogOpen,
    selectedVstEq,
    selectedVstEqName,
    vstParams,
    vstParamsLoading,
    vstPlugins,
    vstPluginsLoading,
    vstUiOpen: actualEditorOpen ?? vstUiOpen,
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
    handleRetestVst,
    handleOpenVstFolder,
    updateConfig,
    updateAudioConfig,
    setVstParameterDialogOpen,
    setVstEqDialogOpen,
    handleVstParamChange,
    handleVstEqChange,
    handleVstEqReset,
  };
}
