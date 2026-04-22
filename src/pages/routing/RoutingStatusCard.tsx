import type { AppConfig, MidiActivityInfo, RuntimeStatus } from "../../api";
import { Activity } from "lucide-react";

import { Badge } from "../../components/ui/Badge";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";

import type { Translate } from "./shared";

type Props = {
  t: Translate;
  config: AppConfig | null;
  status: RuntimeStatus | null;
  activity: MidiActivityInfo[];
  displayPortName: (name: string) => string;
  formatNote: (note?: number | null) => string;
};

export function RoutingStatusCard({ t, config, status, activity, displayPortName, formatNote }: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <Activity className="h-4 w-4" />
          {t("routing.statusTitle")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-3">
          <div className="flex items-center justify-between rounded-lg border border-slate-100 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/50">
            <span className="text-sm text-muted-foreground">{t("routing.input")}</span>
            <Badge variant={config?.midi.inputDevice ? "success" : "secondary"}>
              {config?.midi.inputDevice ? displayPortName(config.midi.inputDevice) : t("routing.notConfigured")}
            </Badge>
          </div>
          <div className="flex items-center justify-between rounded-lg border border-slate-100 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/50">
            <span className="text-sm text-muted-foreground">{t("routing.output")}</span>
            <Badge variant={config?.midi.outputDevice ? "success" : "secondary"}>
              {config?.midi.outputDevice ? displayPortName(config.midi.outputDevice) : t("routing.notConfigured")}
            </Badge>
          </div>
          <div className="flex items-center justify-between rounded-lg border border-slate-100 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/50">
            <span className="text-sm text-muted-foreground">{t("routing.bridge")}</span>
            <Badge variant={status?.running ? "success" : "secondary"}>
              {status?.running ? t("common.active") : t("common.stopped")}
            </Badge>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">{t("routing.tip")}</p>
        <div className="space-y-3 border-t pt-4">
          <h4 className="text-sm font-semibold">{t("routing.activityTitle")}</h4>
          {activity.length === 0 ? (
            <div className="text-xs text-muted-foreground">{t("routing.activityEmpty")}</div>
          ) : (
            <div className="space-y-2">
              {activity.map((item) => {
                const isActive = typeof item.lastSeenMs === "number" && Date.now() - item.lastSeenMs < 1200;
                const level = Math.min(item.messagesPerSec ?? 0, 60);
                return (
                  <div
                    key={item.source}
                    className="flex items-center justify-between rounded-md border border-slate-100 bg-white/70 px-3 py-2 dark:border-slate-800 dark:bg-slate-950/40"
                  >
                    <div className="flex items-center gap-3">
                      <div className={`h-2.5 w-2.5 rounded-full ${isActive ? "bg-emerald-500 animate-pulse" : "bg-slate-400"}`} />
                      <div>
                        <div className="text-sm font-medium">{item.source}</div>
                        <div className="text-xs text-muted-foreground">
                          {t("routing.activityNote")} {formatNote(item.lastNote)} - {t("routing.activityChannel")} {item.lastChannel ?? "--"}
                        </div>
                      </div>
                    </div>
                    <div className="space-y-1 text-right">
                      <div className="text-sm font-semibold">{item.messagesPerSec ?? 0}/s</div>
                      <div className="h-1.5 w-20 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
                        <div className="h-full bg-emerald-500" style={{ width: `${(level / 60) * 100}%` }} />
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
