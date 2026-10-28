import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Bug, Copy, Info, Pause, Play, Trash2, Download, XCircle } from "lucide-react";
import { toast } from "sonner";
import type { LogEntry } from "../api";
import { useBridge } from "../providers/BridgeProvider";
import { useI18n } from "../providers/LanguageProvider";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { useConfirmAction } from "../components/ui/useConfirmAction";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "../components/ui/dialog";

const ids = new WeakMap<LogEntry, number>();
let nextId = 0;
function idFor(entry: LogEntry) {
  let id = ids.get(entry);
  if (id === undefined) { id = ++nextId; ids.set(entry, id); }
  return id;
}
const icons = { info: Info, warn: AlertTriangle, error: XCircle, debug: Bug };
const format = (entries: LogEntry[]) => entries.map(entry => `${entry.timestamp} [${entry.level}] ${entry.message}`).join("\n");

export default function LogsPage() {
  const { logs, clearLogs } = useBridge();
  const { t } = useI18n();
  const { confirmAction, confirmation } = useConfirmAction(t);
  const [snapshot, setSnapshot] = useState<LogEntry[] | null>(null);
  const [follow, setFollow] = useState(true);
  const [query, setQuery] = useState("");
  const [level, setLevel] = useState("all");
  const [selection, setSelection] = useState<Map<number, LogEntry>>(new Map());
  const [detail, setDetail] = useState<LogEntry | null>(null);
  const [viewport, setViewport] = useState({ top: 0, height: 400, row: 48 });
  const viewportRef = useRef<HTMLDivElement>(null);
  const source = snapshot ?? logs;
  const filtered = useMemo(() => {
    const needle = query.toLocaleLowerCase();
    return source.filter(entry => (level === "all" || entry.level === level)
      && `${entry.timestamp} ${entry.message}`.toLocaleLowerCase().includes(needle));
  }, [source, level, query]);
  const start = Math.max(0, Math.min(Math.floor(viewport.top / viewport.row) - 5, filtered.length - 1));
  const end = Math.min(filtered.length, start + Math.ceil(viewport.height / viewport.row) + 10);

  useEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const resize = () => setViewport(previous => ({
      ...previous, height: element.clientHeight || 400,
      row: (parseFloat(getComputedStyle(element).fontSize) || 12) * 4,
    }));
    resize();
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(resize) : null;
    observer?.observe(element);
    return () => observer?.disconnect();
  }, []);
  useEffect(() => {
    if (follow && snapshot === null && viewportRef.current) {
      viewportRef.current.scrollTop = 0;
      setViewport(previous => previous.top === 0 ? previous : { ...previous, top: 0 });
    }
  }, [source, follow, snapshot]);
  useEffect(() => {
    if (viewportRef.current) viewportRef.current.scrollTop = 0;
    setViewport(previous => ({ ...previous, top: 0 }));
  }, [level, query]);

  const copy = async (entries: LogEntry[]) => {
    try { await navigator.clipboard.writeText(format(entries)); toast.success(t("ui.logs.copied")); }
    catch { toast.error(t("ui.copyFailed")); }
  };
  const exportLogs = () => {
    try {
      const url = URL.createObjectURL(new Blob([format(filtered)], { type: "text/plain;charset=utf-8" }));
      const link = document.createElement("a");
      link.href = url; link.download = "OSCMidi-logs.txt"; link.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch { toast.error(t("ui.exportFailed")); }
  };
  const toggleSelection = (entry: LogEntry) => {
    // Freeze the displayed entries so selection never shifts under the pointer.
    if (snapshot === null) setSnapshot(logs);
    setSelection(previous => {
      const next = new Map(previous);
      const id = idFor(entry);
      if (next.has(id)) next.delete(id); else next.set(id, entry);
      return next;
    });
  };
  return <div className="space-y-4">
    {confirmation}
    <div className="flex flex-wrap items-end gap-3">
      <Button variant="outline" onClick={() => {
        setSnapshot(snapshot === null ? logs : null);
        setSelection(new Map());
      }}>
        {snapshot === null ? <Pause className="h-4 w-4" aria-hidden="true" /> : <Play className="h-4 w-4" aria-hidden="true" />}
        {t(snapshot === null ? "ui.logs.pause" : "ui.logs.resume")}
      </Button>
      <label className="flex min-h-9 items-center gap-2 text-sm">
        <input type="checkbox" className="h-7 w-7 accent-primary" checked={follow} onChange={event => setFollow(event.target.checked)} />
        {t("ui.logs.autoscroll")}
      </label>
      <label className="min-w-0 flex-1 basis-48 space-y-1 text-sm">
        <span>{t("ui.logs.search")}</span>
        <Input type="search" value={query} onChange={event => setQuery(event.target.value)} />
      </label>
      <label className="space-y-1 text-sm">
        <span className="block">{t("ui.logs.level")}</span>
        <select className="min-h-10 max-w-full rounded-lg border border-input bg-control px-3 text-sm" value={level} onChange={event => setLevel(event.target.value)}>
          <option value="all">{t("ui.logs.allLevels")}</option>
          {(["info", "warn", "error", "debug"] as const).map(value => <option key={value} value={value}>{t(`logs.level.${value}`)}</option>)}
        </select>
      </label>
    </div>
    <div className="flex flex-wrap gap-2">
      <Button variant="outline" onClick={() => void copy(Array.from(selection.values()))} disabled={selection.size === 0}><Copy className="h-4 w-4" aria-hidden="true" />{t("ui.logs.copySelection")}</Button>
      <Button variant="outline" onClick={exportLogs} disabled={filtered.length === 0}><Download className="h-4 w-4" aria-hidden="true" />{t("ui.logs.export")}</Button>
      <Button variant="outline" onClick={() => confirmAction(t("ui.clearLogView"), () => {
        clearLogs(); setSnapshot(null); setSelection(new Map());
      }, t("ui.logs.clearDescription"))}><Trash2 className="h-4 w-4" aria-hidden="true" />{t("ui.clearLogView")}</Button>
    </div>
    {snapshot !== null && <p role="status" className="text-sm text-muted-foreground">{t("ui.logs.paused")}</p>}
    <p id="log-selection-hint" className="text-sm text-muted-foreground">{t("ui.logs.selectHint")}</p>
    <div ref={viewportRef} role="log" aria-label={t("logs.entries")} aria-live="off" aria-describedby="log-selection-hint" tabIndex={0}
      className="relative h-[55vh] min-h-48 max-h-[34rem] overflow-auto rounded-xl border bg-card font-mono text-[0.75rem] tabular-nums"
      onScroll={event => {
        const top = event.currentTarget.scrollTop;
        setViewport(previous => ({ ...previous, top }));
        if (top > 0 && follow) setFollow(false);
      }}
      onKeyDown={event => {
        if (event.ctrlKey && event.key.toLowerCase() === "c" && selection.size > 0) {
          event.preventDefault(); void copy(Array.from(selection.values()));
        }
      }}>
      {filtered.length === 0 ? <p className="p-6 text-center text-sm text-muted-foreground">{t(logs.length === 0 ? "logs.none" : "ui.logs.empty")}</p> :
        <div style={{ height: filtered.length * viewport.row }} className="relative min-w-0">
          {filtered.slice(start, end).map((entry, offset) => {
            const id = idFor(entry);
            const Icon = icons[entry.level];
            return <div key={id} data-log-row className="absolute inset-x-0 flex min-w-0 items-center gap-2 border-b border-border px-2" style={{ top: (start + offset) * viewport.row, height: viewport.row }}>
              <input type="checkbox" className="h-7 w-7 shrink-0 accent-primary" aria-label={t("ui.logs.selectLine", { number: id })} checked={selection.has(id)} onChange={() => toggleSelection(entry)} />
              <span className="hidden w-28 shrink-0 truncate text-muted-foreground sm:block">{entry.timestamp}</span>
              <Badge variant={entry.level === "error" ? "destructive" : entry.level === "warn" ? "warning" : "outline"} className="shrink-0 px-2 font-mono text-[0.75rem]">
                <Icon className="h-3.5 w-3.5" aria-hidden="true" />{t(`logs.level.${entry.level}`)}
              </Badge>
              <button className="min-h-7 min-w-0 flex-1 truncate text-left" aria-label={`${t("ui.logs.details")}: ${entry.message}`} title={entry.message} onClick={() => setDetail(entry)}>{entry.message}</button>
              <Button variant="ghost" size="icon" className="h-7 w-7 shrink-0" aria-label={`${t("ui.logs.copyLine")}: ${entry.message}`} onClick={() => void copy([entry])}><Copy className="h-4 w-4" aria-hidden="true" /></Button>
            </div>;
          })}
        </div>}
    </div>
    <Dialog open={detail !== null} onOpenChange={open => { if (!open) setDetail(null); }}>
      <DialogContent>
        <DialogTitle>{t("ui.logs.details")}</DialogTitle>
        <DialogDescription className="break-all">{detail?.timestamp} · {detail && t(`logs.level.${detail.level}`)}</DialogDescription>
        <pre className="whitespace-pre-wrap break-all font-mono text-[0.8125rem]">{detail?.message}</pre>
        <Button onClick={() => detail && void copy([detail])}>{t("ui.logs.copyLine")}</Button>
      </DialogContent>
    </Dialog>
  </div>;
}
