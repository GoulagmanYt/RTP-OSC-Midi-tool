import { useLocation } from "react-router-dom";
import { Search, Sun, Moon } from "lucide-react";
import { Button } from "../ui/Button";
import { useBridge } from "../../providers/BridgeProvider";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import Pastille from "../../assets/pastille-96.jpg";
import { useI18n } from "../../providers/LanguageProvider";
import { toast } from "sonner";

let appWindow: ReturnType<typeof getCurrentWebviewWindow> | null = null;
function getAppWindow() {
  if (!appWindow) appWindow = getCurrentWebviewWindow();
  return appWindow;
}

export function TopBar({ onOpenCommand }: { onOpenCommand: () => void }) {
  const location = useLocation();
  const { config, updateConfig } = useBridge();
  const { t, language, setLanguage } = useI18n();

  const nextLanguage = language === "fr" ? "en" : "fr";
  const nextLanguageLabel = nextLanguage === "fr" ? t("language.french") : t("language.english");
  const switchLanguageLabel = t("language.switchTo", { language: nextLanguageLabel });

  const getTitle = () => {
    switch (location.pathname) {
      case "/":
        return t("topBar.dashboard");
      case "/routing":
        return t("topBar.routingMidi");
      case "/osc":
        return t("topBar.osc");
      case "/rtp":
        return t("topBar.rtpMidi");
      case "/audio":
        return t("topBar.audioVst");
      case "/logs":
        return t("topBar.logs");
      case "/settings":
        return t("topBar.settings");
      default:
        return t("topBar.defaultTitle");
    }
  };

  const toggleTheme = async () => {
    const currentTheme = config?.ui.theme ?? "light";
    const newTheme = currentTheme === "dark" ? "light" : "dark";
    const nextPalette =
      newTheme === "dark"
        ? config?.ui.themePalette === "contrast"
          ? "contrast"
          : "dark"
        : "light";
    try {
      await updateConfig({ ui: { theme: newTheme, themePalette: nextPalette } });
    } catch (error) {
      console.error("Theme update failed", error);
      toast.error(t("toasts.settings.themeUpdateFailed"));
    }
  };

  const runWindowAction = (action: () => Promise<void>) => {
    void action().catch((error) => {
      console.error("Window action failed", error);
      toast.error(t("toasts.window.actionFailed"));
    });
  };

  return (
    <header
      data-tauri-drag-region
      className="grid h-13 grid-cols-[1fr_auto_1fr] items-center border-b px-3 glass-surface backdrop-blur-xl select-none"
    >
      <div data-tauri-drag-region className="z-10 flex items-center justify-self-start">
        <div data-tauri-drag-region className="flex items-center gap-0.5">
          <button
            type="button"
            aria-label={t("window.close")}
            title={t("window.close")}
            onClick={() => runWindowAction(() => getAppWindow().close())}
            className="group grid h-6 w-6 place-items-center rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="h-3 w-3 rounded-full bg-red-500 transition-transform group-hover:scale-110" />
          </button>
          <button
            type="button"
            aria-label={t("window.minimize")}
            title={t("window.minimize")}
            onClick={() => runWindowAction(() => getAppWindow().minimize())}
            className="group grid h-6 w-6 place-items-center rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="h-3 w-3 rounded-full bg-yellow-500 transition-transform group-hover:scale-110" />
          </button>
          <button
            type="button"
            aria-label={t("window.maximize")}
            title={t("window.maximize")}
            onClick={() => runWindowAction(() => getAppWindow().toggleMaximize())}
            className="group grid h-6 w-6 place-items-center rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="h-3 w-3 rounded-full bg-green-500 transition-transform group-hover:scale-110" />
          </button>
        </div>
      </div>

      <div className="text-sm font-medium text-foreground/80" data-tauri-drag-region>
        {getTitle()}
      </div>

      <div data-tauri-drag-region className="flex items-center justify-self-end gap-1">
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={onOpenCommand}
          aria-label={t("topBar.openCommandPalette")}
          title={t("topBar.openCommandPalette")}
        >
          <Search className="w-4 h-4 text-muted-foreground" />
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={toggleTheme}
          aria-label={t("topBar.toggleTheme")}
          title={t("topBar.toggleTheme")}
        >
          {config?.ui.theme === "dark" ? (
            <Sun className="w-4 h-4 text-muted-foreground" />
          ) : (
            <Moon className="w-4 h-4 text-muted-foreground" />
          )}
        </Button>
        <Button
          variant="ghost"
          className="h-8 px-2 text-xs font-semibold"
          onClick={() => setLanguage(nextLanguage)}
          aria-label={switchLanguageLabel}
          title={switchLanguageLabel}
        >
          {t(`language.short.${language}`)}
        </Button>
        <div
          role="img"
          aria-label={t("topBar.avatar")}
          className="w-8 h-8 rounded-full border overflow-hidden bg-primary/10 flex items-center justify-center text-xs font-bold text-primary"
          style={{
            backgroundImage: `url(${Pastille})`,
            backgroundSize: "cover",
            backgroundPosition: "center",
            backgroundRepeat: "no-repeat",
          }}
        />
      </div>
    </header>
  );
}
