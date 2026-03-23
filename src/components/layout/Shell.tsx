import { Suspense, lazy, useEffect, useState } from "react";
import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";

const CommandPalette = lazy(() => import("./CommandPalette"));

export function Shell() {
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
        <main className="flex-1 overflow-auto scroll-smooth page-body">
          <Outlet />
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
