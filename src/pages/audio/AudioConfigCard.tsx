import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../components/ui/Select";
import { Switch } from "../../components/ui/Switch";
import type { AppConfig, RuntimeStatus, VstPluginEntry } from "../../api-types";
import type { TranslateFn } from "./shared";
import { Power, RefreshCcw, X } from "lucide-react";
import { useMemo, useState } from "react";

type Props = {
  audioBackends: string[];
  audioDevices: string[];
  audioReloading: boolean;
  bridgeRunning: boolean;
  bufferMismatch: boolean | null;
  canOpenSelectedVstUi: boolean;
  canOpenVstParameterFallback: boolean;
  config: AppConfig | null;
  currentLatencyMs: number | null;
  midiMessagesPerSec: number | null;
  selectedPluginKindLabel: string;
  selectedPluginStatusLabel: string;
  selectedVstPlugin: VstPluginEntry | null;
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
  onOpenVstUi: () => Promise<void>;
  onPingAudio: () => Promise<void>;
  onRefreshVstPluginsList: () => Promise<void>;
  onReloadVst: () => Promise<void>;
  onSaveConfig: () => Promise<void>;
  onSelectVst: (pluginId: string) => Promise<void>;
  onRetestVst: (pluginId: string) => Promise<void>;
  onOpenVstFolder: (pluginId: string) => Promise<void>;
  onToggleAudio: (checked: boolean) => Promise<void>;
  onUpdateAudioConfig: (patch: Partial<AppConfig["audio"]>) => Promise<void>;
};

export function AudioConfigCard({
  audioBackends,
  audioDevices,
  audioReloading,
  bridgeRunning,
  bufferMismatch,
  canOpenSelectedVstUi,
  canOpenVstParameterFallback,
  config,
  currentLatencyMs,
  midiMessagesPerSec,
  selectedPluginKindLabel,
  selectedPluginStatusLabel,
  selectedVstPlugin,
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
  const [vstSearch, setVstSearch] = useState("");
  const [vstFormat, setVstFormat] = useState("all");
  const [vstArchitecture, setVstArchitecture] = useState("all");
  const [vstStatus, setVstStatus] = useState("all");
  const [vstVendor, setVstVendor] = useState("all");
  const showVstTechnicalDetails = config?.ui.developerMode === true;
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
        (vstArchitecture === "all" || plugin.architecture === vstArchitecture) &&
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
          <Label className="flex flex-col gap-1">
            <span>{t("audio.enableEngine")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.enableEngineHint")}</span>
          </Label>
          <Switch checked={config?.audio.enabled ?? false} onCheckedChange={onToggleAudio} />
        </div>

        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-2">
            <Label>{t("audio.backendLabel")}</Label>
            <Select value={config?.audio.backend || ""} onValueChange={onBackendChange} disabled={audioReloading}>
              <SelectTrigger>
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
            <Label>{t("audio.deviceLabel")}</Label>
            <Select
              value={config?.audio.device || ""}
              onValueChange={(value) => onUpdateAudioConfig({ device: value, deviceId: null })}
              disabled={!config?.audio.backend || audioReloading}
            >
              <SelectTrigger>
                <SelectValue placeholder={t("audio.devicePlaceholder")} />
              </SelectTrigger>
              <SelectContent>
                {audioDevices.map((device) => (
                  <SelectItem key={device} value={device}>
                    {device}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="grid gap-4 md:grid-cols-3">
          <div className="space-y-2">
            <Label>{t("audio.sampleRate")}</Label>
            <Select
              value={config?.audio.sampleRate?.toString()}
              onValueChange={(value) => onUpdateAudioConfig({ sampleRate: parseInt(value, 10) })}
              disabled={audioReloading}
            >
              <SelectTrigger>
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
            <Label>{t("audio.bufferSize")}</Label>
            <Select
              value={config?.audio.bufferSize?.toString()}
              onValueChange={(value) => onUpdateAudioConfig({ bufferSize: parseInt(value, 10) })}
              disabled={audioReloading}
            >
              <SelectTrigger>
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
            <Label>
              {t("audio.masterGain", { value: config?.audio.gainDb || 0, unit: t("units.db") })}
            </Label>
            <Input
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

        <div className="flex items-center justify-between">
          <Label className="flex flex-col gap-1">
            <span>{t("audio.limiter")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.limiterHint")}</span>
          </Label>
          <Switch checked={config?.audio.limiterEnabled ?? false} onCheckedChange={onLimiterToggle} />
        </div>

        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <span>{t("audio.estimatedLatency")}</span>
          <span className="font-semibold text-foreground">
            {currentLatencyMs !== null ? `${currentLatencyMs.toFixed(2)} ${t("units.ms")}` : "--"}
          </span>
        </div>
        {config?.ui.developerMode ? (
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
            {bufferMismatch ? <p className="text-xs text-amber-600">{t("audio.bufferMismatchWarning")}</p> : null}
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
        ) : null}

        <div className="space-y-2">
          <Label>{t("audio.vstInstruments", { path: "Windows VST folders" })}</Label>
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-5">
            <Input
              value={vstSearch}
              onChange={(event) => setVstSearch(event.target.value)}
              placeholder={t("audio.vstSearch")}
              aria-label={t("audio.vstSearch")}
            />
            <Select value={vstFormat} onValueChange={setVstFormat}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("audio.vstAllFormats")}</SelectItem>
                <SelectItem value="VST2">VST2</SelectItem>
                <SelectItem value="VST3">VST3</SelectItem>
              </SelectContent>
            </Select>
            <Select value={vstArchitecture} onValueChange={setVstArchitecture}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("audio.vstAllArchitectures")}</SelectItem>
                <SelectItem value="x64">x64</SelectItem>
                <SelectItem value="x86">x86</SelectItem>
              </SelectContent>
            </Select>
            <Select value={vstStatus} onValueChange={setVstStatus}>
              <SelectTrigger><SelectValue /></SelectTrigger>
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
                <SelectTrigger><SelectValue /></SelectTrigger>
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
              <SelectTrigger className="md:flex-1">
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
                    {`${plugin.name} · ${plugin.format} · ${plugin.architecture} · ${plugin.status}`}
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
                <Badge variant="secondary">{selectedVstPlugin.format}</Badge>
                <Badge variant="outline">{selectedVstPlugin.architecture}</Badge>
                <Badge variant="outline">
                  {selectedVstPlugin.hostingMode === "bridgedX86" ? t("audio.vstModeBridged") : t("audio.vstModeDirect")}
                </Badge>
                <Badge variant={selectedVstPlugin.supported ? "success" : "warning"}>
                  {selectedVstPlugin.supported ? t("audio.vstStatusCompatible") : t("audio.vstStatusUnsupported")}
                </Badge>
                <Badge variant={selectedVstPlugin.kind === "instrument" ? "success" : "warning"}>
                  {selectedPluginKindLabel}
                </Badge>
                <Badge variant={selectedVstPlugin.hasEditor ? "secondary" : "outline"}>
                  {selectedVstPlugin.hasEditor ? t("audio.vstEditorAvailable") : t("audio.vstNoEditor")}
                </Badge>
                {selectedVstPlugin.midiCompatible !== null ? (
                  <Badge variant={selectedVstPlugin.midiCompatible ? "success" : "warning"}>
                    {selectedVstPlugin.midiCompatible ? t("audio.vstMidiCompatibleBadge") : t("audio.vstMidiIncompatibleBadge")}
                  </Badge>
                ) : null}
              </div>
              <p className="break-all text-xs text-muted-foreground">{selectedVstPlugin.path}</p>
              {showVstTechnicalDetails && selectedVstPlugin.vendor ? (
                <p className="text-xs text-muted-foreground">{selectedVstPlugin.vendor}</p>
              ) : null}
              {showVstTechnicalDetails && selectedVstPlugin.channelLayout ? (
                  <p className="text-xs text-muted-foreground">
                    {t("audio.vstChannelLayout", { layout: selectedVstPlugin.channelLayout })}
                  </p>
                ) : null}
              {selectedVstPlugin.lastError || !selectedVstPlugin.supported ? (
                <p className="text-xs text-amber-600">
                  {selectedVstPlugin.failureStage ? `${selectedVstPlugin.failureStage}: ` : ""}
                  {selectedVstPlugin.lastError || selectedPluginStatusLabel}
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
              <div className="flex flex-wrap gap-2">
                <Button variant="outline" onClick={() => onRetestVst(selectedVstPlugin.id)} disabled={vstPluginsLoading}>
                  {t("audio.vstRetest")}
                </Button>
                <Button variant="outline" onClick={() => onOpenVstFolder(selectedVstPlugin.id)}>
                  {t("audio.vstOpenFolder")}
                </Button>
              </div>
            </div>
          ) : null}
          <div className="space-y-2 rounded-md border border-border/60 p-3">
            <Label htmlFor="vst-scan-paths">{t("audio.vstCustomPaths")}</Label>
            <Input
              id="vst-scan-paths"
              value={(config?.audio.vstScanPaths || []).join(";")}
              onChange={(event) =>
                onUpdateAudioConfig({
                  vstScanPaths: event.target.value.split(";").map((path) => path.trim()).filter(Boolean),
                })
              }
              placeholder={t("audio.vstCustomPathsPlaceholder")}
            />
            <p className="text-xs text-muted-foreground">{t("audio.vstCustomPathsHint")}</p>
          </div>
          <div className="flex flex-wrap gap-2">
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
          <p className="text-xs text-muted-foreground">{t("audio.vstFormats")}</p>
          {selectedVstPlugin?.supported ? <p className="text-xs text-muted-foreground">{t("audio.vstParameterFallback")}</p> : null}
          {!bridgeRunning ? <p className="text-xs text-amber-600">{t("audio.startBridgeHint")}</p> : null}
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
