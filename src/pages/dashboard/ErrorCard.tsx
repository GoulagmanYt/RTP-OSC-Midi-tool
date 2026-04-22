import { AlertCircle } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/Card";
import type { TranslateFn } from "../audio/shared";

type Props = {
  error: string;
  t: TranslateFn;
};

export function ErrorCard({ error, t }: Props) {
  return (
    <Card className="border-destructive/50 bg-destructive/10">
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-lg text-destructive">
          <AlertCircle className="h-5 w-5" /> {t("dashboard.errorDetected")}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <p className="break-words font-mono text-sm text-destructive-foreground">{error}</p>
      </CardContent>
    </Card>
  );
}
