import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useBridge } from "../providers/BridgeProvider";
import { pingAudio, reloadVst } from "../api";
import { toast } from "sonner";
import { useI18n } from "../providers/LanguageProvider";
import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../constants";
import { DiagnosticsGrid } from "./dashboard/DiagnosticsGrid";
import { ErrorCard } from "./dashboard/ErrorCard";
import { OverviewGrid } from "./dashboard/OverviewGrid";
import { getErrorMessage } from "../api-errors";

export default function Dashboard() {
  const navigate = useNavigate();
  const { status, config, metrics, toggleBridge, logs, preflight, runPreflight, refreshStatus } =
    useBridge();
  const { t } = useI18n();
  const [lastDropoutAt, setLastDropoutAt] = useState<number | null>(null);
  const lastXrun = useRef<number | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const rtpPort = status?.rtpBoundPort ?? config?.rtp.port;
  const audioBackend = status?.audioBackend || config?.audio.backend || t("common.auto");
  const audioDevice = status?.audioDevice || config?.audio.device || t("dashboard.systemDefault");
  const vstPath = config?.audio.vstPath || t("dashboard.bundledVstDefault");
  const shortVstPath = vstPath.length > 42 ? `...${vstPath.slice(vstPath.length - 42)}` : vstPath;
  const bufferSamples = status?.audioBufferSize ?? config?.audio.bufferSize ?? undefined;
  const requestedBufferSamples =
    status?.audioRequestedBufferSize ?? config?.audio.bufferSize ?? undefined;
  const sampleRate = status?.audioSampleRate ?? config?.audio.sampleRate ?? undefined;
  const streamBufferSamples = status?.audioStreamBufferSize ?? undefined;
  const bufferMismatch =
    status?.audioBufferMismatch ??
    (bufferSamples && requestedBufferSamples ? bufferSamples !== requestedBufferSamples : false);
  const vstMidiCompatible = status?.vstMidiCompatible ?? null;
  const limiterEnabled = status?.audioLimiterEnabled ?? config?.audio.limiterEnabled ?? false;
  const xrunCount = metrics?.audioXruns ?? status?.audioXruns ?? null;
  const audioMidiDrops = metrics?.audioMidiDrops ?? status?.audioMidiDrops ?? null;
  const audioLockMisses = metrics?.audioLockMisses ?? status?.audioLockMisses ?? null;
  const audioEmergencyResets = metrics?.audioEmergencyResets ?? status?.audioEmergencyResets ?? null;
  const audioCallbackMaxUs = metrics?.audioCallbackMaxUs ?? status?.audioCallbackMaxUs ?? null;
  const audioCallbackOverBudgetCount =
    metrics?.audioCallbackOverBudgetCount ?? status?.audioCallbackOverBudgetCount ?? null;
  const audioMmcssEnabled = metrics?.audioMmcssEnabled ?? status?.audioMmcssEnabled ?? null;
  const audioPowerThrottlingDisabled =
    metrics?.audioPowerThrottlingDisabled ?? status?.audioPowerThrottlingDisabled ?? null;
  const latencyMs =
    metrics?.audioLatencyMs ??
    status?.audioLatencyMs ??
    (bufferSamples && sampleRate ? Number(((bufferSamples / sampleRate) * 1000 * 2).toFixed(2)) : null);
  const audioPeakL = metrics?.audioPeakL ?? null;
  const audioPeakR = metrics?.audioPeakR ?? null;
  const midiRate = metrics ? metrics.midiMessagesPerSec : null;
  const oscRate = metrics ? metrics.oscMessagesPerSec : null;
  const dropoutsActive = lastDropoutAt !== null && now - lastDropoutAt < 30_000;
  const latencyBadge: "secondary" | "warning" | "success" =
    latencyMs === null ? "secondary" : latencyMs > 40 ? "warning" : "success";
  const latencyDisplay = latencyMs !== null ? Number(latencyMs.toFixed(1)) : null;

  useEffect(() => {
    const healthTotal =
      (metrics?.audioXruns ?? 0) +
      (metrics?.audioMidiDrops ?? 0) +
      (metrics?.audioLockMisses ?? 0) +
      (metrics?.audioEmergencyResets ?? 0) +
      (metrics?.audioCallbackOverBudgetCount ?? 0);
    if (!metrics || healthTotal === 0) {
      return;
    }
    const current = healthTotal;
    const previous = lastXrun.current;
    lastXrun.current = current;
    if (previous !== null && current > previous) {
      setLastDropoutAt(Date.now());
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
    const id = window.setInterval(() => setNow(Date.now()), 5000);
    return () => window.clearInterval(id);
  }, [lastDropoutAt]);

  const logPreview = logs.slice(0, 3);

  const displayPortName = (name: string) => {
    if (name === RTP_VIRTUAL_INPUT) return t("devices.rtpVirtualInput");
    if (name === VST_OUTPUT) return t("devices.vstInternalOutput");
    return name;
  };

  const handlePing = async () => {
    try {
      await pingAudio();
      toast.success(t("toasts.audio.pingSent"));
      refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.pingFailed"));
    }
  };

  const handleReloadVst = async () => {
    try {
      await reloadVst();
      toast.success(t("toasts.audio.vstReloaded"));
      refreshStatus();
    } catch (e) {
      const message = getErrorMessage(e);
      toast.error(message || t("toasts.audio.vstReloadFailed"));
    }
  };

  return (
    <div className="space-y-6 min-w-0">
      <OverviewGrid
        audioBackend={audioBackend}
        audioDevice={audioDevice}
        bufferSamples={bufferSamples}
        config={config}
        limiterEnabled={limiterEnabled}
        logPreview={logPreview}
        rtpPort={rtpPort}
        sampleRate={sampleRate}
        status={status}
        t={t}
        xrunCount={xrunCount}
        onNavigateLogs={() => navigate("/logs")}
        onToggleBridge={toggleBridge}
      />
      {config?.ui.developerMode && (
        <DiagnosticsGrid
          audioPeakL={audioPeakL}
          audioPeakR={audioPeakR}
          bufferMismatch={bufferMismatch}
          bufferSamples={bufferSamples}
          config={config}
          displayPortName={displayPortName}
          dropoutsActive={dropoutsActive}
          latencyBadge={latencyBadge}
          latencyDisplay={latencyDisplay}
          midiRate={midiRate}
          oscRate={oscRate}
          audioMidiDrops={audioMidiDrops}
          audioLockMisses={audioLockMisses}
          audioEmergencyResets={audioEmergencyResets}
          audioCallbackMaxUs={audioCallbackMaxUs}
          audioCallbackOverBudgetCount={audioCallbackOverBudgetCount}
          audioMmcssEnabled={audioMmcssEnabled}
          audioPowerThrottlingDisabled={audioPowerThrottlingDisabled}
          preflight={preflight}
          requestedBufferSamples={requestedBufferSamples}
          sampleRate={sampleRate}
          status={status}
          streamBufferSamples={streamBufferSamples}
          t={t}
          vstMidiCompatible={vstMidiCompatible}
          vstPath={vstPath}
          shortVstPath={shortVstPath}
          xrunCount={xrunCount}
          onHandlePing={handlePing}
          onHandleReloadVst={handleReloadVst}
          onRunPreflight={runPreflight}
        />
      )}
      {status?.lastError ? <ErrorCard error={status.lastError} t={t} /> : null}
    </div>
  );
}
