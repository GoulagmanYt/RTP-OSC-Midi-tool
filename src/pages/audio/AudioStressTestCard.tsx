import { useState } from "react";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { runAutomatedStressTest, type StressTestResult, type StressTestMode } from "../../api";
import { Activity, CheckCircle, XCircle, AudioWaveform, Workflow } from "lucide-react";
import { getErrorMessage } from "../../api-errors";

const STRESS_RATE_MESSAGES_PER_SECOND = 5_000;
const STRESS_DURATION_SECONDS = 5;

type Props = {
  bridgeRunning: boolean;
  audioRunning: boolean;
  t: (key: string, vars?: Record<string, string | number>) => string;
};

const TEST_MODES = [
  { value: "audio-vst", labelKey: "stress.mode.audio", descriptionKey: "stress.mode.audioDescription", Icon: AudioWaveform },
  { value: "bridge", labelKey: "stress.mode.bridge", descriptionKey: "stress.mode.bridgeDescription", Icon: Workflow },
] satisfies Array<{
  value: StressTestMode;
  labelKey: string;
  descriptionKey: string;
  Icon: typeof AudioWaveform;
}>;

export function AudioStressTestCard({ bridgeRunning, audioRunning, t }: Props) {
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<StressTestResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<StressTestMode>("bridge");
  const requiresBridge = mode !== "audio-vst";

  const handleRunTest = async () => {
    setRunning(true);
    setResult(null);
    setError(null);
    try {
      const res = await runAutomatedStressTest(
        STRESS_RATE_MESSAGES_PER_SECOND,
        STRESS_DURATION_SECONDS,
        mode
      );
      setResult(res);
    } catch (err: unknown) {
      setError(getErrorMessage(err) || t("stress.runFailed"));
    } finally {
      setRunning(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Activity className="h-5 w-5 text-primary" />
          {t("stress.title")}
        </CardTitle>
        <CardDescription>
          {t("stress.description")}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-3">
          <div className="flex flex-wrap gap-2">
            {TEST_MODES.map(({ value, labelKey, descriptionKey, Icon }) => (
              <Button
                key={value}
                onClick={() => setMode(value)}
                disabled={running}
                variant={mode === value ? "default" : "secondary"}
                size="sm"
                aria-pressed={mode === value}
                title={t(descriptionKey)}
              >
                <Icon className="h-4 w-4" />
                {t(labelKey)}
              </Button>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            {t(TEST_MODES.find((item) => item.value === mode)?.descriptionKey ?? "")}
          </p>
        </div>

        <div className="flex items-center gap-4">
          <Button
            onClick={handleRunTest}
            disabled={running || !audioRunning || (requiresBridge && !bridgeRunning)}
            className="w-40"
          >
            {running ? t("stress.running") : t("stress.run")}
          </Button>
          {!audioRunning && (
            <span className="text-sm text-amber-600">
              {t("stress.audioRequired")}
            </span>
          )}
          {audioRunning && requiresBridge && !bridgeRunning && (
            <span className="text-sm text-amber-600">
              {t("stress.bridgeRequired")}
            </span>
          )}
        </div>

        {error && (
          <div className="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-600 dark:border-red-900/50 dark:bg-red-900/20 dark:text-red-400" role="alert">
            {error}
          </div>
        )}

        {result && (
          <div className="space-y-4 rounded-md border border-border/60 bg-muted/30 p-4" aria-live="polite">
            <div className="flex items-center justify-between">
              <h4 className="font-medium text-foreground">{t("stress.results", { duration: result.elapsedMs })}</h4>
              <span className="text-xs text-muted-foreground">
                {t("stress.modeLabel")}: {t(TEST_MODES.find((item) => item.value === mode)?.labelKey ?? "")}
              </span>
            </div>

            <div className="grid grid-cols-3 gap-4 text-sm border-b border-border/40 pb-3">
              <div className="flex flex-col">
                <span className="text-muted-foreground">{t("stress.sent")}</span>
                <span className="font-semibold">{result.sentNotes.toLocaleString()}</span>
              </div>
              <div className="flex flex-col">
                <span className="text-muted-foreground">{t("stress.drops")}</span>
                <span className={`font-semibold flex items-center gap-1 ${result.droppedNotes > 0 ? "text-red-500" : "text-green-500"}`}>
                  {result.droppedNotes > 0 ? <XCircle className="h-4 w-4" /> : <CheckCircle className="h-4 w-4" />}
                  {result.droppedNotes.toLocaleString()}
                </span>
              </div>
              <div className="flex flex-col">
                <span className="text-muted-foreground">{t("stress.xruns")}</span>
                <span className={`font-semibold flex items-center gap-1 ${result.xruns > 0 ? "text-red-500" : "text-green-500"}`}>
                  {result.xruns > 0 ? <XCircle className="h-4 w-4" /> : <CheckCircle className="h-4 w-4" />}
                  {result.xruns.toLocaleString()}
                </span>
              </div>
            </div>

            {(result.segmentRtp || result.segmentBridge || result.segmentAudio) && (
              <div className="space-y-2">
                <h5 className="text-xs font-medium text-muted-foreground uppercase tracking-wider">{t("stress.segments")}</h5>
                <div className="grid gap-3 text-xs sm:grid-cols-2">
                  {result.segmentBridge && (
                    <div className={`p-2 rounded-md ${result.segmentBridge.dropped > 0 ? "bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-900/50" : "bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-900/50"}`}>
                      <div className="flex items-center gap-1 font-medium mb-1">
                        <Workflow className="h-3 w-3" />
                        {t("stress.bridgeSegment")}
                      </div>
                      <div className="space-y-0.5 text-muted-foreground">
                        <div>{t("stress.dropsShort")}: <span className={result.segmentBridge.dropped > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentBridge.dropped}</span></div>
                        {result.receivedNotes !== undefined && (
                          <div>{t("stress.received")}: {result.receivedNotes.toLocaleString()}</div>
                        )}
                      </div>
                    </div>
                  )}

                  {result.segmentAudio && (
                    <div className={`p-2 rounded-md ${(result.segmentAudio.dropped > 0 || result.segmentAudio.xruns > 0) ? "bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-900/50" : "bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-900/50"}`}>
                      <div className="flex items-center gap-1 font-medium mb-1">
                        <AudioWaveform className="h-3 w-3" />
                        Audio/VST
                      </div>
                      <div className="space-y-0.5 text-muted-foreground">
                        <div>{t("stress.dropsShort")}: <span className={result.segmentAudio.dropped > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentAudio.dropped}</span></div>
                        <div>{t("stress.xrunsShort")}: <span className={result.segmentAudio.xruns > 0 ? "text-red-600 dark:text-red-400 font-medium" : "text-green-600 dark:text-green-400"}>{result.segmentAudio.xruns}</span></div>
                        {result.segmentAudio.latencyMs !== null && (
                          <div>{t("stress.latency")}: {result.segmentAudio.latencyMs.toFixed(1)}ms</div>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            )}

            {result.droppedNotes === 0 && result.xruns === 0 ? (
              <p className="text-xs text-green-600 dark:text-green-400">
                <CheckCircle className="h-3 w-3 inline mr-1" />
                {t("stress.stable")}
              </p>
            ) : (
              <div className="text-xs text-red-600 dark:text-red-400 space-y-1">
                <p className="font-medium"><XCircle className="h-3 w-3 inline mr-1" />{t("stress.unstable")}</p>
                {result.segmentBridge && result.segmentBridge.dropped > 0 && <p>• {t("stress.bridgeDropsHint")}</p>}
                {result.segmentAudio && result.segmentAudio.dropped > 0 && result.segmentAudio.xruns === 0 && (
                  <p>• {t("stress.audioDropsHint")}</p>
                )}
                {result.segmentAudio && result.segmentAudio.dropped > 0 && result.segmentAudio.xruns > 0 && (
                  <p>• {t("stress.audioXrunDropsHint")}</p>
                )}
                {result.segmentAudio && result.segmentAudio.xruns > 0 && <p>• {t("stress.xrunsHint")}</p>}
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
