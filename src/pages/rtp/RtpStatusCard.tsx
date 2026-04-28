import { Badge } from "../../components/ui/Badge";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { AppConfig, RuntimeStatus } from "../../api-types";
import type { TranslateFn } from "./shared";
import { Server } from "lucide-react";

type Props = {
  config: AppConfig | null;
  status: RuntimeStatus | null;
  t: TranslateFn;
};

export function RtpStatusCard({ config, status, t }: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <Server className="h-4 w-4" />
          {t("rtp.statusTitle")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-3 md:grid-cols-2">
          <div className="rounded-lg border border-slate-100 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/50">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-sm text-muted-foreground">{t("rtp.server")}</span>
              <Badge variant={status?.rtpActive ? "success" : "secondary"}>
                {status?.rtpActive ? t("common.active") : t("common.inactive")}
              </Badge>
            </div>
            <div className="text-xs text-muted-foreground">
              {config?.rtp.sessionName || "OSCMidi"} - {t("dashboard.rtpPort")} {config?.rtp.port || 5004}
            </div>
          </div>

          <div className="rounded-lg border border-slate-100 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/50">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-sm text-muted-foreground">{t("rtp.boundPorts")}</span>
              <code className="rounded bg-slate-200 px-1.5 py-0.5 font-mono text-xs dark:bg-slate-700">
                {status?.rtpBoundPort ? `${status.rtpBoundPort}/${status.rtpBoundPort + 1}` : "-"}
              </code>
            </div>
            <div className="text-xs text-muted-foreground">
              {status?.rtpActive ? (
                <div className="flex items-center gap-1">
                  <div className="h-1.5 w-1.5 animate-pulse rounded-full bg-green-500" />
                  {t("rtp.ready")}
                </div>
              ) : (
                t("rtp.waiting")
              )}
            </div>
          </div>
        </div>

        <div className="space-y-2 border-t pt-4">
          <h4 className="text-sm font-semibold">{t("rtp.details")}</h4>
          <div className="space-y-2">
            <div className="flex items-center justify-between rounded p-2 text-sm">
              <span className="text-muted-foreground">{t("rtp.sessionNameLabel")}</span>
              <span className="font-medium">{config?.rtp.sessionName || "-"}</span>
            </div>
            <div className="flex items-center justify-between rounded p-2 text-sm">
              <span className="text-muted-foreground">{t("rtp.portConfigured")}</span>
              <span className="font-medium">{config?.rtp.port || "-"}</span>
            </div>
            <div className="flex items-center justify-between rounded p-2 text-sm">
              <span className="text-muted-foreground">{t("rtp.portData")}</span>
              <span className="font-medium">{config?.rtp.port ? config.rtp.port + 1 : "-"}</span>
            </div>
            <div className="flex items-center justify-between gap-3 rounded p-2 text-sm">
              <span className="text-muted-foreground">{t("rtp.advertisedHost")}</span>
              <span className="min-w-0 truncate font-medium">{status?.rtpAdvertisedHost || "-"}</span>
            </div>
            <div className="flex items-center justify-between gap-3 rounded p-2 text-sm">
              <span className="text-muted-foreground">{t("rtp.advertisedAddresses")}</span>
              <span className="min-w-0 truncate font-medium">
                {status?.rtpAdvertisedAddresses?.length ? status.rtpAdvertisedAddresses.join(", ") : "-"}
              </span>
            </div>
            {status?.rtpNetworkWarning ? (
              <div className="rounded border border-amber-200 bg-amber-50 p-2 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100">
                {status.rtpNetworkWarning}
              </div>
            ) : null}
            <div className="flex items-center justify-between rounded p-2 text-sm">
              <span className="text-muted-foreground">{t("rtp.statusLabel")}</span>
              <span className="font-medium">
                {status?.rtpActive ? t("common.active") : t("common.inactive")}
              </span>
            </div>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
