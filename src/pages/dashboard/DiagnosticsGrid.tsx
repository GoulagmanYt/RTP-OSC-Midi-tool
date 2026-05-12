import { BarChart3, Music, Power, RefreshCcw, Timer, Waves } from "lucide-react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { AppConfig, PreflightReport, RuntimeStatus } from "../../api-types";
import type { TranslateFn } from "../audio/shared";
import { MeterRow } from "./shared";

type Props = {
  audioPeakL?: number | null;
  audioPeakR?: number | null;
  bufferMismatch: boolean;
  bufferSamples?: number;
  config: AppConfig | null;
  displayPortName: (name: string) => string;
  dropoutsActive: boolean;
  latencyBadge: "secondary" | "warning" | "success";
  latencyDisplay: number | null;
  midiRate?: number | null;
  oscRate?: number | null;
  audioMidiDrops?: number | null;
  audioLockMisses?: number | null;
  audioEmergencyResets?: number | null;
  audioCallbackMaxUs?: number | null;
  audioCallbackOverBudgetCount?: number | null;
  audioMmcssEnabled?: boolean | null;
  audioPowerThrottlingDisabled?: boolean | null;
  preflight: PreflightReport | null;
  requestedBufferSamples?: number;
  sampleRate?: number;
  status: RuntimeStatus | null;
  streamBufferSamples?: number;
  t: TranslateFn;
  vstMidiCompatible: boolean | null;
  vstPath: string;
  shortVstPath: string;
  xrunCount?: number | null;
  onHandlePing: () => Promise<void>;
  onHandleReloadVst: () => Promise<void>;
  onRunPreflight: () => Promise<unknown>;
};

export function DiagnosticsGrid({
  audioPeakL,
  audioPeakR,
  bufferMismatch,
  bufferSamples,
  config,
  displayPortName,
  dropoutsActive,
  latencyBadge,
  latencyDisplay,
  preflight,
  midiRate,
  oscRate,
  audioMidiDrops,
  audioLockMisses,
  audioEmergencyResets,
  audioCallbackMaxUs,
  audioCallbackOverBudgetCount,
  audioMmcssEnabled,
  audioPowerThrottlingDisabled,
  requestedBufferSamples,
  sampleRate,
  status,
  streamBufferSamples,
  t,
  vstMidiCompatible,
  vstPath,
  shortVstPath,
  xrunCount,
  onHandlePing,
  onHandleReloadVst,
  onRunPreflight,
}: Props) {
  const renderPreflightBadge = (value: boolean | undefined, label: string) => {
    const variant: "warning" | "success" | "destructive" =
      value === undefined ? "warning" : value ? "success" : "destructive";
    const text =
      value === undefined ? t("dashboard.preflight.status.pending") : value ? t("dashboard.preflight.status.ok") : t("dashboard.preflight.status.missing");
    return (
      <div className="flex items-center justify-between">
        <span>{label}</span>
        <Badge variant={variant}>{text}</Badge>
      </div>
    );
  };

  const formatPercent = (value?: number | null) => (value === null || value === undefined ? "--" : `${Math.round(value * 100)}%`);
  const formatRate = (value?: number | null) => (value === null || value === undefined ? "--" : `${value}/s`);

  return (
    <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.midiVst")}</CardTitle>
          <Waves className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          <div className="flex min-w-0 items-center justify-between gap-3">
            <span>{t("dashboard.midiIn")}</span>
            <span className="max-w-[60%] truncate text-right font-medium" title={status?.midiInput ? displayPortName(status.midiInput) : t("common.none")}>
              {status?.midiInput ? displayPortName(status.midiInput) : t("common.none")}
            </span>
          </div>
          <div className="flex min-w-0 items-center justify-between gap-3">
            <span>{t("dashboard.midiOut")}</span>
            <span className="max-w-[60%] truncate text-right font-medium" title={status?.midiOutput ? displayPortName(status.midiOutput) : t("common.none")}>
              {status?.midiOutput ? displayPortName(status.midiOutput) : t("common.none")}
            </span>
          </div>
          <div className="flex min-w-0 items-center justify-between gap-3">
            <span>{t("dashboard.vstPath")}</span>
            <span className="max-w-[60%] truncate text-right font-medium" title={vstPath}>
              {shortVstPath}
            </span>
          </div>
          <p className="text-xs text-muted-foreground">{t("dashboard.vstHint")}</p>
        </CardContent>
      </Card>

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.runtimeSignals")}</CardTitle>
          <Music className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          <div className="flex items-center justify-between">
            <span>{t("dashboard.audioEngine")}</span>
            <Badge variant={config?.audio.enabled ? "success" : "secondary"}>
              {config?.audio.enabled ? t("common.on") : t("common.off")}
            </Badge>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("routing.hotplug")}</span>
            <Badge variant={config?.midi.hotplug ? "success" : "secondary"}>
              {config?.midi.hotplug ? t("common.enabled") : t("common.disabled")}
            </Badge>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.logging")}</span>
            <Badge variant={config?.logging.logAllToFile ? "success" : "secondary"}>
              {config?.logging.logAllToFile ? t("dashboard.loggingToFile") : t("dashboard.loggingConsoleOnly")}
            </Badge>
          </div>
          <p className="text-xs text-muted-foreground">{t("dashboard.logsHint")}</p>
        </CardContent>
      </Card>

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.latencyVst")}</CardTitle>
          <Timer className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          <div className="flex items-center justify-between">
            <span>{t("dashboard.buffer")}</span>
            <span className="font-medium">{bufferSamples ? `${bufferSamples} ${t("units.samples")}` : "--"}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.sampleRate")}</span>
            <span className="font-medium">{sampleRate ? `${sampleRate} ${t("units.hz")}` : "--"}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.bufferRequested")}</span>
            <span className="font-medium">{requestedBufferSamples ? `${requestedBufferSamples} ${t("units.samples")}` : "--"}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.bufferStream")}</span>
            <span className="font-medium">{streamBufferSamples ? `${streamBufferSamples} ${t("units.samples")}` : t("common.auto")}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.midiVstCompat")}</span>
            <span className="font-medium">
              {vstMidiCompatible === null ? "--" : vstMidiCompatible ? t("dashboard.loaded") : t("dashboard.missing")}
            </span>
          </div>
          {bufferMismatch ? <p className="text-xs text-amber-600">{t("dashboard.bufferMismatchWarning")}</p> : null}
          <div className="flex items-center justify-between">
            <span>{t("dashboard.estRoundtrip")}</span>
            <span className="font-medium">{latencyDisplay !== null ? `${latencyDisplay} ${t("units.ms")}` : "--"}</span>
          </div>
          <div className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <span>{t("dashboard.vstHealth")}</span>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={status?.vstLoaded ? "success" : "destructive"}>
                {status?.vstLoaded ? t("dashboard.loaded") : t("dashboard.missing")}
              </Badge>
              <Button variant="outline" size="sm" onClick={onHandleReloadVst} className="whitespace-nowrap">
                <RefreshCcw className="mr-1 h-4 w-4" />
                {t("dashboard.reload")}
              </Button>
            </div>
          </div>
          <div className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <span>{t("dashboard.testAudio")}</span>
            <Button variant="outline" size="sm" onClick={onHandlePing} className="w-fit whitespace-nowrap">
              <Power className="mr-1 h-4 w-4" />
              {t("dashboard.ping")}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">{t("dashboard.latencyHint")}</p>
        </CardContent>
      </Card>

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.preflightTitle")}</CardTitle>
          <RefreshCcw className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          {renderPreflightBadge(preflight?.midiInOk, t("dashboard.preflight.midiIn"))}
          {renderPreflightBadge(preflight?.midiOutOk, t("dashboard.preflight.midiOut"))}
          {renderPreflightBadge(preflight?.audioBackendOk, t("dashboard.preflight.audioBackend"))}
          {renderPreflightBadge(preflight?.rtpPortOk, t("dashboard.preflight.rtpPort"))}
          <div className="flex items-center justify-between pt-2">
            <Button variant="outline" size="sm" onClick={onRunPreflight}>
              {t("dashboard.rerunCheck")}
            </Button>
          </div>
          {preflight?.messages?.length ? (
            <ul className="list-disc space-y-1 break-words pl-4 text-xs text-muted-foreground">
              {preflight.messages.map((msg, i) => (
                <li key={i}>{msg}</li>
              ))}
            </ul>
          ) : (
            <p className="text-xs text-muted-foreground">{t("dashboard.preflightNote")}</p>
          )}
        </CardContent>
      </Card>

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.meters")}</CardTitle>
          <BarChart3 className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-4 text-sm">
          <div className="space-y-3">
            <div className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.metersAudio")}</div>
            <MeterRow
              label={t("dashboard.metersLeft")}
              value={audioPeakL}
              max={1}
              valueLabel={formatPercent(audioPeakL)}
              className="bg-gradient-to-r from-emerald-400 via-emerald-500 to-emerald-600"
            />
            <MeterRow
              label={t("dashboard.metersRight")}
              value={audioPeakR}
              max={1}
              valueLabel={formatPercent(audioPeakR)}
              className="bg-gradient-to-r from-emerald-400 via-emerald-500 to-emerald-600"
            />
          </div>
          <div className="space-y-3">
            <div className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.metersMidi")}</div>
            <MeterRow
              label={t("dashboard.metersMessages")}
              value={midiRate}
              max={200}
              valueLabel={formatRate(midiRate)}
              className="bg-gradient-to-r from-sky-400 to-sky-600"
            />
          </div>
          <div className="space-y-3">
            <div className="text-xs font-semibold uppercase text-muted-foreground">{t("dashboard.metersOsc")}</div>
            <MeterRow
              label={t("dashboard.metersMessages")}
              value={oscRate}
              max={200}
              valueLabel={formatRate(oscRate)}
              className="bg-gradient-to-r from-amber-400 to-amber-600"
            />
          </div>
          <div className="min-w-0 space-y-2 rounded-lg border bg-muted/20 p-3">
            <div className="flex min-w-0 items-center justify-between gap-3">
              <span>{t("dashboard.monitorLatency")}</span>
              <Badge variant={latencyBadge}>{latencyDisplay !== null ? `${latencyDisplay} ${t("units.ms")}` : "--"}</Badge>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.xruns")}</span>
              <span className="font-medium">{xrunCount ?? "--"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioMidiDrops")}</span>
              <span className="font-medium">{audioMidiDrops ?? "--"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioLockMisses")}</span>
              <span className="font-medium">{audioLockMisses ?? "--"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioEmergencyResets")}</span>
              <span className="font-medium">{audioEmergencyResets ?? "--"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioCallbackMax")}</span>
              <span className="font-medium">
                {audioCallbackMaxUs !== null && audioCallbackMaxUs !== undefined ? `${audioCallbackMaxUs} us` : "--"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioCallbackOverBudget")}</span>
              <span className="font-medium">{audioCallbackOverBudgetCount ?? "--"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioMmcss")}</span>
              <Badge variant={audioMmcssEnabled === false ? "warning" : audioMmcssEnabled ? "success" : "secondary"}>
                {audioMmcssEnabled === null || audioMmcssEnabled === undefined
                  ? "--"
                  : audioMmcssEnabled
                  ? t("common.on")
                  : t("common.off")}
              </Badge>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioPowerTuning")}</span>
              <Badge
                variant={
                  audioPowerThrottlingDisabled === false
                    ? "warning"
                    : audioPowerThrottlingDisabled
                    ? "success"
                    : "secondary"
                }
              >
                {audioPowerThrottlingDisabled === null || audioPowerThrottlingDisabled === undefined
                  ? "--"
                  : audioPowerThrottlingDisabled
                  ? t("common.on")
                  : t("common.off")}
              </Badge>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.monitorDropouts")}</span>
              <Badge variant={dropoutsActive ? "destructive" : "success"}>
                {dropoutsActive ? t("dashboard.dropoutsDetected") : t("dashboard.stable")}
              </Badge>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
