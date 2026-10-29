import { LoaderCircle, PanelLeft, Play, Search, Square } from "lucide-react";
import { Button } from "../ui/Button";
import { useBridge } from "../../providers/BridgeProvider";
import { useI18n } from "../../providers/LanguageProvider";
import { isDeveloperMode } from "../../developer-mode";
import { WindowControls } from "./WindowControls";

type Props = { onOpenCommand: () => void; sidebarVisible: boolean; onToggleSidebar: () => void };

export function TopBar({ onOpenCommand, sidebarVisible, onToggleSidebar }: Props) {
  const { config, status, isLoading, bridgeTransition, toggleBridge, updateConfig } = useBridge();
  const { t, language, setLanguage } = useI18n();
  const nextLanguage = language === "fr" ? "en" : "fr";
  const nextLanguageLabel = nextLanguage === "fr" ? t("language.french") : t("language.english");
  const switchLanguageLabel = t("language.switchTo", { language: nextLanguageLabel });
  const bridgeBusy = isLoading || Boolean(bridgeTransition);
  const bridgeLabel = bridgeTransition ? t(`actions.${bridgeTransition}Bridge`)
    : isLoading ? t("common.loading")
    : status?.running ? t("actions.stopBridge") : t("actions.startBridge");
  const bridgeFeedback = bridgeBusy ? bridgeLabel
    : !status ? t("common.unknown")
    : t(status.running ? "topBar.bridgeRunning" : "topBar.bridgeStopped");

  return (
    <header data-tauri-drag-region className="toolbar-material relative z-10 flex min-h-18 shrink-0 flex-wrap items-center gap-3 px-4 select-none">
      <Button variant="ghost" size="icon" className="toolbar-icon h-9 w-9" onClick={onToggleSidebar} aria-label={t("topBar.toggleSidebar")} title={`${t(sidebarVisible ? "ui.hideSidebar" : "ui.showSidebar")} (Ctrl+B)`} aria-expanded={sidebarVisible} aria-controls="app-sidebar">
        <PanelLeft className="h-[1.125rem] w-[1.125rem]" strokeWidth={1.75} aria-hidden="true" />
      </Button>
      <div className="min-w-0 flex-1 self-stretch" data-tauri-drag-region />
      <div className="toolbar-utilities flex shrink-0 items-center gap-1.5">
        <Button variant="ghost" size="sm" data-command-exclude className="toolbar-search gap-2 px-3 text-muted-foreground" onClick={onOpenCommand} aria-label={t("topBar.openCommandPalette")} title={`${t("topBar.openCommandPalette")} (Ctrl+K)`}>
          <Search className="h-4 w-4" strokeWidth={1.75} aria-hidden="true" />
          <span className="toolbar-search-label">{t("topBar.search")}</span>
          <kbd className="toolbar-search-label ml-5 text-xs font-normal" aria-hidden="true">Ctrl K</kbd>
        </Button>
        <Button variant="ghost" className="toolbar-icon h-9 w-9 px-0 text-xs font-semibold" onClick={() => setLanguage(nextLanguage)} aria-label={switchLanguageLabel} title={switchLanguageLabel}>
          {t(`language.short.${language}`)}
        </Button>
      </div>
      <Button variant={bridgeTransition === "stopping" || (!bridgeTransition && status?.running) ? "secondary" : "default"} size="sm" className="bridge-action shrink-0 px-4" onClick={toggleBridge} disabled={bridgeBusy || !config || !status} aria-busy={bridgeBusy} aria-label={bridgeLabel} title={bridgeBusy ? bridgeLabel : `${bridgeLabel} (Ctrl+Enter)`}>
        <span className="flex h-4 w-4 items-center justify-center" aria-hidden="true">
          {bridgeBusy ? <LoaderCircle className="h-4 w-4 motion-safe:animate-spin" /> : status?.running ? <Square className="h-3 w-3 fill-current" /> : <Play className="h-3.5 w-3.5 fill-current" />}
        </span>
        <span className="bridge-action-label">{bridgeLabel}</span>
      </Button>
      <span role="status" aria-live="polite" aria-atomic="true" className="sr-only">{bridgeFeedback}</span>
      {isDeveloperMode(config) && <Button variant="outline" size="sm" onClick={() => updateConfig({ ui: { developerMode: false } })} aria-label={t("ui.disableDeveloper")} title={t("ui.disableDeveloper")}>{t("ui.developerActive")}</Button>}
      <WindowControls />
    </header>
  );
}
