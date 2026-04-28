import { useState } from "react";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { runAutomatedStressTest, type StressTestResult, type StressTestMode } from "../../api";
import { Activity, CheckCircle, XCircle, Network, AudioWaveform, Layers, Workflow } from "lucide-react";

type Props = {
  bridgeRunning: boolean;
  audioRunning: boolean;
};

const TEST_MODES: { value: StressTestMode; label: string; icon: React.ReactNode; description: string }[] = [
  { value: "audio-vst", label: "Audio/VST Only", icon: <AudioWaveform className="h-4 w-4" />, description: "Test direct moteur audio (bypass bridge)" },
  { value: "bridge", label: "Bridge Pipeline", icon: <Workflow className="h-4 w-4" />, description: "Test routage et filtrage MIDI" },
  { value: "end-to-end", label: "End-to-End", icon: <Layers className="h-4 w-4" />, description: "Test complet de la pipeline" },
  { value: "rtp", label: "RTP Input", icon: <Network className="h-4 w-4" />, description: "Test entrée RTP-MIDI (externe)" },
];

export function AudioStressTestCard({ bridgeRunning, audioRunning }: Props) {
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<StressTestResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<StressTestMode>("end-to-end");

  const handleRunTest = async () => {
    setRunning(true);
    setResult(null);
    setError(null);
    try {
      // 50,000 messages over 5 seconds
      const res = await runAutomatedStressTest(50000, 5, mode);
      setResult(res);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to run stress test");
    } finally {
      setRunning(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Activity className="h-5 w-5 text-primary" />
          Diagnostic / Stress Test
        </CardTitle>
        <CardDescription>
          Automated load testing to verify VST and Audio Engine stability.
          Injects 50,000 notes per second for 5 seconds to detect dropouts or buffer underruns.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-3">
          <div className="flex flex-wrap gap-2">
            {TEST_MODES.map((m) => (
              <button
                key={m.value}
                onClick={() => setMode(m.value)}
                disabled={running}
                className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm font-medium transition-colors ${
                  mode === m.value
                    ? "bg-primary text-primary-foreground"
                    : "bg-muted hover:bg-muted/80 text-muted-foreground"
                } disabled:opacity-50`}
                title={m.description}
              >
                {m.icon}
                {m.label}
              </button>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            {TEST_MODES.find(m => m.value === mode)?.description}
          </p>
        </div>

        <div className="flex items-center gap-4">
          <Button
            onClick={handleRunTest}
            disabled={running || !audioRunning || !bridgeRunning}
            className="w-40"
          >
            {running ? "Testing..." : "Lancer le Test"}
          </Button>
          {!audioRunning && (
            <span className="text-sm text-amber-600">
              Le moteur audio doit être actif.
            </span>
          )}
        </div>

        {error && (
          <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-600 dark:border-red-900/50 dark:bg-red-900/20 dark:text-red-400">
            {error}
          </div>
        )}

        {result && (
          <div className="space-y-4 rounded-md border border-border/60 bg-muted/30 p-4">
            <div className="flex items-center justify-between">
              <h4 className="font-medium text-foreground">Résultats du Test ({result.elapsedMs} ms)</h4>
              <span className="text-xs text-muted-foreground">
                Mode: {TEST_MODES.find(m => m.value === mode)?.label}
              </span>
            </div>

            {/* Global Summary */}
            <div className="grid grid-cols-3 gap-4 text-sm border-b border-border/40 pb-3">
              <div className="flex flex-col">
                <span className="text-muted-foreground">Notes Envoyées</span>
                <span className="font-semibold">{result.sentNotes.toLocaleString()}</span>
              </div>
              <div className="flex flex-col">
                <span className="text-muted-foreground">Drops Total</span>
                <span className={`font-semibold flex items-center gap-1 ${result.droppedNotes > 0 ? "text-red-500" : "text-green-500"}`}>
                  {result.droppedNotes > 0 ? <XCircle className="h-4 w-4" /> : <CheckCircle className="h-4 w-4" />}
                  {result.droppedNotes.toLocaleString()}
                </span>
              </div>
              <div className="flex flex-col">
                <span className="text-muted-foreground">XRuns Total</span>
                <span className={`font-semibold flex items-center gap-1 ${result.xruns > 0 ? "text-red-500" : "text-green-500"}`}>
                  {result.xruns > 0 ? <XCircle className="h-4 w-4" /> : <CheckCircle className="h-4 w-4" />}
                  {result.xruns.toLocaleString()}
                </span>
              </div>
            </div>

            {/* Per-Segment Breakdown */}
            {(result.segmentRtp || result.segmentBridge || result.segmentAudio) && (
              <div className="space-y-2">
                <h5 className="text-xs font-medium text-muted-foreground uppercase tracking-wider">Détail par Segment</h5>
                <div className="grid grid-cols-3 gap-3 text-xs">
                  {/* RTP Segment */}
                  {result.segmentRtp && (
                    <div className={`p-2 rounded-md ${result.segmentRtp.dropped > 0 ? "bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-900/50" : "bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-900/50"}`}>
                      <div className="flex items-center gap-1 font-medium mb-1">
                        <Network className="h-3 w-3" />
                        RTP Input
                      </div>
                      <div className="space-y-0.5 text-muted-foreground">
                        <div>Drops: <span className={result.segmentRtp.dropped > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentRtp.dropped}</span></div>
                        {result.segmentRtp.latencyMs !== null && (
                          <div>Latence: {result.segmentRtp.latencyMs.toFixed(1)}ms</div>
                        )}
                      </div>
                    </div>
                  )}

                  {/* Bridge Segment */}
                  {result.segmentBridge && (
                    <div className={`p-2 rounded-md ${result.segmentBridge.dropped > 0 ? "bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-900/50" : "bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-900/50"}`}>
                      <div className="flex items-center gap-1 font-medium mb-1">
                        <Workflow className="h-3 w-3" />
                        Bridge
                      </div>
                      <div className="space-y-0.5 text-muted-foreground">
                        <div>Drops: <span className={result.segmentBridge.dropped > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentBridge.dropped}</span></div>
                        {result.receivedNotes !== undefined && (
                          <div>Reçus: {result.receivedNotes.toLocaleString()}</div>
                        )}
                      </div>
                    </div>
                  )}

                  {/* Audio Segment */}
                  {result.segmentAudio && (
                    <div className={`p-2 rounded-md ${(result.segmentAudio.dropped > 0 || result.segmentAudio.xruns > 0) ? "bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-900/50" : "bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-900/50"}`}>
                      <div className="flex items-center gap-1 font-medium mb-1">
                        <AudioWaveform className="h-3 w-3" />
                        Audio/VST
                      </div>
                      <div className="space-y-0.5 text-muted-foreground">
                        <div>Drops: <span className={result.segmentAudio.dropped > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentAudio.dropped}</span></div>
                        <div>XRuns: <span className={result.segmentAudio.xruns > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentAudio.xruns}</span></div>
                        {result.segmentAudio.latencyMs !== null && (
                          <div>Latence: {result.segmentAudio.latencyMs.toFixed(1)}ms</div>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Diagnostic Message */}
            {result.droppedNotes === 0 && result.xruns === 0 ? (
              <p className="text-xs text-green-600 dark:text-green-400">
                <CheckCircle className="h-3 w-3 inline mr-1" />
                Pipeline MIDI parfaitement stable sous charge extrême.
              </p>
            ) : (
              <div className="text-xs text-red-600 dark:text-red-400 space-y-1">
                <p className="font-medium"><XCircle className="h-3 w-3 inline mr-1" />Instabilité détectée :</p>
                {result.segmentRtp && result.segmentRtp.dropped > 0 && <p>• Segment RTP: pertes réseau ou buffer RTP plein</p>}
                {result.segmentBridge && result.segmentBridge.dropped > 0 && <p>• Segment Bridge: messages filtrés ou pipeline saturé</p>}
                {result.segmentAudio && result.segmentAudio.dropped > 0 && <p>• Segment Audio: buffer MIDI audio plein ou VST trop lent</p>}
                {result.segmentAudio && result.segmentAudio.xruns > 0 && <p>• XRuns: buffer audio trop petit ou VST trop lourd</p>}
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
