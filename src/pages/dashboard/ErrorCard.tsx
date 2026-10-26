import { AlertCircle } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { TranslateFn } from "../audio/shared";

type Props = {
  error: string;
  developerMode?: boolean;
  t: TranslateFn;
};

export function ErrorCard({ error, developerMode = false, t }: Props) {
  return (
    <Card className="border-destructive" role="alert">
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-lg text-destructive">
          <AlertCircle className="h-5 w-5" /> {t("dashboard.errorDetected")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <p className="text-sm text-foreground">{t("dashboard.errorRecoveryHint")}</p>
        {developerMode && <p className="mt-3 break-words font-mono text-sm text-foreground">{error}</p>}
      </CardContent>
    </Card>
  );
}
