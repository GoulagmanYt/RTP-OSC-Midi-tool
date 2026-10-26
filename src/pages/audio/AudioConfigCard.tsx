import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../components/ui/Select";
import { Switch } from "../../components/ui/Switch";
import type { AppConfig, AudioDeviceEntry, RuntimeStatus, VstPluginEntry } from "../../api-types";
import type { TranslateFn } from "./shared";
import { Power, RefreshCcw, SlidersHorizontal, X } from "lucide-react";
import { useMemo, useState } from "react";
import { isDeveloperMode } from "../../developer-mode";

type Props = {
  audioBackends: string[];
  audioDevices: AudioDeviceEntry[];
  audioReloading: boolean;
  bridgeRunning: boolean;
  bufferMismatch: boolean | null;
  canOpenSelectedVstUi: boolean;
  canOpenVstParameterFallback: boolean;
  canOpenVstEq: boolean;
  config: AppConfig | null;
  currentLatencyMs: number | null;
  latencyIsBufferPeriod?: boolean;
  midiMessagesPerSec: number | null;
  selectedPluginKindLabel: string;
  selectedPluginStatusLabel: string;
  selectedVstPlugin: VstPluginEntry | null;
  vstEqEnabled: boolean;
  status: RuntimeStatus | null;
  streamBufferSize: number | null;
  t: TranslateFn;
  vstMidiCompatible: boolean | null;
  vstPlugins: VstPluginEntry[];
  vstPluginsLoading: boolean;
  vstUiOpen: boolean;
  activeBufferSize: number | null;
  activeSampleRate: number | null;
  requestedBufferSize: number | null;
  onBackendChange: (value: string) => Promise<void>;
  onCloseVstUi: () => Promise<void>;
  onGainChange: (value: number) => Promise<void>;
  onLimiterToggle: (checked: boolean) => Promise<void>;
  onOpenVstParameters: () => Promise<void>;
  onOpenVstEq: () => void;
  onOpenVstUi: () => Promise<void>;
  onPingAudio: () => Promise<void>;
  onRefreshVstPluginsList: () => Promise<void>;
  onReloadVst: () => Promise<void>;
  onSaveConfig: () => Promise<void>;
  onSelectVst: (pluginId: string) => Promise<void>;
  onRetestVst: (pluginId: string) => Promise<void>;
  onOpenVstFolder: (pluginId: string) => Promise<void>;
  onToggleAudio: (checked: boolean) => Promise<void>;
  onUpdateAudioConfig: (patch: Partial<AppConfig["audio"]>) => Promise<boolean | void>;
};

export function AudioConfigCard({
  audioBackends,
  audioDevices,
  audioReloading,
  bridgeRunning,
  bufferMismatch,
  canOpenSelectedVstUi,
  canOpenVstParameterFallback,
  canOpenVstEq,
  config,
  currentLatencyMs,
  latencyIsBufferPeriod = false,
  midiMessagesPerSec,
  selectedPluginKindLabel,
  selectedPluginStatusLabel,
  selectedVstPlugin,
  vstEqEnabled,
  status,
  streamBufferSize,
  t,
  vstMidiCompatible,
  vstPlugins,
  vstPluginsLoading,
  vstUiOpen,
  activeBufferSize,
  activeSampleRate,
  requestedBufferSize,
  onBackendChange,
  onCloseVstUi,
  onGainChange,
  onLimiterToggle,
  onOpenVstParameters,
  onOpenVstEq,
  onOpenVstUi,
  onPingAudio,
  onRefreshVstPluginsList,
  onReloadVst,
  onSaveConfig,
  onSelectVst,
  onRetestVst,
  onOpenVstFolder,
  onToggleAudio,
  onUpdateAudioConfig,
}: Props) {
  const savedScanPaths = (config?.audio.vstScanPaths || []).join(";");
  const [scanPathsDraft, setScanPathsDraft] = useState<string | null>(null);
  const [vstSearch, setVstSearch] = useState("");
  const [vstFormat, setVstFormat] = useState("all");
  const [vstArchitecture, setVstArchitecture] = useState("all");
  const [vstStatus, setVstStatus] = useState("all");
  const [vstVendor, setVstVendor] = useState("all");
  const selectedAudioDeviceValue =
    audioDevices.find((device) => device.id && device.id === config?.audio.deviceId)?.id ||
    audioDevices.find((device) => device.name === config?.audio.device)?.id ||
    config?.audio.device ||
    "";
  const showVstTechnicalDetails = isDeveloperMode(config);
  const vstVendors = useMemo(
    () =>
      Array.from(
        new Set(vstPlugins.map((plugin) => plugin.vendor).filter((vendor): vendor is string => Boolean(vendor)))
      ).sort(),
    [vstPlugins]
  );
  const filteredVstPlugins = useMemo(() => {
    const query = vstSearch.trim().toLocaleLowerCase();
    return vstPlugins.filter((plugin) => {
      const haystack = `${plugin.name} ${plugin.vendor || ""} ${plugin.path}`.toLocaleLowerCase();
      return (
        (!query || haystack.includes(query)) &&
        (vstFormat === "all" || plugin.format === vstFormat) &&
        (!showVstTechnicalDetails || vstArchitecture === "all" || plugin.architecture === vstArchitecture) &&
        (vstStatus === "all" || plugin.status === vstStatus) &&
        (!showVstTechnicalDetails || vstVendor === "all" || plugin.vendor === vstVendor)
      );
    });
  }, [showVstTechnicalDetails, vstArchitecture, vstFormat, vstPlugins, vstSearch, vstStatus, vstVendor]);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("audio.title")}</CardTitle>
        <CardDescription>{t("audio.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="flex items-center justify-between">
          <Label htmlFor="audio-engine-enabled" className="flex flex-col gap-1">
            <span>{t("audio.enableEngine")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.enableEngineHint")}</span>
          </Label>
          <Switch id="audio-engine-enabled" checked={config?.audio.enabled ?? false} onCheckedChange={onToggleAudio} />
        </div>

        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-2">
            <Label htmlFor="audio-backend">{t("audio.backendLabel")}</Label>
            <Select value={config?.audio.backend || ""} onValueChange={onBackendChange} disabled={audioReloading}>
              <SelectTrigger id="audio-backend">
                <SelectValue placeholder={t("audio.backendPlaceholder")} />
              </SelectTrigger>
              <SelectContent>
                {audioBackends.map((backend) => (
                  <SelectItem key={backend} value={backend}>
                    {backend}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label htmlFor="audio-device">{t("audio.deviceLabel")}</Label>
            <Select
              value={selectedAudioDeviceValue}
              onValueChange={(value) => {
                const device = audioDevices.find((candidate) => (candidate.id || candidate.name) === value);
                if (device) {
                  void onUpdateAudioConfig({ device: device.name, deviceId: device.id || null });
                }
              }}
              disabled={!config?.audio.backend || audioReloading}
            >
              <SelectTrigger id="audio-device">
                <SelectValue placeholder={t("audio.devicePlaceholder")} />
              </SelectTrigger>
              <SelectContent>
                {audioDevices.map((device, index) => (
                  <SelectItem key={`${device.id || device.name}-${index}`} value={device.id || device.name}>
                    {device.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="grid gap-4 md:grid-cols-3">
          <div className="space-y-2">
            <Label htmlFor="audio-sample-rate">{t("audio.sampleRate")}</Label>
            <Select
              value={config?.audio.sampleRate?.toString()}
              onValueChange={(value) => onUpdateAudioConfig({ sampleRate: parseInt(value, 10) })}
              disabled={audioReloading}
            >
              <SelectTrigger id="audio-sample-rate">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {[44100, 48000, 88200, 96000].map((sampleRate) => (
                  <SelectItem key={sampleRate} value={sampleRate.toString()}>
                    {sampleRate} {t("units.hz")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="audio-buffer-size">{t("audio.bufferSize")}</Label>
            <Select
              value={config?.audio.bufferSize?.toString()}
              onValueChange={(value) => onUpdateAudioConfig({ bufferSize: parseInt(value, 10) })}
              disabled={audioReloading}
            >
              <SelectTrigger id="audio-buffer-size">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {[64, 128, 256, 480, 512, 1024, 2048].map((bufferSize) => (
                  <SelectItem key={bufferSize} value={bufferSize.toString()}>
                    {bufferSize} {t("units.samples")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="audio-master-gain">
              {t("audio.masterGain", { value: config?.audio.gainDb || 0, unit: t("units.db") })}
            </Label>
            <Input id="audio-master-gain"
              type="range"
              min={-60}
              max={12}
              step={1}
              value={config?.audio.gainDb || 0}
              onChange={(e) => onGainChange(Number(e.target.value))}
              className="w-full"
            />
          </div>
        </div>

        <details className="rounded-md border border-border/60 p-3">
          <summary className="cursor-pointer text-sm font-medium">{t("audio.advancedSettings")}</summary>
          <div className="mt-4 space-y-2">
            <Label htmlFor="audio-render-ahead">{t("audio.renderAhead")}</Label>
            <Select value={String(config?.audio.renderAheadBlocks ?? 1)} disabled={!config || audioReloading}
              onValueChange={(value) => { void onUpdateAudioConfig({ renderAheadBlocks: Number(value) }); }}>
              <SelectTrigger id="audio-render-ahead"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="0">{t("audio.renderDirect")}</SelectItem>
                <SelectItem value="1">{t("audio.renderOne")}</SelectItem>
                <SelectItem value="2">{t("audio.renderTwo")}</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">{showVstTechnicalDetails ? t("audio.renderAheadHint", {
              latency: (((config?.audio.renderAheadBlocks ?? 1) * (activeBufferSize || config?.audio.bufferSize || 512) * 1000) / (activeSampleRate || config?.audio.sampleRate || 48000)).toFixed(2),
            }) : t("audio.renderAheadSimpleHint")}</p>
          </div>
        </details>

        <div className="flex items-center justify-between">
          <Label htmlFor="audio-limiter" className="flex flex-col gap-1">
            <span>{t("audio.limiter")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.limiterHint")}</span>
          </Label>
          <Switch id="audio-limiter" checked={config?.audio.limiterEnabled ?? false} onCheckedChange={onLimiterToggle} />
        </div>

        <div className="flex items-center justify-between gap-3">
          <Label htmlFor="audio-game-mode" className="flex flex-col gap-1">
            <span>{t("audio.gameMode")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.gameModeHint")}</span>
          </Label>
          <Switch id="audio-game-mode" checked={config?.audio.gameMode ?? true}
            disabled={!config || audioReloading || status?.realtime?.worker.maintenance}
            onCheckedChange={(gameMode) => { void onUpdateAudioConfig({ gameMode }); }} />
        </div>
        {status?.realtime?.worker.maintenance && <p role="status" className="text-xs text-warning">{t("audio.maintenance")}</p>}
        {(status?.realtime?.worker.deferredSave || status?.realtime?.worker.deferredEditor != null) &&
          <p role="status" className="text-xs text-muted-foreground">{t("audio.deferredOperations")}</p>}
        {showVstTechnicalDetails && (
          <details open className="rounded-md border border-border/60 p-3">
            <summary className="cursor-pointer text-sm font-medium">{t("audio.developerDiagnostics")}</summary>
            <div className="mt-4 space-y-4">
              <div className="space-y-2">
                <Label htmlFor="audio-dsp-priority">{t("audio.dspPriority")}</Label>
                <Select value={config?.audio.dspThreadPriority ?? "mmcss"}
                  disabled={!config || audioReloading || config.audio.renderAheadBlocks === 0}
                  onValueChange={(value: "mmcss" | "timeCritical") => { void onUpdateAudioConfig({ dspThreadPriority: value }); }}>
                  <SelectTrigger id="audio-dsp-priority"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="mmcss">{t("audio.dspMmcss")}</SelectItem>
                    <SelectItem value="timeCritical">{t("audio.dspTimeCritical")}</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">{t("audio.dspPriorityHint")}</p>
              </div>
              {status?.realtime && (
                <div className="text-xs text-muted-foreground">
                  {t("audio.deliveryLatency")}: p99 {(status.realtime.worker.midiDeliveryP99Us / 1000).toFixed(2)} ms / max {(status.realtime.worker.midiDeliveryMaxUs / 1000).toFixed(2)} ms
                  <br />{t("audio.callbackInterval")}: {(status.realtime.worker.callbackIntervalMaxUs / 1000).toFixed(2)} ms
                  <br />{t("audio.renderUnderflows")}: {status.realtime.worker.renderUnderflows ?? 0} / {t("audio.driverGaps")}: {status.realtime.worker.callbackGapCount ?? 0}
                </div>
              )}

              <div className="flex items-center justify-between text-xs text-muted-foreground">
          <span>{t(latencyIsBufferPeriod ? "audio.latencyBufferPeriod" : "audio.estimatedLatency")}</span>
                <span className="font-semibold text-foreground">
                  {currentLatencyMs !== null ? `${currentLatencyMs.toFixed(2)} ${t("units.ms")}` : "--"}
                </span>
              </div>
              <div className="space-y-2">
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.activeLabel")}</span>
                  <span className="font-semibold text-foreground">
                    {activeSampleRate ? `${activeSampleRate} ${t("units.hz")}` : "--"} /{" "}
                    {activeBufferSize ? `${activeBufferSize} ${t("units.samples")}` : t("common.auto")}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.requestedBuffer")}</span>
                  <span className="font-semibold text-foreground">
                    {requestedBufferSize ? `${requestedBufferSize} ${t("units.samples")}` : "--"}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.streamBuffer")}</span>
                  <span className="font-semibold text-foreground">
                    {streamBufferSize ? `${streamBufferSize} ${t("units.samples")}` : t("common.auto")}
                  </span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.vstMidiCompat")}</span>
                  <span className="font-semibold text-foreground">
                    {vstMidiCompatible === null ? "--" : vstMidiCompatible ? "OK" : t("audio.vstMidiIncompatible")}
                  </span>
                </div>
                {bufferMismatch ? <p className="text-xs text-warning">{t("audio.bufferMismatchWarning")}</p> : null}
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.activeBackend")}</span>
                  <span className="font-semibold text-foreground">{status?.audioBackend ?? "--"}</span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.activeDevice")}</span>
                  <span className="font-semibold text-foreground">{status?.audioDevice ?? "--"}</span>
                </div>
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{t("audio.xruns")}</span>
                  <span className="font-semibold text-foreground">{status?.audioXruns ?? "--"}</span>
                </div>
                <div className="grid gap-2 rounded-md border border-border/60 bg-muted/20 p-3 text-xs text-muted-foreground md:grid-cols-2">
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioMidiDrops")}</span>
                    <span className="font-semibold text-foreground">{status?.audioMidiDrops ?? "--"}</span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioLockMisses")}</span>
                    <span className="font-semibold text-foreground">{status?.audioLockMisses ?? "--"}</span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioEmergencyResets")}</span>
                    <span className="font-semibold text-foreground">{status?.audioEmergencyResets ?? "--"}</span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioCallbackOverBudget")}</span>
                    <span className="font-semibold text-foreground">{status?.audioCallbackOverBudgetCount ?? "--"}</span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioCallbackMax")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.audioCallbackMaxUs !== null && status?.audioCallbackMaxUs !== undefined
                        ? `${status.audioCallbackMaxUs} us`
                        : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioBufferPeriod")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.audioBufferPeriodMs !== null && status?.audioBufferPeriodMs !== undefined
                        ? `${status.audioBufferPeriodMs.toFixed(2)} ms`
                        : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.pluginLatency")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.pluginLatencySamples !== null && status?.pluginLatencySamples !== undefined
                        ? `${status.pluginLatencySamples} ${t("units.samples")}`
                        : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.dspP99")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.dspProcessP99Us !== null && status?.dspProcessP99Us !== undefined
                        ? `${status.dspProcessP99Us} us`
                        : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.dspLast")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.dspProcessLastUs !== null && status?.dspProcessLastUs !== undefined
                        ? `${status.dspProcessLastUs} us`
                        : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.dspMax")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.dspProcessMaxUs !== null && status?.dspProcessMaxUs !== undefined
                        ? `${status.dspProcessMaxUs} us`
                        : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.midiThroughput")}</span>
                    <span className="font-semibold text-foreground">
                      {midiMessagesPerSec !== null ? `${midiMessagesPerSec.toLocaleString()} msg/s` : "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.midiQueue")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.audioMidiQueueDepth ?? "--"} / {status?.audioMidiQueueMaxDepth ?? "--"}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.consecutiveMisses")}</span>
                    <span className="font-semibold text-foreground">{status?.consecutiveDeadlineMisses ?? "--"}</span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span>{t("dashboard.audioMmcss")}</span>
                    <span className="font-semibold text-foreground">
                      {status?.audioMmcssEnabled === null || status?.audioMmcssEnabled === undefined
                        ? "--"
                        : status.audioMmcssEnabled
                        ? "OK"
                        : t("common.off")}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </details>
        )}

        <div className="space-y-2">
          <Label htmlFor="vst-instrument">{t("audio.vstInstrumentLabel")}</Label>
          <div className={`grid gap-2 md:grid-cols-2 ${showVstTechnicalDetails ? "xl:grid-cols-5" : "xl:grid-cols-3"}`}>
            <Input
              value={vstSearch}
              onChange={(event) => setVstSearch(event.target.value)}
              placeholder={t(showVstTechnicalDetails ? "audio.vstSearch" : "audio.vstSearchSimple")}
              aria-label={t(showVstTechnicalDetails ? "audio.vstSearch" : "audio.vstSearchSimple")}
            />
            <Select value={vstFormat} onValueChange={setVstFormat}>
              <SelectTrigger aria-label={t("audio.vstAllFormats")}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("audio.vstAllFormats")}</SelectItem>
                <SelectItem value="VST2">VST2</SelectItem>
                <SelectItem value="VST3">VST3</SelectItem>
              </SelectContent>
            </Select>
            {showVstTechnicalDetails && <Select value={vstArchitecture} onValueChange={setVstArchitecture}>
              <SelectTrigger aria-label={t("audio.vstAllArchitectures")}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("audio.vstAllArchitectures")}</SelectItem>
                <SelectItem value="x64">x64</SelectItem>
                <SelectItem value="x86">x86</SelectItem>
              </SelectContent>
            </Select>}
            <Select value={vstStatus} onValueChange={setVstStatus}>
              <SelectTrigger aria-label={t("audio.vstAllStatuses")}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("audio.vstAllStatuses")}</SelectItem>
                <SelectItem value="compatible">{t("audio.vstStatusCompatible")}</SelectItem>
                <SelectItem value="unverified">{t("audio.vstStatusUnverified")}</SelectItem>
                <SelectItem value="quarantined">{t("audio.vstStatusQuarantined")}</SelectItem>
                <SelectItem value="outOfScope">{t("audio.vstStatusOutOfScope")}</SelectItem>
                <SelectItem value="failed">{t("audio.vstStatusFailed")}</SelectItem>
              </SelectContent>
            </Select>
            {showVstTechnicalDetails ? (
              <Select value={vstVendor} onValueChange={setVstVendor}>
                <SelectTrigger aria-label={t("audio.vstAllVendors")}><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{t("audio.vstAllVendors")}</SelectItem>
                  {vstVendors.map((vendor) => (
                    <SelectItem key={vendor} value={vendor}>{vendor}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : null}
          </div>
          <div className="flex flex-col gap-2 md:flex-row md:flex-wrap md:items-center">
            <Select value={config?.audio.vstPluginId || selectedVstPlugin?.id || ""} onValueChange={onSelectVst} disabled={audioReloading}>
              <SelectTrigger id="vst-instrument" className="md:flex-1">
                <SelectValue placeholder={t("audio.vstSelectPlaceholder")} />
              </SelectTrigger>
              <SelectContent>
                {filteredVstPlugins.length === 0 ? (
                  <SelectItem value="__empty" disabled>
                    {t("audio.vstNoCached")}
                  </SelectItem>
                ) : null}
                {filteredVstPlugins.map((plugin) => (
                  <SelectItem key={plugin.id} value={plugin.id} disabled={!plugin.supported}>
                    {showVstTechnicalDetails ? [plugin.name, plugin.format, plugin.architecture, plugin.status].join(" \u00b7 ") : plugin.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button variant="outline" onClick={onRefreshVstPluginsList} disabled={vstPluginsLoading || audioReloading}>
              {vstPluginsLoading ? t("common.loading") : t("common.refresh")}
            </Button>
          </div>
          {selectedVstPlugin ? (
            <div className="space-y-2 rounded-md border border-border/60 bg-muted/30 p-3">
              <div className="flex flex-wrap gap-2">
                {showVstTechnicalDetails && <Badge variant="secondary">{selectedVstPlugin.format}</Badge>}
                {showVstTechnicalDetails && <Badge variant="outline">{selectedVstPlugin.architecture}</Badge>}
                {showVstTechnicalDetails && <Badge variant="outline">
                  {selectedVstPlugin.hostingMode === "bridgedX86" ? t("audio.vstModeBridged") : t("audio.vstModeDirect")}
                </Badge>}
                <Badge variant={selectedVstPlugin.supported ? "success" : "warning"}>
                  {selectedVstPlugin.supported ? t("audio.vstStatusCompatible") : t("audio.vstStatusUnsupported")}
                </Badge>
                {showVstTechnicalDetails && <Badge variant={selectedVstPlugin.kind === "instrument" ? "success" : "warning"}>
                  {selectedPluginKindLabel}
                </Badge>}
                {showVstTechnicalDetails && <Badge variant={selectedVstPlugin.hasEditor ? "secondary" : "outline"}>
                  {selectedVstPlugin.hasEditor ? t("audio.vstEditorAvailable") : t("audio.vstNoEditor")}
                </Badge>}
                {showVstTechnicalDetails && selectedVstPlugin.midiCompatible != null ? (
                  <Badge variant={selectedVstPlugin.midiCompatible ? "success" : "warning"}>
                    {selectedVstPlugin.midiCompatible ? t("audio.vstMidiCompatibleBadge") : t("audio.vstMidiIncompatibleBadge")}
                  </Badge>
                ) : null}
              </div>
              {showVstTechnicalDetails && <p className="break-all text-xs text-muted-foreground">{selectedVstPlugin.path}</p>}
              {showVstTechnicalDetails && selectedVstPlugin.vendor ? (
                <p className="text-xs text-muted-foreground">{selectedVstPlugin.vendor}</p>
              ) : null}
              {showVstTechnicalDetails && selectedVstPlugin.channelLayout ? (
                  <p className="text-xs text-muted-foreground">
                    {t("audio.vstChannelLayout", { layout: selectedVstPlugin.channelLayout })}
                  </p>
                ) : null}
              {selectedVstPlugin.lastError || !selectedVstPlugin.supported ? (
                <p className="text-xs text-warning">
                  {showVstTechnicalDetails && selectedVstPlugin.failureStage ? `${selectedVstPlugin.failureStage}: ` : ""}
                  {showVstTechnicalDetails ? selectedVstPlugin.lastError || selectedPluginStatusLabel : t("audio.pluginUnavailableHint")}
                </p>
              ) : null}
              {showVstTechnicalDetails && selectedVstPlugin.hostingMode === "bridgedX86" ? (
                <div className="grid gap-1 text-xs text-muted-foreground md:grid-cols-2">
                  <span>{t("audio.vstBridgeWorker")}: {status?.x86WorkerAlive ? "OK" : t("common.off")}</span>
                  <span>{t("audio.vstBridgeLatency")}: {status?.bridgeLatencySamples ?? config?.audio.bufferSize ?? "--"} {t("units.samples")}</span>
                  <span>{t("audio.vstPluginLatency")}: {status?.pluginLatencySamples ?? "--"} {t("units.samples")}</span>
                  <span>{t("audio.vstBridgeUnderruns")}: {status?.x86BridgeUnderruns ?? 0} / {status?.x86BridgeOverruns ?? 0}</span>
                </div>
              ) : null}
              {showVstTechnicalDetails && <div className="flex flex-wrap gap-2">
                <Button variant="outline" onClick={() => onRetestVst(selectedVstPlugin.id)} disabled={vstPluginsLoading}>
                  {t("audio.vstRetest")}
                </Button>
                <Button variant="outline" onClick={() => onOpenVstFolder(selectedVstPlugin.id)}>
                  {t("audio.vstOpenFolder")}
                </Button>
              </div>}
            </div>
          ) : null}
          <details className="rounded-md border border-border/60 p-3">
            <summary className="cursor-pointer text-sm font-medium">{t("audio.vstLibrarySettings")}</summary>
            <div className="mt-3 space-y-2">
              <Label htmlFor="vst-scan-paths">{t("audio.vstCustomPaths")}</Label>
              <Input
                id="vst-scan-paths"
                value={scanPathsDraft ?? savedScanPaths}
                onChange={(event) => setScanPathsDraft(event.target.value)}
                onBlur={async (event) => {
                  const vstScanPaths = event.target.value.split(";").map((path) => path.trim()).filter(Boolean);
                  if (vstScanPaths.join(";") !== savedScanPaths) {
                    const accepted = await onUpdateAudioConfig({ vstScanPaths });
                    if (accepted === false) return;
                  }
                  setScanPathsDraft(null);
                }}
                placeholder={t("audio.vstCustomPathsPlaceholder")}
              />
              <p className="text-xs text-muted-foreground">{t("audio.vstCustomPathsHint")}</p>
            </div>
          </details>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={onOpenVstEq} disabled={!canOpenVstEq}>
              <SlidersHorizontal className="mr-2 h-4 w-4" />
              {t("audio.eq.open")}
              {vstEqEnabled ? <Badge variant="success" className="ml-2">{t("audio.eq.active")}</Badge> : null}
            </Button>
            <Button variant="outline" onClick={onOpenVstUi} disabled={vstUiOpen || !canOpenSelectedVstUi}>
              {t("audio.openVstUi")}
            </Button>
            {vstUiOpen ? (
              <Button variant="outline" onClick={onCloseVstUi}>
                <X className="mr-2 h-4 w-4" />
                {t("audio.hideUi")}
              </Button>
            ) : null}
            {selectedVstPlugin?.supported ? (
              <Button variant="outline" onClick={onOpenVstParameters} disabled={!canOpenVstParameterFallback}>
                {t("audio.openVstParameters")}
              </Button>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">{t("audio.vstSettingsSaved")}</p>
          <p className="text-xs text-muted-foreground">{t("audio.vstMidiOnly")}</p>
          {showVstTechnicalDetails && <p className="text-xs text-muted-foreground">{t("audio.vstFormats")}</p>}
          {selectedVstPlugin?.supported ? <p className="text-xs text-muted-foreground">{t("audio.vstParameterFallback")}</p> : null}
          {!bridgeRunning ? <p className="text-xs text-warning">{t("audio.startBridgeHint")}</p> : null}
          <div className="flex flex-wrap gap-2 pt-2">
            <Button variant="outline" onClick={onPingAudio} disabled={!bridgeRunning || !config?.audio.enabled}>
              <Power className="mr-2 h-4 w-4" />
              {t("audio.pingAudio")}
            </Button>
            <Button variant="outline" onClick={onReloadVst} disabled={!config?.audio.enabled || audioReloading}>
              <RefreshCcw className={`mr-2 h-4 w-4 ${audioReloading ? "animate-spin" : ""}`} />
              {audioReloading ? t("common.loading") : t("audio.reloadVst")}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">{t("audio.pingHint")}</p>
        </div>

        <div className="flex justify-end gap-2 pt-4">
          <Button variant="outline" onClick={onSaveConfig}>
            {t("audio.saveConfiguration")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
