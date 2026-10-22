import { Link } from "react-router-dom";
import { ArrowDownToLine, ArrowRightLeft, ArrowUpFromLine, AudioLines, Cable, ChevronRight, Globe2, RadioTower, type LucideIcon } from "lucide-react";
import type { AppConfig, RuntimeStatus } from "../../api-types";
import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../../constants";
import { Badge } from "../../components/ui/Badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import type { TranslateFn } from "../audio/shared";

type Props = { config: AppConfig | null; status: RuntimeStatus | null; t: TranslateFn };
type Port = { label: string; detail: string; enabled: boolean; to: string; icon: LucideIcon };

export function SignalFlow({ config, status, t }: Props) {
  // RTP also runs for forwarding, remote invitations, or the virtual MIDI input.
  const rtpRequested = Boolean(config && (config.rtp.enabled || config.rtp.thruEnabled || config.rtp.remoteEnabled || config.midi.inputDevice === RTP_VIRTUAL_INPUT));
  const portName = (name?: string | null) => {
    if (name === RTP_VIRTUAL_INPUT) return t("devices.rtpVirtualInput");
    if (name === VST_OUTPUT) return t("devices.vstInternalOutput");
    return name || t("common.none");
  };
  const sources: Port[] = [
    { label: "MIDI", detail: portName(config?.midi.inputDevice), enabled: Boolean(config?.midi.inputDevice), to: "/routing", icon: Cable },
    { label: "RTP-MIDI", detail: config?.rtp.sessionName || t("common.none"), enabled: rtpRequested, to: "/rtp", icon: Globe2 },
    { label: "OSC", detail: config ? `${config.osc.listenIp}:${config.osc.listenPort}` : t("common.none"), enabled: Boolean(config?.osc.inputEnabled), to: "/osc", icon: RadioTower },
  ];
  const destinations: Port[] = [
    { label: "MIDI Thru", detail: portName(config?.midi.outputDevice), enabled: Boolean(config?.midi.thruEnabled && config.midi.outputDevice), to: "/routing", icon: Cable },
    { label: "RTP-MIDI", detail: config?.rtp.sessionName || t("common.none"), enabled: Boolean(config?.rtp.thruEnabled), to: "/rtp", icon: Globe2 },
    { label: "OSC", detail: config ? `${config.osc.targetIp}:${config.osc.targetPort}` : t("common.none"), enabled: Boolean(config?.osc.enabled), to: "/osc", icon: RadioTower },
    { label: t("dashboard.flow.instrument"), detail: config?.audio.vstPath?.split(/[\\/]/).pop() || t("dashboard.bundledVstDefault"), enabled: Boolean(config?.audio.enabled), to: "/audio", icon: AudioLines },
  ];

  const renderPort = (port: Port) => (
    <li key={port.label}>
      <Link to={port.to} className="signal-port group">
        <span className="port-symbol grid h-8 w-8 shrink-0 place-items-center rounded-[10px] bg-accent text-primary" aria-hidden="true"><port.icon className="h-4 w-4" strokeWidth={1.75} /></span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <span className="text-sm font-medium">{port.label}</span>
            <span className="text-xs text-muted-foreground">{port.enabled ? t("common.enabled") : t("common.disabled")}</span>
          </div>
          <p className="mt-0.5 break-all text-xs leading-relaxed text-muted-foreground">{port.detail}</p>
        </div>
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground group-hover:text-primary" aria-hidden="true" />
      </Link>
    </li>
  );

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle>{t("dashboard.flow.title")}</CardTitle>
        <CardDescription>{t("dashboard.flow.description")}</CardDescription>
      </CardHeader>
      <CardContent className="signal-flow px-4 pb-5 pt-1">
        <section aria-label={t("dashboard.flow.sources")}>
          <h3 className="mb-2 flex items-center gap-2 px-3 text-xs font-semibold text-muted-foreground">
            <ArrowDownToLine className="h-3.5 w-3.5" aria-hidden="true" />{t("dashboard.flow.sources")}
          </h3>
          <ul>{sources.map(renderPort)}</ul>
        </section>
        <div className="signal-junction" aria-live="polite">
          <div className="grid h-14 w-14 place-items-center rounded-2xl bg-accent">
            <ArrowRightLeft className="h-6 w-6 text-primary" strokeWidth={1.75} aria-hidden="true" />
          </div>
          <p className="text-sm font-semibold">{t("dashboard.flow.bridge")}</p>
          <Badge variant={status?.running ? "success" : "secondary"}>
            {status ? status.running ? t("common.running") : t("common.stopped") : t("common.unknown")}
          </Badge>
        </div>
        <section aria-label={t("dashboard.flow.destinations")}>
          <h3 className="mb-2 flex items-center gap-2 px-3 text-xs font-semibold text-muted-foreground">
            <ArrowUpFromLine className="h-3.5 w-3.5" aria-hidden="true" />{t("dashboard.flow.destinations")}
          </h3>
          <ul>{destinations.map(renderPort)}</ul>
        </section>
      </CardContent>
    </Card>
  );
}
