import { useState } from "react";
import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "../components/ui/Card";
import { Label } from "../components/ui/Label";
import { Input } from "../components/ui/Input";
import { PortInput } from "../components/ui/PortInput";
import { Switch } from "../components/ui/Switch";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Radio, Server, RefreshCcw } from "lucide-react";
import { useI18n } from "../providers/LanguageProvider";
import { toast } from "sonner";
import { getErrorMessage } from "../api-errors";

export default function OscPage() {
  const { config, status, updateConfig, saveConfig } = useBridge();
  const [busy, setBusy] = useState(false);
  const [portResetNonce, setPortResetNonce] = useState(0);
  const { t } = useI18n();

  const handleSave = async () => {
    setBusy(true);
    try {
      await saveConfig();
      toast.success(t("toasts.settings.configSaved"));
    } catch (error) {
      toast.error(getErrorMessage(error) || t("toasts.settings.configSaveFailed"));
    } finally {
      setBusy(false);
    }
  };

  const handleToggleOsc = async (checked: boolean) => {
    await updateConfig({ osc: { enabled: checked } });
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Radio className="h-5 w-5 text-muted-foreground" />
            {t("osc.configurationTitle")}
          </CardTitle>
          <CardDescription>{t("osc.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="flex items-center justify-between p-4 rounded-lg bg-background border border-border/60">
            <div className="flex flex-col gap-1">
              <Label htmlFor="osc-enabled" className="font-medium">{t("osc.enable")}</Label>
              <p className="text-xs text-muted-foreground">{t("osc.enableHint")}</p>
            </div>
            <Switch id="osc-enabled" checked={config?.osc.enabled ?? false} onCheckedChange={handleToggleOsc} />
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-background border border-border/60">
            <h3 className="font-semibold text-sm">{t("osc.target")}</h3>
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="osc-ip" className="text-sm">
                  {t("osc.ip")}
                </Label>
                <Input
                  id="osc-ip"
                  type="text"
                  placeholder="127.0.0.1"
                  value={config?.osc.targetIp ?? ""}
                  onChange={(e) => updateConfig({ osc: { targetIp: e.target.value } })}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="osc-port" className="text-sm">
                  {t("osc.port")}
                </Label>
                <OscPortInput
                  id="osc-port"
                  placeholder="9000"
                  value={config?.osc.targetPort}
                  resetNonce={portResetNonce}
                  onCommit={(targetPort) => updateConfig({ osc: { targetPort } })}
                />
              </div>
            </div>
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-background border border-border/60">
            <h3 className="font-semibold text-sm">{t("osc.inputTitle")}</h3>
            <p className="text-xs text-muted-foreground">{t("osc.inputHint")}</p>
            <div className="flex items-center justify-between">
              <Label htmlFor="osc-input-enabled">{t("osc.inputEnable")}</Label>
              <Switch id="osc-input-enabled" checked={config?.osc.inputEnabled ?? false}
                onCheckedChange={(inputEnabled) => updateConfig({ osc: { inputEnabled } })} />
            </div>
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="osc-listen-ip">{t("osc.listenIp")}</Label>
                <Input id="osc-listen-ip" value={config?.osc.listenIp ?? "127.0.0.1"}
                  onChange={(e) => updateConfig({ osc: { listenIp: e.target.value } })} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="osc-listen-port">{t("osc.listenPort")}</Label>
                <OscPortInput
                  id="osc-listen-port"
                  placeholder="9001"
                  value={config?.osc.listenPort}
                  resetNonce={portResetNonce}
                  onCommit={(listenPort) => updateConfig({ osc: { listenPort } })}
                />
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-4 border-t">
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => {
                setPortResetNonce((prev) => prev + 1);
                updateConfig({
                  osc: {
                    targetIp: "127.0.0.1",
                    targetPort: 9000,
                    listenIp: "127.0.0.1",
                    listenPort: 9001,
                  },
                });
              }}
            >
              <RefreshCcw className="h-4 w-4 mr-2" />
              {t("common.default")}
            </Button>
            <Button onClick={handleSave} disabled={busy}>
              {t("common.save")}
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm flex items-center gap-2">
            <Server className="h-4 w-4" />
            {t("osc.outputStatus")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <dl className="space-y-3 text-sm">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <dt className="text-muted-foreground">{t("osc.target")}</dt>
              <dd className="break-all font-mono">{config ? `${config.osc.targetIp}:${config.osc.targetPort}` : t("common.unknown")}</dd>
            </div>
            <div className="flex items-center justify-between gap-2">
              <dt className="text-muted-foreground">{t("osc.transmission")}</dt>
              <dd><Badge variant={config?.osc.enabled && status?.running ? "success" : "secondary"}>
                {!config ? t("common.unknown") : !config.osc.enabled ? t("common.disabled") : !status ? t("common.unknown") : status.running ? t("osc.outputReady") : t("common.stopped")}
              </Badge></dd>
            </div>
          </dl>
          <p className="text-xs text-muted-foreground">{t("osc.deliveryHint")}</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">{t("osc.about")}</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-2">
          <p>{t("osc.aboutText")}</p>
          <p>
            {t("osc.defaultPort")}{" "}
            <code className="bg-secondary px-1.5 py-0.5 rounded text-xs">9000</code>
          </p>
        </CardContent>
      </Card>
    </div>
  );
}

type OscPortInputProps = {
  id: string;
  value?: number;
  placeholder?: string;
  resetNonce?: number;
  onCommit: (port: number) => void;
};

export function OscPortInput({ id, value, placeholder = "9000", resetNonce, onCommit }: OscPortInputProps) {
  const { t } = useI18n();
  return <PortInput id={id} value={value} placeholder={placeholder} resetNonce={resetNonce} onCommit={onCommit} min={1} max={65535} t={t} />;
}
