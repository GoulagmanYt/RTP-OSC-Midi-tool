import { useBridge } from "../providers/BridgeProvider";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Trash2 } from "lucide-react";
import { cn } from "../utils";
import { useI18n } from "../providers/LanguageProvider";

export default function LogsPage() {
  const { logs, clearLogs } = useBridge();
  const { t } = useI18n();

  return (
    <div className="h-full flex flex-col space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-2xl font-bold tracking-tight">{t("logs.title")}</h2>
        <Button variant="outline" size="sm" onClick={clearLogs}>
          <Trash2 className="mr-2 h-4 w-4" />
          {t("logs.clear")}
        </Button>
      </div>

      <div className="flex-1 overflow-hidden rounded-xl border bg-card text-card-foreground shadow-sm">
        <div className="h-full space-y-2 overflow-auto p-4 font-mono text-xs">
          {logs.length === 0 && (
            <div className="py-10 text-center text-muted-foreground">{t("logs.none")}</div>
          )}
          {logs.map((log, index) => (
            <div
              key={`${log.timestamp}-${index}`}
              className="flex items-start gap-3 border-b border-border/40 pb-2 last:border-0 last:pb-0"
            >
              <span className="w-32 shrink-0 text-muted-foreground">{log.timestamp}</span>
              <Badge
                variant={log.level === "error" ? "destructive" : log.level === "warn" ? "warning" : "outline"}
                className="h-5 w-16 shrink-0 justify-center text-[10px] uppercase"
              >
                {t(`logs.level.${log.level}`)}
              </Badge>
              <span
                className={cn(
                  "break-all",
                  log.level === "error"
                    ? "text-red-500"
                    : log.level === "warn"
                      ? "text-yellow-500"
                      : "text-foreground",
                )}
              >
                {log.message}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
