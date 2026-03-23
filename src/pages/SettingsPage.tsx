import { useEffect, useState } from "react";
import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/ui/Card";
import { Label } from "../components/ui/Label";
import { Switch } from "../components/ui/Switch";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Palette, Trash2, Save, RefreshCcw, Download, FileOutput } from "lucide-react";
import { toast } from "sonner";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  clearLogFile,
  exportConfig,
  exportDiagnostics,
  getAppPaths,
  importConfig,
  openAppDir,
  resetConfigDefaults,
  type AppPaths,
} from "../api";
import { useI18n } from "../providers/LanguageProvider";

type PaletteChoice = {
  id: "light" | "dark" | "contrast";
  labelKey: string;
};

const PALETTE_CHOICES: PaletteChoice[] = [
  { id: "light", labelKey: "settings.theme.palette.light" },
  { id: "dark", labelKey: "settings.theme.palette.dark" },
  { id: "contrast", labelKey: "settings.theme.palette.contrast" },
];

export default function SettingsPage() {
  const { config, updateConfig, saveConfig, clearLogs, reloadConfig } = useBridge();
  const [busy, setBusy] = useState(false);
  const [paths, setPaths] = useState<AppPaths | null>(null);
  const { t } = useI18n();

  useEffect(() => {
    getAppPaths()
      .then(setPaths)
      .catch((e) => console.error("Failed to load app paths", e));
  }, []);

  const currentPalette = (config?.themePalette as PaletteChoice["id"]) || "light";

  const handleSave = async () => {
    setBusy(true);
    try {
      await saveConfig();
      toast.success(t("toasts.settings.configSaved"));
    } finally {
      setBusy(false);
    }
  };

  const handleClearLogs = async () => {
    setBusy(true);
    try {
      await clearLogFile();
      clearLogs();
      toast.success(t("toasts.settings.logsCleared"));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.settings.logsClearFailed"));
    } finally {
      setBusy(false);
    }
  };

  const handleReset = async () => {
    setBusy(true);
    try {
      await resetConfigDefaults();
      await reloadConfig();
      toast.success(t("toasts.settings.configReset"));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.settings.configResetFailed"));
    } finally {
      setBusy(false);
    }
  };

  const handlePaletteChange = async (palette: PaletteChoice["id"]) => {
    const nextTheme = palette === "light" ? "light" : "dark";
    await updateConfig({
      themePalette: palette,
      theme: nextTheme,
    });
  };

  const handleExportConfig = async () => {
    setBusy(true);
    try {
      const target = await save({
        title: t("settings.dialog.exportTitle"),
        defaultPath: paths?.configDir ? `${paths.configDir}\\config.yaml` : "config.yaml",
        filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
      });
      if (!target) return;
      await exportConfig(target.toString());
      toast.success(t("toasts.settings.exportSuccess"));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.settings.exportFailed"));
    } finally {
      setBusy(false);
    }
  };

  const handleExportDiagnostics = async () => {
    setBusy(true);
    try {
      const target = await save({
        title: t("settings.dialog.exportDiagnosticsTitle"),
        defaultPath: paths?.configDir ? `${paths.configDir}\\diagnostic.json` : "diagnostic.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!target) return;
      await exportDiagnostics(target.toString());
      toast.success(t("toasts.settings.exportDiagnosticsSuccess"));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.settings.exportDiagnosticsFailed"));
    } finally {
      setBusy(false);
    }
  };

  const handleImportConfig = async () => {
    setBusy(true);
    try {
      const selected = await open({
        title: t("settings.dialog.importTitle"),
        multiple: false,
        filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
      });
      if (!selected || Array.isArray(selected)) return;
      await importConfig(selected.toString());
      await reloadConfig();
      toast.success(t("toasts.settings.importSuccess"));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.settings.importFailed"));
    } finally {
      setBusy(false);
    }
  };

  const handleOpenDir = async (target: "config" | "logs") => {
    try {
      await openAppDir(target);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message || t("toasts.settings.openDirFailed"));
    }
  };

  const copyPath = async (value?: string) => {
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      toast.success(t("toasts.settings.copyPathSuccess"));
    } catch (e) {
      console.error("Clipboard copy failed", e);
      toast.error(t("toasts.settings.copyPathFailed"));
    }
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Palette className="h-5 w-5 text-blue-500" />
            {t("settings.theme.title")}
          </CardTitle>
          <CardDescription>{t("settings.theme.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <Label>{t("settings.theme.quickPalette")}</Label>
          <div className="grid gap-2 sm:grid-cols-3">
            {PALETTE_CHOICES.map((palette) => (
              <Button
                key={palette.id}
                variant={currentPalette === palette.id ? "default" : "outline"}
                className="w-full"
                onClick={() => handlePaletteChange(palette.id)}
                disabled={busy}
              >
                {t(palette.labelKey)}
              </Button>
            ))}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <RefreshCcw className="h-5 w-5 text-blue-500" />
            {t("settings.general.title")}
          </CardTitle>
          <CardDescription>{t("settings.general.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0 flex flex-col gap-1">
              <Label>{t("settings.general.verbose")}</Label>
              <p className="text-xs text-muted-foreground">{t("settings.general.verboseHint")}</p>
            </div>
            <Switch checked={config?.verbose || false} onCheckedChange={(checked) => updateConfig({ verbose: checked })} />
          </div>

          <div className="border-t pt-4" />

          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0 flex flex-col gap-1">
              <Label>{t("settings.general.oscLogs")}</Label>
              <p className="text-xs text-muted-foreground">{t("settings.general.oscLogsHint")}</p>
            </div>
            <Switch checked={config?.logOsc || false} onCheckedChange={(checked) => updateConfig({ logOsc: checked })} />
          </div>

          <div className="border-t pt-4" />

          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0 flex flex-col gap-1">
              <Label>{t("settings.general.logAllToFile")}</Label>
              <p className="text-xs text-muted-foreground">{t("settings.general.logAllToFileHint")}</p>
            </div>
            <Switch
              checked={config?.logAllToFile || false}
              onCheckedChange={(checked) => updateConfig({ logAllToFile: checked })}
            />
          </div>

          <div className="border-t pt-4" />

          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0 flex flex-col gap-1">
              <Label>{t("settings.general.disableLogs")}</Label>
              <p className="text-xs text-muted-foreground">{t("settings.general.disableLogsHint")}</p>
            </div>
            <Switch
              checked={!(config?.logsEnabled ?? true)}
              onCheckedChange={(checked) => updateConfig({ logsEnabled: !checked })}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Download className="h-5 w-5 text-blue-500" />
            {t("settings.files.title")}
          </CardTitle>
          <CardDescription>{t("settings.files.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" disabled={busy} onClick={handleExportConfig}>
              <Download className="h-4 w-4 mr-2" />
              {t("settings.files.exportYaml")}
            </Button>
            <Button variant="outline" disabled={busy} onClick={handleExportDiagnostics}>
              <FileOutput className="h-4 w-4 mr-2" />
              {t("settings.files.exportDiagnostics")}
            </Button>
            <Button variant="outline" disabled={busy} onClick={handleImportConfig}>
              <RefreshCcw className="h-4 w-4 mr-2" />
              {t("settings.files.importYaml")}
            </Button>
          </div>

          <div className="space-y-2">
            <Label>{t("settings.files.configFolder")}</Label>
            <div className="flex flex-wrap items-center gap-2">
              <Input readOnly value={paths?.configDir || t("common.unknown")} className="min-w-[240px] flex-1" />
              <Button variant="outline" size="sm" onClick={() => copyPath(paths?.configDir)} disabled={!paths?.configDir}>
                {t("common.copyPath")}
              </Button>
              <Button variant="outline" size="sm" onClick={() => handleOpenDir("config")} disabled={!paths?.configDir}>
                <FileOutput className="w-4 h-4 mr-1" />
                {t("common.openFolder")}
              </Button>
            </div>
          </div>

          <div className="space-y-2">
            <Label>{t("settings.files.logs")}</Label>
            <div className="flex flex-wrap items-center gap-2">
              <Input readOnly value={paths?.logFile || t("common.unknown")} className="min-w-[240px] flex-1" />
              <Button variant="outline" size="sm" onClick={() => copyPath(paths?.logFile)} disabled={!paths?.logFile}>
                {t("common.copyPath")}
              </Button>
              <Button variant="outline" size="sm" onClick={() => handleOpenDir("logs")} disabled={!paths?.logFile}>
                <FileOutput className="w-4 h-4 mr-1" />
                {t("common.openFolder")}
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Trash2 className="h-5 w-5 text-orange-500" />
            {t("settings.logs.title")}
          </CardTitle>
          <CardDescription>{t("settings.logs.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">{t("settings.logs.hint")}</p>
          <div className="flex justify-end gap-2">
            <Button variant="outline" disabled={busy} onClick={handleClearLogs}>
              <RefreshCcw className="h-4 w-4 mr-2" />
              {t("settings.logs.clear")}
            </Button>
          </div>
        </CardContent>
      </Card>

      <div className="flex justify-end gap-2">
        <Button variant="outline" disabled={busy} onClick={handleReset}>
          <RefreshCcw className="h-4 w-4 mr-2" />
          {t("common.reset")}
        </Button>
        <Button disabled={busy} onClick={handleSave}>
          <Save className="h-4 w-4 mr-2" />
          {t("common.save")}
        </Button>
      </div>
    </div>
  );
}
