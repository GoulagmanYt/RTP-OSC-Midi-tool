import { Link } from "react-router-dom";
import { ArrowUpRight, ScrollText } from "lucide-react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { AppConfig, LogEntry, RuntimeStatus } from "../../api-types";
import type { TranslateFn } from "../audio/shared";
import { isDeveloperMode } from "../../developer-mode";
import { selectLogPreview } from "./events";

type Props = {
  audioDevice: string;
  config: AppConfig | null;
  limiterEnabled: boolean;
  logs: LogEntry[];
  rtpPort?: number | null;
  status: RuntimeStatus | null;
  t: TranslateFn;
  onNavigateLogs: () => void;
};

export function OverviewGrid({ audioDevice, config, limiterEnabled, logs, rtpPort, status, t, onNavigateLogs }: Props) {
  const developerMode = isDeveloperMode(config);
  const logPreview = selectLogPreview(logs, developerMode);
  return (
    <div className="dashboard-overview grid gap-4">
      <div className="overview-grid grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="flex-row flex-wrap items-center justify-between gap-2 space-y-0">
            <CardTitle>{t("dashboard.audioEngine")}</CardTitle>
            <Badge variant={status?.audioRunning ? "success" : "secondary"}>
              {status ? status.audioRunning ? t("common.running") : t("common.stopped") : t("common.unknown")}
            </Badge>
          </CardHeader>
          <CardContent>
            <p className="break-words text-sm font-medium">{audioDevice}</p>
            <dl className="mt-3">
              <div className="data-row"><dt>{t("dashboard.gain")} / {t("dashboard.limiter")}</dt><dd className="text-sm tabular-nums">{config ? `${config.audio.gainDb} ${t("units.db")}` : "—"} / {limiterEnabled ? t("common.on") : t("common.off")}</dd></div>
            </dl>
            <Button asChild variant="link" size="sm" className="mt-2 min-h-8 px-0"><Link to="/audio">{t("dashboard.configureAudio")}<ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" /></Link></Button>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("dashboard.oscRtp")}</CardTitle></CardHeader>
          <CardContent>
            <p className="break-all font-mono text-sm font-medium">{config ? `${config.osc.targetIp}:${config.osc.targetPort}` : "—"}</p>
            <p className="mt-1 text-xs text-muted-foreground">{t("dashboard.oscTarget")}</p>
            <dl className="mt-3">
              <div className="data-row"><dt>{t("dashboard.oscOutput")}</dt><dd>{config?.osc.enabled ? t("common.enabled") : t("common.disabled")}</dd></div>
              <div className="data-row"><dt>{t("dashboard.rtpSession")}</dt><dd className="text-sm tabular-nums">{config?.rtp.sessionName || "—"}</dd></div>
              <div className="data-row"><dt>{t("dashboard.rtpPort")}</dt><dd className="font-mono">{rtpPort ?? "—"}</dd></div>
              <div className="data-row"><dt>RTP-MIDI</dt><dd><Badge variant={status?.rtpActive ? "success" : "secondary"}>{status ? status.rtpActive ? t("dashboard.rtpOnline") : t("dashboard.rtpOff") : t("common.unknown")}</Badge></dd></div>
            </dl>
            <Button asChild variant="link" size="sm" className="mt-2 min-h-8 px-0"><Link to="/rtp">{t("dashboard.configureNetwork")}<ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" /></Link></Button>
          </CardContent>
        </Card>
      </div>
      <Card className="overview-events" role="region" aria-labelledby="recent-events-title">
        <CardHeader className="flex-row flex-wrap items-center justify-between gap-2 space-y-0">
          <CardTitle id="recent-events-title" className="flex items-center gap-2"><ScrollText className="h-4 w-4 text-muted-foreground" aria-hidden="true" />{t("dashboard.quickLogs")}</CardTitle>
          {developerMode && <Button variant="link" size="sm" className="min-h-8 px-0" onClick={onNavigateLogs}>{t("dashboard.viewMore")}<ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" /></Button>}
        </CardHeader>
        <CardContent>
          {logPreview.length === 0 ? <p className="overview-events-empty text-sm text-muted-foreground">{t("dashboard.noRecentLogs")}</p> : (
            <ul className="overview-events-list divide-y" tabIndex={0} aria-labelledby="recent-events-title">
              {logPreview.map((log, idx) => {
                const message = developerMode ? log.message : t(log.level === "error" ? "dashboard.eventsError" : "dashboard.eventsWarning");
                return (
                  <li key={idx} className="flex flex-wrap items-start gap-x-3 gap-y-1 py-2 text-sm">
                    <span className="font-mono text-xs leading-6 text-muted-foreground">{log.timestamp}</span>
                    <Badge variant={log.level === "error" ? "destructive" : log.level === "warn" ? "warning" : "secondary"}>{t(`logs.level.${log.level}`)}</Badge>
                    <p className="min-w-0 flex-1 break-words leading-6" title={message}>{message}</p>
                  </li>
                );
              })}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
