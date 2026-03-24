import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardHeader, CardTitle } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import {
  Activity,
  BarChart3,
  Music,
  Globe,
  Play,
  Square,
  AlertCircle,
  Volume2,
  Waves,
  Timer,
  ScrollText,
  Power,
  RefreshCcw,
} from "lucide-react";
import { pingAudio, reloadVst } from "../api";
import { toast } from "sonner";
import { useI18n } from "../providers/LanguageProvider";
import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../constants";
import { cn } from "../utils";

type MeterRowProps = {
  label: string;
  value?: number | null;
  max: number;
  valueLabel?: string;
  className?: string;
};

function MeterRow({ label, value, max, valueLabel, className }: MeterRowProps) {
  const safeValue = value ?? 0;
  const pct = max > 0 ? Math.min(safeValue / max, 1) : 0;
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{label}</span>
        <span className="font-medium text-foreground">{valueLabel ?? "--"}</span>
      </div>
      <div className="h-2 rounded-full bg-muted/30 overflow-hidden">
        <div
          className={cn("h-full transition-all duration-200", className)}
          style={{ width: `${pct * 100}%` }}
        />
      </div>
    </div>
  );
}

export default function Dashboard() {
  const navigate = useNavigate();
  const { status, config, metrics, toggleBridge, logs, preflight, runPreflight, refreshStatus } =
    useBridge();
  const { t } = useI18n();
  const [lastDropoutAt, setLastDropoutAt] = useState<number | null>(null);
  const lastXrun = useRef<number | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const rtpPort = status?.rtpBoundPort ?? config?.rtpPort;
  const audioBackend = status?.audioBackend || config?.audioBackend || t("common.auto");
  const audioDevice = status?.audioDevice || config?.audioDevice || t("dashboard.systemDefault");
  const vstPath = config?.vstPath || t("dashboard.bundledVstDefault");
  const shortVstPath = vstPath.length > 42 ? `...${vstPath.slice(vstPath.length - 42)}` : vstPath;
  const bufferSamples = status?.audioBufferSize ?? config?.audioBufferSize ?? undefined;
  const requestedBufferSamples =
    status?.audioRequestedBufferSize ?? config?.audioBufferSize ?? undefined;
  const sampleRate = status?.audioSampleRate ?? config?.audioSampleRate ?? undefined;
  const streamBufferSamples = status?.audioStreamBufferSize ?? undefined;
  const bufferMismatch =
    status?.audioBufferMismatch ??
    (bufferSamples && requestedBufferSamples ? bufferSamples !== requestedBufferSamples : false);
  const vstMidiCompatible = status?.vstMidiCompatible ?? null;
  const limiterEnabled = status?.audioLimiterEnabled ?? config?.audioLimiterEnabled ?? false;
  const xrunCount = metrics?.audioXruns ?? status?.audioXruns ?? null;
  const latencyMs =
    metrics?.audioLatencyMs ??
    status?.audioLatencyMs ??
    (bufferSamples && sampleRate ? Number(((bufferSamples / sampleRate) * 1000 * 2).toFixed(2)) : null);
  const audioPeakL = metrics?.audioPeakL ?? null;
  const audioPeakR = metrics?.audioPeakR ?? null;
  const midiRate = metrics ? metrics.midiMessagesPerSec : null;
  const oscRate = metrics ? metrics.oscMessagesPerSec : null;
  const dropoutsActive = lastDropoutAt !== null && now - lastDropoutAt < 30_000;
  const latencyBadge: "secondary" | "warning" | "success" =
    latencyMs === null ? "secondary" : latencyMs > 40 ? "warning" : "success";
  const latencyDisplay = latencyMs !== null ? Number(latencyMs.toFixed(1)) : null;

  const formatPercent = (value?: number | null) =>
    value === null || value === undefined ? "--" : `${Math.round(value * 100)}%`;
  const formatRate = (value?: number | null) =>
    value === null || value === undefined ? "--" : `${value}/s`;

  useEffect(() => {
    if (metrics?.audioXruns === null || metrics?.audioXruns === undefined) {
      return;
    }
    const current = metrics.audioXruns;
    const previous = lastXrun.current;
    lastXrun.current = current;
    if (previous !== null && current > previous) {
      setLastDropoutAt(Date.now());
    }
  }, [metrics?.audioXruns]);

  useEffect(() => {
    if (lastDropoutAt === null) return;
    const id = window.setInterval(() => setNow(Date.now()), 5000);
    return () => window.clearInterval(id);
  }, [lastDropoutAt]);

  const logPreview = logs.slice(0, 3);

  const displayPortName = (name: string) => {
    if (name === RTP_VIRTUAL_INPUT) return t("devices.rtpVirtualInput");
    if (name === VST_OUTPUT) return t("devices.vstInternalOutput");
    return name;
  };

  const handlePing = async () => {
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

  const renderPreflightBadge = (value: boolean | undefined, label: string) => {
    const variant = value === undefined ? "warning" : value ? "success" : "destructive";
    const text =
      value === undefined
        ? t("dashboard.preflight.status.pending")
        : value
        ? t("dashboard.preflight.status.ok")
        : t("dashboard.preflight.status.missing");
    return (
      <div className="flex items-center justify-between">
        <span>{label}</span>
        <Badge variant={variant as any}>{text}</Badge>
      </div>
    );
  };

  return (
    <div className="space-y-6 min-w-0">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.bridgeStatus")}</CardTitle>
            <Activity className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-4 min-w-0">
            <div>
              <div className="text-3xl font-semibold">
                {status?.running ? t("common.running") : t("common.stopped")}
              </div>
              <p className="text-sm text-muted-foreground">{t("dashboard.bridgeCore")}</p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={status?.running ? "success" : "secondary"}>
                {status?.running ? t("common.active") : t("common.idle")}
              </Badge>
              <Badge variant={status?.rtpActive ? "success" : "secondary"}>
                RTP {status?.rtpActive ? t("dashboard.rtpOnline") : t("dashboard.rtpOff")}
              </Badge>
            </div>
            <Button
              size="sm"
              variant={status?.running ? "destructive" : "default"}
              onClick={toggleBridge}
              className="w-full"
            >
              {status?.running ? <Square className="mr-2 h-4 w-4 fill-current" /> : <Play className="mr-2 h-4 w-4 fill-current" />}
              {status?.running ? t("actions.stopBridge") : t("actions.startBridge")}
            </Button>
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.audioEngine")}</CardTitle>
            <Volume2 className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            <div className="flex items-center justify-between gap-3 min-w-0">
              <span>{t("dashboard.backend")}</span>
              <span className="font-medium max-w-[60%] truncate text-right" title={audioBackend}>
                {audioBackend}
              </span>
            </div>
            <div className="flex items-center justify-between gap-3 min-w-0">
              <span>{t("dashboard.device")}</span>
              <span className="font-medium max-w-[60%] truncate text-right" title={audioDevice}>
                {audioDevice}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.sampleRate")}</span>
              <span className="font-medium">
                {sampleRate ? `${sampleRate} ${t("units.hz")}` : "--"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.buffer")}</span>
              <span className="font-medium">
                {bufferSamples ? `${bufferSamples} ${t("units.samples")}` : "--"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.gain")}</span>
              <span className="font-medium">{config?.audioGainDb ?? 0} {t("units.db")}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.limiter")}</span>
              <span className="font-medium">{limiterEnabled ? t("common.on") : t("common.off")}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.xruns")}</span>
              <span className="font-medium">{xrunCount ?? "--"}</span>
            </div>
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.oscRtp")}</CardTitle>
            <Globe className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            <div>
              <div className="text-lg font-semibold truncate min-w-0" title={status?.oscTarget}>
                {config?.oscTargetIp}:{config?.oscTargetPort}
              </div>
              <p className="text-xs text-muted-foreground">{t("dashboard.oscTarget")}</p>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.oscOutput")}</span>
              <Badge variant={config?.oscEnabled ? "success" : "secondary"}>
                {config?.oscEnabled ? t("common.enabled") : t("common.disabled")}
              </Badge>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.rtpPort")}</span>
              <span className="font-medium">{rtpPort ?? "--"}</span>
            </div>
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.quickLogs")}</CardTitle>
            <ScrollText className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            {logPreview.length === 0 && (
              <p className="text-muted-foreground text-xs">{t("dashboard.noRecentLogs")}</p>
            )}
            {logPreview.map((log, idx) => (
              <div key={idx} className="flex items-start gap-2">
                <Badge
                  variant={
                    log.level === "error"
                      ? "destructive"
                      : log.level === "warn"
                      ? "warning"
                      : "secondary"
                  }
                  className="shrink-0"
                >
                  {t(`logs.level.${log.level}`)}
                </Badge>
                <div className="flex-1 min-w-0">
                  <div className="text-xs text-muted-foreground">{log.timestamp}</div>
                  <div className="text-sm leading-tight line-clamp-2 break-words">{log.message}</div>
                </div>
              </div>
            ))}
            <Button variant="link" className="px-0" onClick={() => navigate("/logs")}>
              {t("dashboard.viewMore")}
            </Button>
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.midiVst")}</CardTitle>
            <Waves className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            <div className="flex items-center justify-between gap-3 min-w-0">
              <span>{t("dashboard.midiIn")}</span>
              <span
                className="font-medium max-w-[60%] truncate text-right"
                title={status?.midiIn ? displayPortName(status.midiIn) : t("common.none")}
              >
                {status?.midiIn ? displayPortName(status.midiIn) : t("common.none")}
              </span>
            </div>
            <div className="flex items-center justify-between gap-3 min-w-0">
              <span>{t("dashboard.midiOut")}</span>
              <span
                className="font-medium max-w-[60%] truncate text-right"
                title={status?.midiOut ? displayPortName(status.midiOut) : t("common.none")}
              >
                {status?.midiOut ? displayPortName(status.midiOut) : t("common.none")}
              </span>
            </div>
            <div className="flex items-center justify-between gap-3 min-w-0">
              <span>{t("dashboard.vstPath")}</span>
              <span className="font-medium max-w-[60%] truncate text-right" title={vstPath}>
                {shortVstPath}
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("dashboard.vstHint")}
            </p>
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.runtimeSignals")}</CardTitle>
            <Music className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            <div className="flex items-center justify-between">
              <span>{t("dashboard.audioEngine")}</span>
              <Badge variant={config?.audioEnabled ? "success" : "secondary"}>
                {config?.audioEnabled ? t("common.on") : t("common.off")}
              </Badge>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("routing.hotplug")}</span>
              <Badge variant={config?.hotplug ? "success" : "secondary"}>
                {config?.hotplug ? t("common.enabled") : t("common.disabled")}
              </Badge>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.logging")}</span>
              <Badge variant={config?.logAllToFile ? "success" : "secondary"}>
                {config?.logAllToFile ? t("dashboard.loggingToFile") : t("dashboard.loggingConsoleOnly")}
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground">{t("dashboard.logsHint")}</p>
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.latencyVst")}</CardTitle>
            <Timer className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            <div className="flex items-center justify-between">
              <span>{t("dashboard.buffer")}</span>
              <span className="font-medium">
                {bufferSamples ? `${bufferSamples} ${t("units.samples")}` : "--"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.sampleRate")}</span>
              <span className="font-medium">{sampleRate ? `${sampleRate} ${t("units.hz")}` : "--"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.bufferRequested")}</span>
              <span className="font-medium">
                {requestedBufferSamples ? `${requestedBufferSamples} ${t("units.samples")}` : "--"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.bufferStream")}</span>
              <span className="font-medium">
                {streamBufferSamples ? `${streamBufferSamples} ${t("units.samples")}` : t("common.auto")}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t("dashboard.midiVstCompat")}</span>
              <span className="font-medium">
                {vstMidiCompatible === null
                  ? "--"
                  : vstMidiCompatible
                  ? t("dashboard.loaded")
                  : t("dashboard.missing")}
              </span>
            </div>
            {bufferMismatch && (
              <p className="text-xs text-amber-600">
                {t("dashboard.bufferMismatchWarning")}
              </p>
            )}
            <div className="flex items-center justify-between">
              <span>{t("dashboard.estRoundtrip")}</span>
              <span className="font-medium">
                {latencyDisplay !== null ? `${latencyDisplay} ${t("units.ms")}` : "--"}
              </span>
            </div>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between min-w-0">
              <span>{t("dashboard.vstHealth")}</span>
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={status?.vstLoaded ? "success" : "destructive"}>
                  {status?.vstLoaded ? t("dashboard.loaded") : t("dashboard.missing")}
                </Badge>
                <Button variant="outline" size="sm" onClick={handleReloadVst} className="whitespace-nowrap">
                  <RefreshCcw className="w-4 h-4 mr-1" />
                  {t("dashboard.reload")}
                </Button>
              </div>
            </div>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between min-w-0">
              <span>{t("dashboard.testAudio")}</span>
              <Button variant="outline" size="sm" onClick={handlePing} className="w-fit whitespace-nowrap">
                <Power className="w-4 h-4 mr-1" />
                {t("dashboard.ping")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">{t("dashboard.latencyHint")}</p>
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.preflightTitle")}</CardTitle>
            <RefreshCcw className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-3 text-sm min-w-0">
            {renderPreflightBadge(preflight?.midiInOk, t("dashboard.preflight.midiIn"))}
            {renderPreflightBadge(preflight?.midiOutOk, t("dashboard.preflight.midiOut"))}
            {renderPreflightBadge(preflight?.audioBackendOk, t("dashboard.preflight.audioBackend"))}
            {renderPreflightBadge(preflight?.rtpPortOk, t("dashboard.preflight.rtpPort"))}
            <div className="flex items-center justify-between pt-2">
              <Button variant="outline" size="sm" onClick={runPreflight}>
                {t("dashboard.rerunCheck")}
              </Button>
            </div>
            {preflight?.messages?.length ? (
              <ul className="text-xs text-muted-foreground list-disc pl-4 space-y-1 break-words">
                {preflight.messages.map((msg, i) => (
                  <li key={i}>{msg}</li>
                ))}
              </ul>
            ) : (
              <p className="text-xs text-muted-foreground">{t("dashboard.preflightNote")}</p>
            )}
          </CardContent>
        </Card>

        <Card className="bg-card/60 backdrop-blur-sm border-muted min-h-[180px] min-w-0">
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-base font-semibold">{t("dashboard.meters")}</CardTitle>
            <BarChart3 className="h-5 w-5 text-muted-foreground" />
          </CardHeader>
          <CardContent className="space-y-4 text-sm min-w-0">
            <div className="space-y-3">
              <div className="text-xs font-semibold uppercase text-muted-foreground">
                {t("dashboard.metersAudio")}
              </div>
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
              <div className="text-xs font-semibold uppercase text-muted-foreground">
                {t("dashboard.metersMidi")}
              </div>
              <MeterRow
                label={t("dashboard.metersMessages")}
                value={midiRate}
                max={200}
                valueLabel={formatRate(midiRate)}
                className="bg-gradient-to-r from-sky-400 to-sky-600"
              />
            </div>
            <div className="space-y-3">
              <div className="text-xs font-semibold uppercase text-muted-foreground">
                {t("dashboard.metersOsc")}
              </div>
              <MeterRow
                label={t("dashboard.metersMessages")}
                value={oscRate}
                max={200}
                valueLabel={formatRate(oscRate)}
                className="bg-gradient-to-r from-amber-400 to-amber-600"
              />
            </div>
            <div className="rounded-lg border bg-muted/20 p-3 space-y-2 min-w-0">
              <div className="flex items-center justify-between gap-3 min-w-0">
                <span>{t("dashboard.monitorLatency")}</span>
                <Badge variant={latencyBadge}>
                  {latencyDisplay !== null ? `${latencyDisplay} ${t("units.ms")}` : "--"}
                </Badge>
              </div>
              <div className="flex items-center justify-between">
                <span>{t("dashboard.xruns")}</span>
                <span className="font-medium">{xrunCount ?? "--"}</span>
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

      {status?.lastError && (
        <Card className="border-destructive/50 bg-destructive/10">
          <CardHeader className="pb-2">
            <CardTitle className="text-lg text-destructive flex items-center gap-2">
              <AlertCircle className="w-5 h-5" /> {t("dashboard.errorDetected")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-sm font-mono text-destructive-foreground break-words">{status.lastError}</p>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
