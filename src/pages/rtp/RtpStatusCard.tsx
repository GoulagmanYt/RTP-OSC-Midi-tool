import { Badge } from "../../components/ui/Badge";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { AppConfig, RuntimeStatus } from "../../api-types";
import type { TranslateFn } from "./shared";
import { Server } from "lucide-react";
import { isDeveloperMode } from "../../developer-mode";

type Props = {
  config: AppConfig | null;
  status: RuntimeStatus | null;
  t: TranslateFn;
};

export function RtpStatusCard({ config, status, t }: Props) {
  const boundPort = status?.rtpActive ? status.rtpBoundPort : null;
  const connectionPort = boundPort ?? config?.rtp.port;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <Server className="h-4 w-4" />
          {t("rtp.statusTitle")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <dl className="space-y-3 text-sm">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <dt className="text-muted-foreground">{t("rtp.sessionNameLabel")}</dt>
            <dd className="break-all font-medium">{config?.rtp.sessionName || t("common.unknown")}</dd>
          </div>
          <div className="flex items-center justify-between gap-2">
            <dt className="text-muted-foreground">{t("rtp.statusLabel")}</dt>
            <dd><Badge variant={status?.rtpActive ? "success" : "secondary"}>
              {!status ? t("common.unknown") : status.rtpActive ? t("rtp.ready") : t("rtp.waiting")}
            </Badge></dd>
          </div>
          <div className="flex items-center justify-between gap-2">
            <dt className="text-muted-foreground">{t(boundPort ? "rtp.boundPorts" : "rtp.connectionPorts")}</dt>
            <dd className="font-mono">{connectionPort ? `${connectionPort}/${connectionPort + 1}` : t("common.unknown")}</dd>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <dt className="text-muted-foreground">{t("rtp.connectionAddresses")}</dt>
            <dd className="min-w-0 break-all text-right font-medium">
              {status?.rtpAdvertisedAddresses?.length ? status.rtpAdvertisedAddresses.join(", ") : t("common.unknown")}
            </dd>
          </div>
        </dl>
        {status?.rtpNetworkWarning ? (
          <p role="status" className="rounded border border-warning bg-warning-surface p-2 text-xs text-warning">
            {status.rtpNetworkWarning}
          </p>
        ) : null}
        <details open={isDeveloperMode(config)} className="border-t pt-4">
          <summary className="cursor-pointer text-sm font-medium">{t("rtp.networkDetails")}</summary>
          <dl className="mt-3 space-y-3 text-sm">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <dt className="text-muted-foreground">{t("rtp.advertisedHost")}</dt>
              <dd className="min-w-0 break-all font-medium">{status?.rtpAdvertisedHost || t("common.unknown")}</dd>
            </div>
            {boundPort && boundPort !== config?.rtp.port ? (
              <div className="flex items-center justify-between gap-2">
                <dt className="text-muted-foreground">{t("rtp.portConfigured")}</dt>
                <dd className="font-mono">{config?.rtp.port ?? t("common.unknown")}</dd>
              </div>
            ) : null}
          </dl>
        </details>
      </CardContent>
    </Card>
  );
}
