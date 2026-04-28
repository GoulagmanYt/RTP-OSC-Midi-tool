import { Activity, Globe, Play, ScrollText, Square, Volume2 } from "lucide-react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { AppConfig, LogEntry, RuntimeStatus } from "../../api-types";
import type { TranslateFn } from "../audio/shared";

type Props = {
  audioBackend: string;
  audioDevice: string;
  bufferSamples?: number;
  config: AppConfig | null;
  limiterEnabled: boolean;
  logPreview: LogEntry[];
  rtpPort?: number | null;
  sampleRate?: number;
  status: RuntimeStatus | null;
  t: TranslateFn;
  xrunCount?: number | null;
  onNavigateLogs: () => void;
  onToggleBridge: () => Promise<void> | void;
};

export function OverviewGrid({
  audioBackend,
  audioDevice,
  bufferSamples,
  config,
  limiterEnabled,
  logPreview,
  rtpPort,
  sampleRate,
  status,
  t,
  xrunCount,
  onNavigateLogs,
  onToggleBridge,
}: Props) {
  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.bridgeStatus")}</CardTitle>
          <Activity className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-4">
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
          <Button size="sm" variant={status?.running ? "destructive" : "default"} onClick={onToggleBridge} className="w-full">
            {status?.running ? <Square className="mr-2 h-4 w-4 fill-current" /> : <Play className="mr-2 h-4 w-4 fill-current" />}
            {status?.running ? t("actions.stopBridge") : t("actions.startBridge")}
          </Button>
        </CardContent>
      </Card>

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.audioEngine")}</CardTitle>
          <Volume2 className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          <div className="flex min-w-0 items-center justify-between gap-3">
            <span>{t("dashboard.backend")}</span>
            <span className="max-w-[60%] truncate text-right font-medium" title={audioBackend}>
              {audioBackend}
            </span>
          </div>
          <div className="flex min-w-0 items-center justify-between gap-3">
            <span>{t("dashboard.device")}</span>
            <span className="max-w-[60%] truncate text-right font-medium" title={audioDevice}>
              {audioDevice}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.sampleRate")}</span>
            <span className="font-medium">{sampleRate ? `${sampleRate} ${t("units.hz")}` : "--"}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.buffer")}</span>
            <span className="font-medium">{bufferSamples ? `${bufferSamples} ${t("units.samples")}` : "--"}</span>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.gain")}</span>
            <span className="font-medium">
              {config?.audio.gainDb ?? 0} {t("units.db")}
            </span>
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

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.oscRtp")}</CardTitle>
          <Globe className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          <div>
            <div className="min-w-0 truncate text-lg font-semibold" title={status?.oscTarget}>
              {config?.osc.targetIp}:{config?.osc.targetPort}
            </div>
            <p className="text-xs text-muted-foreground">{t("dashboard.oscTarget")}</p>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.oscOutput")}</span>
            <Badge variant={config?.osc.enabled ? "success" : "secondary"}>
              {config?.osc.enabled ? t("common.enabled") : t("common.disabled")}
            </Badge>
          </div>
          <div className="flex items-center justify-between">
            <span>{t("dashboard.rtpPort")}</span>
            <span className="font-medium">{rtpPort ?? "--"}</span>
          </div>
        </CardContent>
      </Card>

      <Card className="min-h-[180px] min-w-0 border-muted bg-card/60 backdrop-blur-sm">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-semibold">{t("dashboard.quickLogs")}</CardTitle>
          <ScrollText className="h-5 w-5 text-muted-foreground" />
        </CardHeader>
        <CardContent className="min-w-0 space-y-3 text-sm">
          {logPreview.length === 0 ? <p className="text-xs text-muted-foreground">{t("dashboard.noRecentLogs")}</p> : null}
          {logPreview.map((log, idx) => (
            <div key={idx} className="flex items-start gap-2">
              <Badge
                variant={log.level === "error" ? "destructive" : log.level === "warn" ? "warning" : "secondary"}
                className="shrink-0"
              >
                {t(`logs.level.${log.level}`)}
              </Badge>
              <div className="min-w-0 flex-1">
                <div className="text-xs text-muted-foreground">{log.timestamp}</div>
                <div className="line-clamp-2 break-words text-sm leading-tight">{log.message}</div>
              </div>
            </div>
          ))}
          {config?.ui.developerMode && (
            <Button variant="link" className="px-0" onClick={onNavigateLogs}>
              {t("dashboard.viewMore")}
            </Button>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
