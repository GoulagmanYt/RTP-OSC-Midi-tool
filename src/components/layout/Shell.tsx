import { Suspense, lazy, useEffect, useRef, useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import { useI18n } from "../../providers/LanguageProvider";
import { useBridge } from "../../providers/BridgeProvider";
import { pageKeys } from "./pageMeta";

const CommandPalette = lazy(() => import("./CommandPalette"));

export function Shell() {
  const { t } = useI18n();
  const location = useLocation();
  const navigate = useNavigate();
  const { toggleBridge, isLoading, config } = useBridge();
  const shortcutState = useRef({ navigate, toggleBridge, isLoading, config });
  shortcutState.current = { navigate, toggleBridge, isLoading, config };
  const pageKey = pageKeys[location.pathname] ?? "dashboard";
  const [open, setOpen] = useState(false);
  const [sidebarVisible, setSidebarVisible] = useState(true);
  const [commandPaletteRequested, setCommandPaletteRequested] = useState(false);

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !e.ctrlKey || e.altKey || e.shiftKey || e.repeat || e.isComposing) return;
      const key = e.key.toLowerCase();
      const modal = document.querySelector('[role="dialog"]');
      if (key === "k") {
        e.preventDefault();
        setCommandPaletteRequested(true);
        setOpen((open) => !open);
        return;
      }
      if (modal || document.querySelector('[role="listbox"], [role="menu"], [role="alertdialog"]')) return;
      if (key === ",") {
        e.preventDefault();
        shortcutState.current.navigate("/settings");
        return;
      }
      if (e.target instanceof Element && e.target.closest('input, textarea, select, [role="combobox"], [role="slider"], [role="spinbutton"], [role="textbox"], [contenteditable="true"]')) return;
      if (key === "b") {
        e.preventDefault();
        setSidebarVisible(visible => !visible);
      }
      if (key === "enter" && !shortcutState.current.isLoading && shortcutState.current.config) {
        e.preventDefault();
        void shortcutState.current.toggleBridge();
      }
    };
    document.addEventListener("keydown", down);
    return () => document.removeEventListener("keydown", down);
  }, []);

  return (
    <div className="page-shell relative flex h-screen w-screen overflow-hidden bg-background text-foreground">
      <a className="skip-link" href="#main-content">
        {t("common.skipToContent")}
      </a>
      {sidebarVisible && <Sidebar />}
      <div className="relative flex h-full min-w-0 flex-1 flex-col overflow-hidden">
        <TopBar
          sidebarVisible={sidebarVisible}
          onToggleSidebar={() => setSidebarVisible(visible => !visible)}
          onOpenCommand={() => {
            setCommandPaletteRequested(true);
            setOpen(true);
          }}
        />
        <main id="main-content" className={`page-body flex-1 overflow-auto ${location.pathname === "/" ? "dashboard-page" : ""}`} tabIndex={-1}>
          <div className="page-content">
            <div className="page-heading mb-7 space-y-1.5">
              <h1 className="text-[1.75rem] font-semibold tracking-tight">{t(`topBar.${pageKey}`)}</h1>
              <p className="text-sm leading-relaxed text-muted-foreground">{t(`pageDescriptions.${pageKey}`)}</p>
            </div>
            <Outlet />
          </div>
        </main>
      </div>

      {commandPaletteRequested ? (
        <Suspense fallback={null}>
          <CommandPalette open={open} onOpenChange={setOpen} />
        </Suspense>
      ) : null}
    </div>
  );
}
