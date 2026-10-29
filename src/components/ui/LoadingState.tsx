import { LoaderCircle } from "lucide-react";
import { useI18n } from "../../providers/LanguageProvider";

export function LoadingState() {
  const { t } = useI18n();
  return (
    <div className="flex min-h-[40vh] flex-col items-center justify-center gap-3 text-sm text-muted-foreground" role="status" aria-live="polite">
      <LoaderCircle className="h-5 w-5 motion-safe:animate-spin" strokeWidth={1.75} aria-hidden="true" />
      <span>{t("common.loading")}</span>
    </div>
  );
}
