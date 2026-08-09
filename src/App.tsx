import { Suspense, lazy, useEffect } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { BridgeProvider, useBridge } from "./providers/BridgeProvider";
import { Shell } from "./components/layout/Shell";
import { Toaster } from "sonner";
import { applyTheme } from "./themes";
import { Button } from "./components/ui/Button";
import { AppErrorBoundary } from "./components/AppErrorBoundary";
import { useI18n } from "./providers/LanguageProvider";

const Dashboard = lazy(() => import("./pages/Dashboard"));
const AudioPage = lazy(() => import("./pages/AudioPage"));
const LogsPage = lazy(() => import("./pages/LogsPage"));
const RoutingPage = lazy(() => import("./pages/RoutingPage"));
const OscPage = lazy(() => import("./pages/OscPage"));
const RtpPage = lazy(() => import("./pages/RtpPage"));
const SettingsPage = lazy(() => import("./pages/SettingsPage"));

function RouteFallback() {
  const { t } = useI18n();
  return (
    <div
      className="flex min-h-[40vh] items-center justify-center text-sm text-muted-foreground"
      role="status"
      aria-live="polite"
    >
      {t("common.loading")}
    </div>
  );
}

function RecoveryScreen({ message }: { message: string }) {
  const { t } = useI18n();

  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-8 text-foreground">
      <section className="w-full max-w-lg rounded-xl border bg-card p-8 shadow-sm" role="alert">
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-destructive">
          {t("recovery.eyebrow")}
        </p>
        <h1 className="mt-3 text-2xl font-semibold tracking-tight">{t("recovery.title")}</h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">{t("recovery.description")}</p>
        <pre className="mt-5 max-h-32 overflow-auto rounded-md border bg-muted/40 p-3 text-xs whitespace-pre-wrap">
          {message}
        </pre>
        <Button className="mt-6" onClick={() => window.location.reload()}>
          {t("recovery.reload")}
        </Button>
      </section>
    </main>
  );
}

function AppContent() {
  const { config, initializationError, isLoading } = useBridge();

  useEffect(() => {
    if (!config) return;
    const palette = config.ui.themePalette || (config.ui.theme === "dark" ? "dark" : "light");
    const radius = config.ui.cornerRadius ?? 12;
    applyTheme(palette, radius);
  }, [config]);

  if (isLoading) {
    return <RouteFallback />;
  }
  if (initializationError || !config) {
    return <RecoveryScreen message={initializationError || "Configuration unavailable"} />;
  }

  return (
    <Suspense fallback={<RouteFallback />}>
      <Routes>
        <Route element={<Shell />}>
          <Route path="/" element={<Dashboard />} />
          <Route path="/routing" element={<RoutingPage />} />
          <Route path="/osc" element={<OscPage />} />
          <Route path="/rtp" element={<RtpPage />} />
          <Route path="/audio" element={<AudioPage />} />
          <Route path="/logs" element={config?.ui.developerMode ? <LogsPage /> : <Navigate to="/" replace />} />
          <Route path="/settings" element={<SettingsPage />} />
        </Route>
      </Routes>
    </Suspense>
  );
}

function App() {
  const { t } = useI18n();

  return (
    <AppErrorBoundary fallback={(error) => <RecoveryScreen message={error.message || t("recovery.unknown")} />}>
      <BrowserRouter>
        <BridgeProvider>
          <AppContent />
          <Toaster closeButton richColors position="bottom-right" />
        </BridgeProvider>
      </BrowserRouter>
    </AppErrorBoundary>
  );
}

export default App;
