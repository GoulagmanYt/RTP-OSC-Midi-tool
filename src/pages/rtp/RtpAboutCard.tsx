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
        <div className="rounded-lg border border-slate-100 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/50">
          <p className="mb-2 text-xs font-semibold">{t("rtp.defaultPorts")}</p>
          <ul className="space-y-1 text-xs">
            <li>
              <code className="rounded bg-slate-100 px-1.5 py-0.5 dark:bg-slate-800">5004</code> -{" "}
              {t("rtp.controlPort")}
            </li>
            <li>
              <code className="rounded bg-slate-100 px-1.5 py-0.5 dark:bg-slate-800">5005</code> -{" "}
              {t("rtp.dataPort")}
            </li>
          </ul>
        </div>
      </CardContent>
    </Card>
  );
}
