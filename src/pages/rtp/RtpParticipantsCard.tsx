import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { RtpParticipantInfo } from "../../api-types";
import type { TranslateFn } from "./shared";
import { Server } from "lucide-react";

type Props = {
  participants: RtpParticipantInfo[];
  t: TranslateFn;
};

export function RtpParticipantsCard({ participants, t }: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <Server className="h-4 w-4" />
          {t("rtp.connectedTitle")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {participants.length === 0 ? (
          <div className="text-xs text-muted-foreground">{t("rtp.connectedEmpty")}</div>
        ) : (
          participants.map((participant) => (
            <div
              key={`${participant.addr}-${participant.name}`}
              className="flex items-center justify-between rounded-md border border-slate-100 bg-white/70 px-3 py-2 dark:border-slate-800 dark:bg-slate-950/40"
            >
              <div className="text-sm font-medium">{participant.name}</div>
              <code className="rounded bg-slate-200 px-1.5 py-0.5 font-mono text-xs dark:bg-slate-700">
                {participant.addr}
              </code>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}
