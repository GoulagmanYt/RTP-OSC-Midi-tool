import { Suspense, lazy, useEffect } from "react";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { BridgeProvider, useBridge } from "./providers/BridgeProvider";
import { Shell } from "./components/layout/Shell";
import { Toaster } from "sonner";
import { applyTheme } from "./themes";

const Dashboard = lazy(() => import("./pages/Dashboard"));
const AudioPage = lazy(() => import("./pages/AudioPage"));
const LogsPage = lazy(() => import("./pages/LogsPage"));
const RoutingPage = lazy(() => import("./pages/RoutingPage"));
const OscPage = lazy(() => import("./pages/OscPage"));
const RtpPage = lazy(() => import("./pages/RtpPage"));
const SettingsPage = lazy(() => import("./pages/SettingsPage"));

function RouteFallback() {
  return (
    <div className="flex min-h-[40vh] items-center justify-center text-sm text-muted-foreground">
      Loading...
    </div>
  );
}

function AppContent() {
  const { config } = useBridge();

  useEffect(() => {
    if (!config) return;
    const palette = config.themePalette || (config.theme === "dark" ? "dark" : "light");
    const radius = config.cornerRadius ?? 12;
    applyTheme(palette, radius);
  }, [config]);

  // Handle generic loading state if needed, but Shell handles it gracefully via Sidebar
  return (
    <Suspense fallback={<RouteFallback />}>
      <Routes>
        <Route element={<Shell />}>
          <Route path="/" element={<Dashboard />} />
          <Route path="/routing" element={<RoutingPage />} />
          <Route path="/osc" element={<OscPage />} />
          <Route path="/rtp" element={<RtpPage />} />
          <Route path="/audio" element={<AudioPage />} />
          <Route path="/logs" element={<LogsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
        </Route>
      </Routes>
    </Suspense>
  );
}

function App() {
  return (
    <BrowserRouter>
      <BridgeProvider>
        <AppContent />
        <Toaster />
      </BridgeProvider>
    </BrowserRouter>
  );
}

export default App;
