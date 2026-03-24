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
          <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" onClick={clearLogs}>
                  <Trash2 className="w-4 h-4 mr-2"/> {t("logs.clear")}
              </Button>
          </div>
      </div>

      <div className="flex-1 overflow-hidden rounded-xl border bg-card text-card-foreground shadow-sm">
          <div className="h-full overflow-auto p-4 space-y-2 font-mono text-xs">
              {logs.length === 0 && (
                  <div className="text-center text-muted-foreground py-10">{t("logs.none")}</div>
              )}
              {logs.map((log, i) => (
                  <div key={i} className="flex gap-3 items-start border-b border-border/40 pb-2 last:border-0 last:pb-0">
                      <span className="text-muted-foreground w-32 shrink-0">{log.timestamp}</span>
                      <Badge variant={
                          log.level === 'error' ? 'destructive' : 
                          log.level === 'warn' ? 'warning' : 'outline'
                      } className="uppercase w-16 justify-center shrink-0 text-[10px] h-5">
                          {t(`logs.level.${log.level}`)}
                      </Badge>
                      <span className={cn(
                          "break-all",
                          log.level === 'error' ? 'text-red-500' : 
                          log.level === 'warn' ? 'text-yellow-500' : 'text-foreground'
                      )}>
                          {log.message}
                      </span>
                  </div>
              ))}
          </div>
      </div>
    </div>
  );
}
