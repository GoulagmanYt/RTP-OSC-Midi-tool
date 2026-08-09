import { useState } from "react";
import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "../components/ui/Card";
import { Label } from "../components/ui/Label";
import { Input } from "../components/ui/Input";
import { Switch } from "../components/ui/Switch";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Radio, Server, RefreshCcw } from "lucide-react";
import { useI18n } from "../providers/LanguageProvider";
import { toast } from "sonner";
import { getErrorMessage } from "../api-errors";

export default function OscPage() {
  const { config, updateConfig, saveConfig } = useBridge();
  const [busy, setBusy] = useState(false);
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
            <Radio className="h-5 w-5 text-purple-500" />
            {t("osc.title")}
          </CardTitle>
          <CardDescription>{t("osc.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="flex items-center justify-between p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
            <div className="flex flex-col gap-1">
              <Label className="font-medium">{t("osc.enable")}</Label>
              <p className="text-xs text-muted-foreground">{t("osc.enableHint")}</p>
            </div>
            <Switch checked={config?.osc.enabled ?? false} onCheckedChange={handleToggleOsc} />
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
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
                <Input
                  id="osc-port"
                  type="number"
                  placeholder="9000"
                  value={config?.osc.targetPort ?? ""}
                  onChange={(e) =>
                    updateConfig({ osc: { targetPort: Number(e.target.value || 0) } })
                  }
                />
              </div>
            </div>
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
            <h3 className="font-semibold text-sm">{t("osc.protocolOptions")}</h3>
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <Label className="text-sm">{t("osc.logOsc")}</Label>
                <Switch
                  checked={config?.osc.logMessages ?? false}
                  onCheckedChange={(c) => updateConfig({ osc: { logMessages: c } })}
                />
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-sm">{t("osc.verbose")}</Label>
                <Switch
                  checked={config?.logging.verbose ?? false}
                  onCheckedChange={(c) => updateConfig({ logging: { verbose: c } })}
                />
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-4 border-t">
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => updateConfig({ osc: { targetIp: "127.0.0.1", targetPort: 9000 } })}
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
            {t("osc.connectionStatus")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid gap-3 md:grid-cols-2">
            <div className="p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">{t("osc.statusLabel")}</span>
                <Badge variant={config?.osc.enabled ? "success" : "secondary"}>
                  {config?.osc.enabled ? t("common.enabled") : t("common.disabled")}
                </Badge>
              </div>
              <div className="text-xs text-muted-foreground mt-2">
                {config?.osc.targetIp}:{config?.osc.targetPort}
              </div>
            </div>

            <div className="p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">{t("osc.connected")}</span>
                <Badge variant={config?.osc.enabled ? "success" : "secondary"}>
                  {config?.osc.enabled ? t("common.yes") : t("common.no")}
                </Badge>
              </div>
              <div className="text-xs text-muted-foreground mt-2">
                {config?.osc.enabled ? (
                  <div className="flex items-center gap-1">
                    <div className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse" />
                    {t("osc.flowActive")}
                  </div>
                ) : (
                  t("osc.waiting")
                )}
              </div>
            </div>
          </div>
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
            <code className="bg-slate-100 dark:bg-slate-900 px-1.5 py-0.5 rounded text-xs">9000</code>
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
