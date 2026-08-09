import { Suspense, lazy, useEffect, useState } from "react";
import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import { useI18n } from "../../providers/LanguageProvider";

const CommandPalette = lazy(() => import("./CommandPalette"));

export function Shell() {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [commandPaletteRequested, setCommandPaletteRequested] = useState(false);

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setCommandPaletteRequested(true);
        setOpen((open) => !open);
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
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="aurora -left-24 -top-24" />
        <div className="aurora aurora--alt right-[-8rem] top-6" />
        <div className="aurora aurora--muted left-28 -bottom-16" />
        <div className="grain" />
      </div>

      <Sidebar />
      <div className="flex-1 flex flex-col h-full overflow-hidden relative">
        <TopBar
          onOpenCommand={() => {
            setCommandPaletteRequested(true);
            setOpen(true);
          }}
        />
        <main id="main-content" className="flex-1 overflow-auto scroll-smooth page-body" tabIndex={-1}>
          <div className="page-content">
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
