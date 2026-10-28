import { useEffect, useState } from "react";
import { Copy, Minus, Square, X } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { toast } from "sonner";
import { useI18n } from "../../providers/LanguageProvider";

export function WindowControls() {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    const appWindow = getCurrentWebviewWindow();
    let active = true;
    let release: (() => void) | undefined;
    const refresh = () => appWindow.isMaximized().then(value => {
      if (active) setMaximized(value);
    }).catch(error => console.error("Failed to read window state", error));
    void appWindow.onResized(() => void refresh()).then(unlisten => {
      if (active) release = unlisten;
      else unlisten();
    }).catch(error => console.error("Failed to observe window state", error));
    void refresh();
    return () => { active = false; release?.(); };
  }, []);

  const run = (action: () => Promise<void>) => {
    void Promise.resolve().then(action).catch(error => {
      console.error("Window action failed", error);
      toast.error(t("toasts.window.actionFailed"));
    });
  };
  const maximizeLabel = t(maximized ? "window.restore" : "window.maximize");

  return (
    <div className="window-controls ml-auto flex shrink-0 items-center gap-1" role="group" aria-label={t("window.controls")}>
      <button type="button" className="window-action" aria-label={t("window.minimize")} title={t("window.minimize")} onClick={() => run(() => getCurrentWebviewWindow().minimize())}>
        <Minus className="h-4 w-4" strokeWidth={1.75} aria-hidden="true" />
      </button>
      <button type="button" className="window-action" aria-label={maximizeLabel} title={maximizeLabel} onClick={() => run(async () => {
        const appWindow = getCurrentWebviewWindow();
        await appWindow.toggleMaximize();
        setMaximized(await appWindow.isMaximized());
      })}>
        {maximized ? <Copy className="h-[0.875rem] w-[0.875rem]" strokeWidth={1.75} aria-hidden="true" /> : <Square className="h-[0.875rem] w-[0.875rem]" strokeWidth={1.75} aria-hidden="true" />}
      </button>
      <button type="button" className="window-action window-action--close" aria-label={t("window.close")} title={`${t("window.close")} (Alt+F4)`} aria-keyshortcuts="Alt+F4" onClick={() => run(() => getCurrentWebviewWindow().close())}>
        <X className="h-[1.0625rem] w-[1.0625rem]" strokeWidth={1.75} aria-hidden="true" />
      </button>
    </div>
  );
}
