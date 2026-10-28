import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useBridge } from "../providers/BridgeProvider";
import { pingAudio, reloadVst } from "../api";
import { toast } from "sonner";
import { useI18n } from "../providers/LanguageProvider";
import { DiagnosticsGrid } from "./dashboard/DiagnosticsGrid";
import { ErrorCard } from "./dashboard/ErrorCard";
import { OverviewGrid } from "./dashboard/OverviewGrid";
import { SignalFlow } from "./dashboard/SignalFlow";
import { getErrorMessage } from "../api-errors";
import { isDeveloperMode } from "../developer-mode";
import { mergeLiveAudioMetrics } from "./audio/liveStatus";
import { getAudioLatency } from "./audio/latency";

export default function Dashboard() {
  const navigate = useNavigate();
  const { status: statusSnapshot, config, metrics, logs, preflight, runPreflight, refreshStatus } =
    useBridge();
  const status = mergeLiveAudioMetrics(statusSnapshot, metrics);
  const { t } = useI18n();
  const developerMode = isDeveloperMode(config);
  const [lastDropoutAt, setLastDropoutAt] = useState<number | null>(null);
  const lastXrun = useRef<number | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const rtpPort = status?.rtpBoundPort ?? config?.rtp.port;
  const audioBackend = status?.audioBackend || config?.audio.backend || t("common.auto");
  const audioDevice = status?.audioDevice || config?.audio.device || t("dashboard.systemDefault");
  const vstPath = config?.audio.vstPath || t("dashboard.bundledVstDefault");
  const bufferSamples = status?.audioBufferSize ?? config?.audio.bufferSize ?? undefined;
  const requestedBufferSamples =
    status?.audioRequestedBufferSize ?? config?.audio.bufferSize ?? undefined;
  const sampleRate = status?.audioSampleRate ?? config?.audio.sampleRate ?? undefined;
  const streamBufferSamples = status?.audioStreamBufferSize ?? undefined;
  const bufferMismatch =
    status?.audioBufferMismatch ??
    (bufferSamples && requestedBufferSamples ? bufferSamples !== requestedBufferSamples : false);
  const limiterEnabled = status?.audioLimiterEnabled ?? config?.audio.limiterEnabled ?? false;
  const latency = getAudioLatency(status, config);
  const dropoutsActive = lastDropoutAt !== null && now - lastDropoutAt < 30_000;

  useEffect(() => {
    const healthTotal =
      (metrics?.audioXruns ?? 0) +
      (metrics?.audioMidiDrops ?? 0) +
      (metrics?.audioLockMisses ?? 0) +
      (metrics?.audioEmergencyResets ?? 0) +
      (metrics?.audioCallbackOverBudgetCount ?? 0);
    if (!metrics) {
      lastXrun.current = null;
      setLastDropoutAt(null);
      return;
    }
    const current = healthTotal;
    const previous = lastXrun.current;
    lastXrun.current = current;
    if (previous !== null && current > previous) {
      setLastDropoutAt(Date.now());
    } else if (previous !== null && current < previous) {
      setLastDropoutAt(null);
    }
  }, [
    metrics,
    metrics?.audioCallbackOverBudgetCount,
    metrics?.audioEmergencyResets,
    metrics?.audioLockMisses,
    metrics?.audioMidiDrops,
    metrics?.audioXruns,
  ]);

  useEffect(() => {
    if (lastDropoutAt === null) return;
    const id = window.setInterval(() => {
      const currentNow = Date.now();
      setNow(currentNow);
      if (currentNow - lastDropoutAt >= 30_000) {
        window.clearInterval(id);
      }
    }, 5000);
    return () => window.clearInterval(id);
  }, [lastDropoutAt]);

  const handlePing = async () => {
    try {
      await pingAudio();
      toast.success(t("toasts.audio.pingSent"));
      await refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.pingFailed"));
    }
  };

  const handleReloadVst = async () => {
    try {
      await reloadVst();
      toast.success(t("toasts.audio.vstReloaded"));
      await refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstReloadFailed"));
    }
  };

  return (
    <div className={`dashboard-layout min-w-0 ${developerMode ? "dashboard-layout--developer" : "dashboard-layout--standard"}`}>
      {status?.lastError ? <ErrorCard error={status.lastError} developerMode={developerMode} t={t} /> : null}
      <div className="dashboard-primary">
        <SignalFlow config={config} status={status} t={t} />
        <OverviewGrid
          audioDevice={audioDevice}
          config={config}
          limiterEnabled={limiterEnabled}
          logs={logs}
          rtpPort={rtpPort}
          status={status}
          t={t}
          onNavigateLogs={() => navigate("/logs")}
        />
      </div>
      {developerMode && (
        <DiagnosticsGrid
          audioBackend={audioBackend}
          bufferMismatch={bufferMismatch}
          bufferSamples={bufferSamples}
          config={config}
          dropoutsActive={dropoutsActive}
          latency={latency}
          metrics={metrics}
          preflight={preflight}
          requestedBufferSamples={requestedBufferSamples}
          sampleRate={sampleRate}
          status={status}
          streamBufferSamples={streamBufferSamples}
          t={t}
          vstPath={vstPath}
          onHandlePing={handlePing}
          onHandleReloadVst={handleReloadVst}
          onRunPreflight={runPreflight}
        />
      )}
    </div>
  );
}
