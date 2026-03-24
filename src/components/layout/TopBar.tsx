import { useLocation } from "react-router-dom";
import { Search, Sun, Moon } from "lucide-react";
import { Button } from "../ui/Button";
import { useBridge } from "../../providers/BridgeProvider";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import Pastille from "../../assets/pastille-96.jpg";
import { useI18n } from "../../providers/LanguageProvider";

let _appWindow: ReturnType<typeof getCurrentWebviewWindow> | null = null;
function getAppWindow() {
  if (!_appWindow) _appWindow = getCurrentWebviewWindow();
  return _appWindow;
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
    const newTheme = config?.theme === "dark" ? "light" : "dark";
    // We update config, but the actual theme switching logic (CSS class) 
    // should be handled by a useEffect in App.tsx observing config.theme
    const nextPalette =
      newTheme === "dark"
        ? config?.themePalette === "contrast"
          ? "contrast"
          : "dark"
        : "light";
    await updateConfig({ theme: newTheme, themePalette: nextPalette });
  };

  // Use a fixed, packaged image for the avatar.
  return (
    <header 
      data-tauri-drag-region
      className="h-12 border-b flex items-center justify-between px-4 glass-surface backdrop-blur-xl select-none"
    >
      {/* Traffic Lights Area */}
      <div className="flex items-center gap-2 w-20 z-50">
          <div className="flex gap-2">
            <button 
              aria-label={t("window.close")}
              onClick={() => getAppWindow().close()}
              className="w-3 h-3 rounded-full bg-red-500 hover:bg-red-600 transition-colors" 
            />
            <button 
              aria-label={t("window.minimize")}
              onClick={() => getAppWindow().minimize()}
              className="w-3 h-3 rounded-full bg-yellow-500 hover:bg-yellow-600 transition-colors" 
            />
            <button 
              aria-label={t("window.maximize")}
              onClick={() => getAppWindow().toggleMaximize()}
              className="w-3 h-3 rounded-full bg-green-500 hover:bg-green-600 transition-colors" 
            />
         </div>
      </div>

      {/* Center Title */}
      <div className="flex-1 flex justify-center items-center font-medium text-sm text-foreground/80" data-tauri-drag-region>
         {getTitle()}
      </div>

      {/* Right Actions */}
      <div className="flex items-center gap-2 w-auto justify-end">
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={onOpenCommand}>
          <Search className="w-4 h-4 text-muted-foreground" />
        </Button>
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={toggleTheme}>
          {config?.theme === "dark" ? (
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
          className="w-8 h-8 rounded-full border overflow-hidden bg-primary/10 flex items-center justify-center text-xs font-bold text-primary"
          title={t("topBar.avatar")}
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
