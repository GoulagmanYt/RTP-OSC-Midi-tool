import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/ui/Card";
import { Label } from "../components/ui/Label";
import { Switch } from "../components/ui/Switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/Select";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Power, RefreshCcw, X } from "lucide-react";
import {
  closeVstUi,
  listAudioDevices,
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
  VstParameter,
  VstPluginEntry,
} from "../api";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { useI18n } from "../providers/LanguageProvider";

export default function AudioPage() {
  const { config, status, updateConfig, saveConfig: persistConfig, audioBackends, audioDevices, refreshAudioDevices, refreshStatus } =
    useBridge();
  const { t } = useI18n();

  // FIX: Track VST UI open state and sync it via the "vst_editor_hidden" Tauri event
  // emitted by the WM_CLOSE handler when the user closes the VST window via its X button.
  const [vstUiOpen, setVstUiOpen] = useState(false);
  const [vstParameterDialogOpen, setVstParameterDialogOpen] = useState(false);
  const [vstPlugins, setVstPlugins] = useState<VstPluginEntry[]>([]);
  const [vstPluginsLoading, setVstPluginsLoading] = useState(false);
  const [vstParams, setVstParams] = useState<VstParameter[]>([]);
  const [vstParamsLoading, setVstParamsLoading] = useState(false);

  // Subscribe to the backend "vst_editor_hidden" event so the button label stays
  // in sync when the user closes the VST window via its own title bar X button.
  useEffect(() => {
    const unlisten = listen("vst_editor_hidden", () => {
      setVstUiOpen(false);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const bridgeRunning = Boolean(status?.running);
  const isVst3 = (config?.vstPath || "").toLowerCase().endsWith(".vst3");

  const quickPresets = useMemo(
    () => [
      {
        id: "low",
        label: t("audio.preset.low"),
        backend: "asio",
        bufferSize: 64,
        sampleRate: 48000,
        hint: t("audio.preset.hint.low"),
      },
      {
        id: "balanced",
        label: t("audio.preset.balanced"),
        backend: "asio",
        bufferSize: 256,
        sampleRate: 48000,
        hint: t("audio.preset.hint.balanced"),
      },
      {
        id: "safe",
        label: t("audio.preset.safe"),
        backend: "wasapi",
        bufferSize: 512,
        sampleRate: 44100,
        hint: t("audio.preset.hint.safe"),
      },
    ],
    [t]
  );

  const activeSampleRate = status?.audioSampleRate ?? config?.audioSampleRate ?? null;
  const activeBufferSize = status?.audioBufferSize ?? config?.audioBufferSize ?? null;
  const requestedBufferSize = status?.audioRequestedBufferSize ?? config?.audioBufferSize ?? null;
  const streamBufferSize = status?.audioStreamBufferSize ?? null;
  const bufferMismatch =
    status?.audioBufferMismatch ??
    (activeBufferSize && requestedBufferSize ? activeBufferSize !== requestedBufferSize : null);
  const vstMidiCompatible = status?.vstMidiCompatible ?? null;

  const selectedVstPlugin = useMemo(
    () => vstPlugins.find((plugin) => plugin.path === (config?.vstPath || "")) ?? null,
    [config?.vstPath, vstPlugins]
  );

  const canOpenSelectedVstUi =
    Boolean(config?.audioEnabled) &&
    bridgeRunning &&
    Boolean(selectedVstPlugin?.supported) &&
    Boolean(selectedVstPlugin?.hasEditor);

  const canOpenVstParameterFallback =
    Boolean(config?.audioEnabled) &&
    bridgeRunning &&
    isVst3 &&
    Boolean(selectedVstPlugin?.supported);

  const currentLatencyMs = useMemo(() => {
    if (status?.audioLatencyMs !== undefined && status?.audioLatencyMs !== null) {
      return status.audioLatencyMs;
    }
    if (!activeBufferSize || !activeSampleRate) return null;
    return Number(((activeBufferSize / activeSampleRate) * 1000 * 2).toFixed(2));
  }, [status?.audioLatencyMs, activeBufferSize, activeSampleRate]);

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

  const handleBackendChange = async (val: string) => {
    await updateConfig({ audioBackend: val, audioDevice: null });
    await refreshAudioDevices(val);
  };

  const handleAudioToggle = async (checked: boolean) => {
    await updateConfig({ audioEnabled: checked });
    await persistConfig();
  };

  const applyPreset = async (presetId: string) => {
    const preset = quickPresets.find((p) => p.id === presetId);
    if (!preset) return;
    try {
      const devices = await listAudioDevices(preset.backend);
      const chosen =
        devices.find((d) => d === config?.audioDevice) ||
        devices[0] ||
        null;
      await updateConfig({
        audioBackend: preset.backend,
        audioDevice: chosen,
        audioBufferSize: preset.bufferSize,
        audioSampleRate: preset.sampleRate,
      });
      await refreshAudioDevices(preset.backend);
      toast.success(t("toasts.audio.presetApplied", { preset: preset.label }));
    } catch (e) {
      console.error("Failed to apply preset", e);
      toast.error(t("toasts.audio.presetFailed"));
    }
  };

  const handleGainChange = async (value: number) => {
    await updateConfig({ audioGainDb: value });
    try {
      await setMasterGain(value);
    } catch (e) {
      console.error("Failed to set master gain", e);
    }
  };

  const handleLimiterToggle = async (checked: boolean) => {
    await updateConfig({ audioLimiterEnabled: checked });
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
      const updated = { ...config, vstPath: path };
      await updateConfig({ vstPath: path });
      try {
        await saveConfigApi(updated);
      } catch (e) {
        console.error("Failed to save VST path", e);
      }
      if (status?.audioRunning && updated.audioEnabled) {
        try {
          await reloadVst();
          await refreshStatus();
        } catch (e) {
          const message = e instanceof Error ? e.message : String(e);
          toast.error(message || t("toasts.audio.vstReloadFailed"));
        }
      }
    },
    [config, refreshStatus, status?.audioRunning, t, updateConfig]
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
      const message = e instanceof Error ? e.message : String(e);
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
      const message = e instanceof Error ? e.message : String(e);
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
      const message = e instanceof Error ? e.message : String(e);
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
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.audio.pingFailed"));
    }
  };

  const handleReloadVst = async () => {
    try {
      await reloadVst();
      toast.success(t("toasts.audio.vstReloaded"));
      refreshStatus();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.audio.vstReloadFailed"));
    }
  };

  const handleVstParamChange = async (index: number, value: number) => {
    setVstParams((prev) =>
      prev.map((param) => (param.index === index ? { ...param, value } : param))
    );
    try {
      await setVstParameter(index, value);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
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

  return (
    <div className="space-y-6">
        <Card>
            <CardHeader>
                <CardTitle>{t("audio.title")}</CardTitle>
                <CardDescription>{t("audio.description")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">

                <div className="flex items-center justify-between">
                    <Label className="flex flex-col gap-1">
                        <span>{t("audio.enableEngine")}</span>
                        <span className="font-normal text-muted-foreground text-xs">{t("audio.enableEngineHint")}</span>
                    </Label>
                    <Switch checked={config?.audioEnabled} onCheckedChange={handleAudioToggle} />
                </div>

                <div className="space-y-2">
                  <Label>{t("audio.quickPresets")}</Label>
                  <div className="grid gap-2 md:grid-cols-3">
                    {quickPresets.map((preset) => {
                      const isActive =
                        (config?.audioBackend || "").toLowerCase().includes(preset.backend) &&
                        config?.audioBufferSize === preset.bufferSize &&
                        config?.audioSampleRate === preset.sampleRate;
                      const latency = Number(((preset.bufferSize / preset.sampleRate) * 1000 * 2).toFixed(1));
                      return (
                        <Button
                          key={preset.id}
                          variant={isActive ? "default" : "outline"}
                          className="flex flex-col items-start gap-1 h-full"
                          onClick={() => applyPreset(preset.id)}
                        >
                          <div className="flex w-full items-center justify-between">
                            <span className="font-semibold">{preset.label}</span>
                            <Badge variant="secondary">
                              {latency} {t("units.ms")}
                            </Badge>
                          </div>
                          <span className="text-xs text-muted-foreground">
                            {t("audio.preset.hintWithBackend", {
                              hint: preset.hint,
                              backend: preset.backend.toUpperCase(),
                            })}
                          </span>
                        </Button>
                      );
                    })}
                  </div>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                    <div className="space-y-2">
                        <Label>{t("audio.backendLabel")}</Label>
                        <Select value={config?.audioBackend || ""} onValueChange={handleBackendChange}>
                            <SelectTrigger>
                                <SelectValue placeholder={t("audio.backendPlaceholder")} />
                            </SelectTrigger>
                            <SelectContent>
                                {audioBackends.map(b => <SelectItem key={b} value={b}>{b}</SelectItem>)}
                            </SelectContent>
                        </Select>
                    </div>

                    <div className="space-y-2">
                        <Label>{t("audio.deviceLabel")}</Label>
                        <Select
                            value={config?.audioDevice || ""}
                            onValueChange={(v) => updateConfig({ audioDevice: v })}
                            disabled={!config?.audioBackend}
                        >
                            <SelectTrigger>
                                <SelectValue placeholder={t("audio.devicePlaceholder")} />
                            </SelectTrigger>
                            <SelectContent>
                                {audioDevices.map(d => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                            </SelectContent>
                        </Select>
                    </div>
                </div>

                <div className="grid gap-4 md:grid-cols-3">
                     <div className="space-y-2">
                        <Label>{t("audio.sampleRate")}</Label>
                         <Select
                            value={config?.audioSampleRate?.toString()}
                            onValueChange={(v) => updateConfig({ audioSampleRate: parseInt(v) })}
                        >
                            <SelectTrigger><SelectValue /></SelectTrigger>
                            <SelectContent>
                                {[44100, 48000, 88200, 96000].map(r => (
                                  <SelectItem key={r} value={r.toString()}>
                                    {r} {t("units.hz")}
                                  </SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                     </div>
                     <div className="space-y-2">
                        <Label>{t("audio.bufferSize")}</Label>
                         <Select
                            value={config?.audioBufferSize?.toString()}
                            onValueChange={(v) => updateConfig({ audioBufferSize: parseInt(v) })}
                        >
                            <SelectTrigger><SelectValue /></SelectTrigger>
                            <SelectContent>
                                {[64, 128, 256, 480, 512, 1024, 2048].map(r => (
                                  <SelectItem key={r} value={r.toString()}>
                                    {r} {t("units.samples")}
                                  </SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                     </div>
                     <div className="space-y-2">
                         <Label>
                          {t("audio.masterGain", {
                            value: config?.audioGainDb || 0,
                            unit: t("units.db"),
                          })}
                         </Label>
                         <Input
                            type="range"
                            min={-60}
                            max={12}
                            step={1}
                            value={config?.audioGainDb || 0}
                            onChange={(e) => handleGainChange(Number(e.target.value))}
                            className="w-full"
                         />
                     </div>
                </div>

                <div className="flex items-center justify-between">
                  <Label className="flex flex-col gap-1">
                    <span>{t("audio.limiter")}</span>
                    <span className="font-normal text-muted-foreground text-xs">{t("audio.limiterHint")}</span>
                  </Label>
                  <Switch checked={config?.audioLimiterEnabled} onCheckedChange={handleLimiterToggle} />
                </div>

                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.estimatedLatency")}</span>
                  <span className="text-foreground font-semibold">
                    {currentLatencyMs !== null ? `${currentLatencyMs} ${t("units.ms")}` : "--"}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.activeLabel")}</span>
                  <span className="text-foreground font-semibold">
                    {activeSampleRate ? `${activeSampleRate} ${t("units.hz")}` : "--"} /{" "}
                    {activeBufferSize ? `${activeBufferSize} ${t("units.samples")}` : t("common.auto")}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>Buffer demandé</span>
                  <span className="text-foreground font-semibold">
                    {requestedBufferSize ? `${requestedBufferSize} ${t("units.samples")}` : "--"}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>Buffer stream</span>
                  <span className="text-foreground font-semibold">
                    {streamBufferSize ? `${streamBufferSize} ${t("units.samples")}` : t("common.auto")}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>Compat MIDI VST</span>
                  <span className="text-foreground font-semibold">
                    {vstMidiCompatible === null
                      ? "--"
                      : vstMidiCompatible
                      ? "OK"
                      : "Non compatible"}
                  </span>
                </div>
                {bufferMismatch && (
                  <p className="text-xs text-amber-600">
                    Le driver audio impose une taille de buffer différente de la valeur demandée.
                  </p>
                )}
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.activeBackend")}</span>
                  <span className="text-foreground font-semibold">{status?.audioBackend ?? "--"}</span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.activeDevice")}</span>
                  <span className="text-foreground font-semibold">{status?.audioDevice ?? "--"}</span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.xruns")}</span>
                  <span className="text-foreground font-semibold">{status?.audioXruns ?? "--"}</span>
                </div>

                <div className="space-y-2">
                  <Label>
                    {t("audio.vstInstruments", { path: "Windows VST folders" })}
                  </Label>
                  <div className="flex flex-col gap-2 md:flex-row md:items-center md:flex-wrap">
                    <Select value={config?.vstPath || ""} onValueChange={handleSelectVst}>
                      <SelectTrigger className="md:flex-1">
                        <SelectValue placeholder={t("audio.vstSelectPlaceholder")} />
                      </SelectTrigger>
                      <SelectContent>
                        {vstPlugins.length === 0 && (
                          <SelectItem value="__empty" disabled>
                            {t("audio.vstNoCached")}
                          </SelectItem>
                        )}
                        {vstPlugins.map((plugin) => (
                          <SelectItem
                            key={plugin.path}
                            value={plugin.path}
                            disabled={!plugin.supported}
                          >
                            {`${plugin.name} - ${plugin.format} - ${plugin.architecture} - ${
                              plugin.supported
                                ? t("audio.vstStatusCompatible")
                                : plugin.unsupportedReason || t("audio.vstStatusUnsupported")
                            }`}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Button variant="outline" onClick={refreshVstPluginsList} disabled={vstPluginsLoading}>
                      {vstPluginsLoading ? t("common.loading") : t("common.refresh")}
                    </Button>
                  </div>
                  {selectedVstPlugin && (
                    <div className="rounded-md border border-border/60 bg-muted/30 p-3 space-y-2">
                      <div className="flex flex-wrap gap-2">
                        <Badge variant="secondary">{selectedVstPlugin.format}</Badge>
                        <Badge variant="outline">{selectedVstPlugin.architecture}</Badge>
                        <Badge variant={selectedVstPlugin.supported ? "success" : "warning"}>
                          {selectedVstPlugin.supported
                            ? t("audio.vstStatusCompatible")
                            : t("audio.vstStatusUnsupported")}
                        </Badge>
                        <Badge
                          variant={
                            selectedVstPlugin.kind === "instrument" ? "success" : "warning"
                          }
                        >
                          {selectedPluginKindLabel}
                        </Badge>
                        <Badge
                          variant={selectedVstPlugin.hasEditor ? "secondary" : "outline"}
                        >
                          {selectedVstPlugin.hasEditor
                            ? t("audio.vstEditorAvailable")
                            : t("audio.vstNoEditor")}
                        </Badge>
                        {selectedVstPlugin.midiCompatible !== null && (
                          <Badge
                            variant={
                              selectedVstPlugin.midiCompatible ? "success" : "warning"
                            }
                          >
                            {selectedVstPlugin.midiCompatible
                              ? t("audio.vstMidiCompatibleBadge")
                              : t("audio.vstMidiIncompatibleBadge")}
                          </Badge>
                        )}
                      </div>
                      <p className="text-xs text-muted-foreground break-all">
                        {selectedVstPlugin.path}
                      </p>
                      {selectedVstPlugin.channelLayout && (
                        <p className="text-xs text-muted-foreground">
                          {t("audio.vstChannelLayout", {
                            layout: selectedVstPlugin.channelLayout,
                          })}
                        </p>
                      )}
                      {!selectedVstPlugin.supported && (
                        <p className="text-xs text-amber-600">{selectedPluginStatusLabel}</p>
                      )}
                    </div>
                  )}
                  <div className="flex flex-wrap gap-2">
                    <Button
                      variant="outline"
                      onClick={handleOpenVstUi}
                      disabled={vstUiOpen || !canOpenSelectedVstUi}
                    >
                      {t("audio.openVstUi")}
                    </Button>
                    {vstUiOpen && (
                      <Button variant="outline" onClick={handleCloseVstUi}>
                        <X className="h-4 w-4 mr-2" />
                        {t("audio.hideUi")}
                      </Button>
                    )}
                    {isVst3 && (
                      <Button
                        variant="outline"
                        onClick={handleOpenVstParameters}
                        disabled={!canOpenVstParameterFallback}
                      >
                        {t("audio.openVstParameters")}
                      </Button>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {t("audio.vstSettingsSaved")}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t("audio.vstMidiOnly")}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t("audio.vstFormats")}
                  </p>
                  {isVst3 && (
                    <p className="text-xs text-muted-foreground">
                      {t("audio.vstParameterFallback")}
                    </p>
                  )}
                  {!bridgeRunning && (
                    <p className="text-xs text-amber-600">
                      {t("audio.startBridgeHint")}
                    </p>
                  )}
                  <div className="flex flex-wrap gap-2 pt-2">
                    <Button variant="outline" onClick={handlePingAudio} disabled={!bridgeRunning || !config?.audioEnabled}>
                      <Power className="w-4 h-4 mr-2" />
                      {t("audio.pingAudio")}
                    </Button>
                    <Button variant="outline" onClick={handleReloadVst} disabled={!config?.audioEnabled}>
                      <RefreshCcw className="w-4 h-4 mr-2" />
                      {t("audio.reloadVst")}
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {t("audio.pingHint")}
                  </p>
                </div>

                <div className="pt-4 flex justify-end gap-2">
                    <Button variant="outline" onClick={() => persistConfig()}>
                      {t("audio.saveConfiguration")}
                    </Button>
                </div>

            </CardContent>
        </Card>

      <Dialog
        open={vstParameterDialogOpen && isVst3}
        onOpenChange={(open) => {
          if (!open) setVstParameterDialogOpen(false);
        }}
      >
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("audio.vst3Parameters")}</DialogTitle>
            <DialogDescription>
              {t("audio.vst3Description")}
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-[60vh] space-y-4 overflow-y-auto pr-2">
            {vstParamsLoading && (
              <p className="text-sm text-muted-foreground">{t("audio.loadingParameters")}</p>
            )}
            {!vstParamsLoading && vstParams.length === 0 && (
              <p className="text-sm text-muted-foreground">{t("audio.noParameters")}</p>
            )}
            {!vstParamsLoading &&
              vstParams.map((param) => {
                const range = param.max - param.min;
                const displayValue = range !== 0 ? param.min + param.value * range : param.value;
                return (
                  <div key={param.index} className="space-y-1">
                    <div className="flex items-center justify-between text-sm">
                      <span className="font-medium">{param.name}</span>
                      <span className="text-muted-foreground">
                        {displayValue.toFixed(3)} {param.unit}
                      </span>
                    </div>
                    <Input
                      type="range"
                      min={0}
                      max={1}
                      step={0.001}
                      value={param.value}
                      onChange={(e) => handleVstParamChange(param.index, Number(e.target.value))}
                      className="w-full"
                    />
                  </div>
                );
              })}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
