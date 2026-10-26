import { BarChart3, Music, Power, RefreshCcw, Timer } from "lucide-react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { AppConfig, PreflightReport, RuntimeMetrics, RuntimeStatus } from "../../api-types";
import type { TranslateFn } from "../audio/shared";
import type { AudioLatency } from "../audio/latency";
import { MeterRow } from "./shared";

type Props = {
  audioBackend: string;
  bufferMismatch: boolean;
  bufferSamples?: number;
  config: AppConfig | null;
  dropoutsActive: boolean;
  latency: AudioLatency;
  metrics: RuntimeMetrics | null;
  preflight: PreflightReport | null;
  requestedBufferSamples?: number;
  sampleRate?: number;
  status: RuntimeStatus | null;
  streamBufferSamples?: number;
  t: TranslateFn;
  vstPath: string;
  onHandlePing: () => Promise<void>;
  onHandleReloadVst: () => Promise<void>;
  onRunPreflight: () => Promise<unknown>;
};

export function DiagnosticsGrid({
  audioBackend, bufferMismatch, bufferSamples, config, dropoutsActive, latency,
  metrics, preflight, requestedBufferSamples, sampleRate, status,
  streamBufferSamples, t, vstPath, onHandlePing, onHandleReloadVst, onRunPreflight,
}: Props) {
  const samples = (value?: number | null) => value == null ? "--" : `${value} ${t("units.samples")}`;
  const number = (value?: number | null) => value == null ? "--" : String(value);
  const row = (label: string, value: React.ReactNode) => (
    <div className="flex min-w-0 items-start justify-between gap-3">
      <span className="min-w-0">{label}</span>
      <span className="min-w-0 break-words text-right font-medium tabular-nums">{value}</span>
    </div>
  );
  const booleanBadge = (value?: boolean | null) => (
    <Badge variant={value === false ? "warning" : value ? "success" : "secondary"}>
      {value == null ? "--" : value ? t("common.on") : t("common.off")}
    </Badge>
  );
  const loggingEnabled = config?.logging?.enabled ?? true;
  const fullFileLogging = Boolean(config?.logging?.verbose || config?.logging?.logAllToFile);
  const loggingLabel = !config ? t("common.unknown") : !loggingEnabled ? t("common.disabled")
    : fullFileLogging ? t("dashboard.loggingToFile") : t("dashboard.loggingWarningsToFile");
  const renderPreflightBadge = (value: boolean | undefined, label: string) => row(label,
    <Badge variant={value === undefined ? "warning" : value ? "success" : "destructive"}>
      {value === undefined ? t("dashboard.preflight.status.pending") : value ? t("dashboard.preflight.status.ok") : t("dashboard.preflight.status.missing")}
    </Badge>,
  );
  const latencyVariant = latency.ms === null || latency.source === "bufferPeriod" ? "secondary" : latency.ms > 40 ? "warning" : "success";
  const formatPercent = (value?: number | null) => value == null ? "--" : `${Math.round(value * 100)}%`;
  const formatRate = (value?: number | null) => value == null ? "--" : `${value}/s`;

  return (
    <div className="space-y-4">
      <div className="grid items-start gap-4 xl:grid-cols-3">
        <Card className="min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.runtimeSignals")}</CardTitle>
            <Music className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="min-w-0 space-y-3 text-sm">
            {row(t("dashboard.workerState"), status?.vstWorkerState ? t(`dashboard.workerStates.${status.vstWorkerState}`) : "--")}
            {row(t("dashboard.workerRestarts"), number(status?.vstWorkerRestarts))}
            {status?.vstWorkerLastExit ? row(t("dashboard.workerLastExit"), status.vstWorkerLastExit) : null}
            {row(t("routing.hotplug"), <Badge variant={config?.midi.hotplug ? "success" : "secondary"}>{config?.midi.hotplug ? t("common.enabled") : t("common.disabled")}</Badge>)}
            {row(t("dashboard.logging"), <Badge variant={loggingEnabled && fullFileLogging ? "success" : "secondary"}>{loggingLabel}</Badge>)}
            <div className="border-t pt-3">
              <p className="text-xs text-muted-foreground">{t("dashboard.vstPath")}</p>
              <p className="mt-1 break-all font-mono text-xs">{vstPath}</p>
            </div>
          </CardContent>
        </Card>

        <Card className="min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.latencyVst")}</CardTitle>
            <Timer className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="min-w-0 space-y-3 text-sm">
            {row(t("dashboard.backend"), audioBackend)}
            {row(t("dashboard.sampleRate"), sampleRate ? `${sampleRate} ${t("units.hz")}` : "--")}
            {row(t("dashboard.bufferActive"), samples(bufferSamples))}
            {row(t("dashboard.bufferRequested"), samples(requestedBufferSamples))}
            {row(t("dashboard.bufferStream"), samples(streamBufferSamples))}
            {bufferMismatch ? <p className="text-xs text-warning">{t("dashboard.bufferMismatchWarning")}</p> : null}
            {row(t(latency.source === "bufferPeriod" ? "audio.latencyBufferPeriod" : "dashboard.estRoundtrip"),
              <Badge variant={latencyVariant}>{latency.ms === null ? "--" : `${latency.ms.toFixed(2)} ${t("units.ms")}`}</Badge>)}
            <p className="text-xs text-muted-foreground">{t(latency.source === "bufferPeriod" ? "dashboard.bufferPeriodHint" : "dashboard.latencyEstimateHint")}</p>
            {row(t("dashboard.midiVstCompat"), status?.vstMidiCompatible == null ? "--" : status.vstMidiCompatible ? t("audio.vstMidiCompatibleBadge") : t("audio.vstMidiIncompatibleBadge"))}
            <div className="flex flex-wrap items-center gap-2 border-t pt-3">
              <Badge variant={status?.vstLoaded ? "success" : "secondary"}>{status?.vstLoaded ? t("dashboard.loaded") : t("dashboard.missing")}</Badge>
              <Button variant="outline" size="sm" onClick={onHandleReloadVst} disabled={!config?.audio.enabled}>
                <RefreshCcw className="mr-1 h-4 w-4" />{t("dashboard.reload")}
              </Button>
              <Button variant="outline" size="sm" onClick={onHandlePing} disabled={!status?.audioRunning}>
                <Power className="mr-1 h-4 w-4" />{t("dashboard.ping")}
              </Button>
            </div>
          </CardContent>
        </Card>

        <Card className="min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.preflightTitle")}</CardTitle>
            <RefreshCcw className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="min-w-0 space-y-3 text-sm">
            {renderPreflightBadge(preflight?.midiInOk, t("dashboard.preflight.midiIn"))}
            {renderPreflightBadge(preflight?.midiOutOk, t("dashboard.preflight.midiOut"))}
            {renderPreflightBadge(preflight?.audioBackendOk, t("dashboard.preflight.audioBackend"))}
            {renderPreflightBadge(preflight?.rtpPortOk, t("dashboard.preflight.rtpPort"))}
            <Button variant="outline" size="sm" onClick={onRunPreflight}>{t("dashboard.rerunCheck")}</Button>
            {preflight?.messages?.length ? <ul className="list-disc space-y-1 break-words pl-4 text-xs text-muted-foreground">
              {preflight.messages.map((message, index) => <li key={index}>{message}</li>)}
            </ul> : <p className="text-xs text-muted-foreground">{t("dashboard.preflightNote")}</p>}
          </CardContent>
        </Card>
      </div>

      <Card className="min-w-0">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.meters")}</CardTitle>
          <BarChart3 className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-5 text-sm">
          <div className="grid gap-6 md:grid-cols-3">
            <div className="space-y-3">
              <h3 className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.metersAudio")}</h3>
              <MeterRow label={t("dashboard.metersLeft")} value={metrics?.audioPeakL} max={1} valueLabel={formatPercent(metrics?.audioPeakL)} className="bg-emerald-500" />
              <MeterRow label={t("dashboard.metersRight")} value={metrics?.audioPeakR} max={1} valueLabel={formatPercent(metrics?.audioPeakR)} className="bg-emerald-500" />
            </div>
            <div className="space-y-3">
              <h3 className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.metersMidi")}</h3>
              <MeterRow label={t("dashboard.metersMessages")} value={metrics?.midiMessagesPerSec} max={200} valueLabel={formatRate(metrics?.midiMessagesPerSec)} className="bg-sky-500" />
            </div>
            <div className="space-y-3">
              <h3 className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.metersOsc")}</h3>
              <MeterRow label={t("dashboard.metersMessages")} value={metrics?.oscMessagesPerSec} max={200} valueLabel={formatRate(metrics?.oscMessagesPerSec)} className="bg-amber-500" />
            </div>
          </div>
          <section className="space-y-3 border-t pt-4" aria-label={t("dashboard.transportTitle")}>
            <h3 className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.transportTitle")}</h3>
            <div className="grid gap-x-6 gap-y-3 md:grid-cols-2">
              {row(t("dashboard.bridgeQueue"), metrics ? `${number(metrics.bridgeQueueDepth)} / ${number(metrics.bridgeQueueMaxDepth)}` : "--")}
              {row(t("dashboard.bridgeMessages"), metrics ? `${number(metrics.bridgeMessagesIn)} / ${number(metrics.bridgeMessagesOut)}` : "--")}
              {row(t("dashboard.bridgeDrops"), number(metrics?.bridgeMessagesDropped))}
              {row(t("dashboard.rtpDrops"), number(metrics?.rtpMidiDrops))}
            </div>
          </section>
          <div className="grid gap-x-6 gap-y-3 rounded-lg border bg-muted/20 p-4 md:grid-cols-2 xl:grid-cols-3">
            {row(t("dashboard.xruns"), number(status?.audioXruns))}
            {row(t("dashboard.audioMidiDrops"), number(status?.audioMidiDrops))}
            {row(t("dashboard.audioLockMisses"), number(status?.audioLockMisses))}
            {row(t("dashboard.audioEmergencyResets"), number(status?.audioEmergencyResets))}
            {row(t("dashboard.audioCallbackMax"), status?.audioCallbackMaxUs == null ? "--" : `${status.audioCallbackMaxUs} µs`)}
            {row(t("dashboard.audioCallbackOverBudget"), number(status?.audioCallbackOverBudgetCount))}
            {row(t("dashboard.audioMmcss"), booleanBadge(status?.audioMmcssEnabled))}
            {row(t("dashboard.audioPowerTuning"), booleanBadge(status?.audioPowerThrottlingDisabled))}
            {row(t("dashboard.monitorDropouts"), <Badge variant={!status?.audioRunning ? "secondary" : dropoutsActive ? "destructive" : "success"}>
              {!status?.audioRunning ? t("common.stopped") : dropoutsActive ? t("dashboard.dropoutsDetected") : t("dashboard.stable")}
            </Badge>)}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
