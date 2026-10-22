import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { TranslateFn } from "./shared";

type Props = {
  t: TranslateFn;
};

export function RtpAboutCard({ t }: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">{t("rtp.about")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-sm text-muted-foreground">
        <p>{t("rtp.aboutText")}</p>
        <div className="rounded-lg border border-border/60 bg-background p-3">
          <p className="mb-2 text-xs font-semibold">{t("rtp.defaultPorts")}</p>
          <ul className="space-y-1 text-xs">
            <li>
              <code className="rounded bg-secondary px-1.5 py-0.5">5004</code> -{" "}
              {t("rtp.controlPort")}
            </li>
            <li>
              <code className="rounded bg-secondary px-1.5 py-0.5">5005</code> -{" "}
              {t("rtp.dataPort")}
            </li>
          </ul>
        </div>
      </CardContent>
    </Card>
  );
}
