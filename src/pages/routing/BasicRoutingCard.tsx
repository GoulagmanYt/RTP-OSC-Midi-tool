import type { AppConfig } from "../../api";
import { Cable, Music, RefreshCcw, Zap } from "lucide-react";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../components/ui/Select";
import { Switch } from "../../components/ui/Switch";

import type { Translate } from "./shared";

type Props = {
  config: AppConfig | null;
  inputsList: string[];
  outputsList: string[];
  t: Translate;
  onMidiInChange: (value: string) => void;
  onMidiOutChange: (value: string) => void;
  onChannelFilterChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  onRefreshLists: () => void;
  onReset: () => void;
  onPanic: () => void;
  onUpdateConfig: (patch: Partial<AppConfig>) => void | Promise<void>;
  displayPortName: (name: string) => string;
};

export function BasicRoutingCard({
  config,
  inputsList,
  outputsList,
  t,
  onMidiInChange,
  onMidiOutChange,
  onChannelFilterChange,
  onRefreshLists,
  onReset,
  onPanic,
  onUpdateConfig,
  displayPortName,
}: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Cable className="h-5 w-5 text-blue-500" />
          {t("routing.title")}
        </CardTitle>
        <CardDescription>{t("routing.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="grid items-center gap-6 md:grid-cols-3">
          <div className="space-y-2">
            <Label htmlFor="midi-input" className="text-sm font-medium">
              {t("routing.midiInput")}
            </Label>
            <Select value={config?.midi.inputDevice ?? ""} onValueChange={onMidiInChange}>
              <SelectTrigger>
                <SelectValue placeholder={t("routing.noneSelected")} />
              </SelectTrigger>
              <SelectContent>
                {inputsList.map((input) => (
                  <SelectItem key={input} value={input}>
                    {displayPortName(input)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col items-center justify-center gap-2 text-sm text-muted-foreground">
            <Music className="h-5 w-5 text-muted-foreground" />
            <span>{t("routing.midiRelay")}</span>
          </div>

          <div className="space-y-2">
            <Label htmlFor="midi-output" className="text-sm font-medium">
              {t("routing.midiOutput")}
            </Label>
            <Select value={config?.midi.outputDevice ?? ""} onValueChange={onMidiOutChange}>
              <SelectTrigger>
                <SelectValue placeholder={t("routing.noneSelected")} />
              </SelectTrigger>
              <SelectContent>
                {outputsList.map((output) => (
                  <SelectItem key={output} value={output}>
                    {displayPortName(output)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div className="flex flex-col gap-1">
                <Label>{t("routing.channelFilter")}</Label>
                <p className="text-xs text-muted-foreground">{t("routing.channelFilterHint")}</p>
              </div>
              <Input
                className="w-28"
                type="number"
                min="1"
                max="16"
                placeholder={t("routing.all")}
                value={config?.midi.channelFilter ? String(config.midi.channelFilter) : ""}
                onChange={onChannelFilterChange}
              />
            </div>

            <div className="flex items-center justify-between">
              <div className="flex flex-col gap-1">
                <Label>{t("routing.midiThru")}</Label>
                <p className="text-xs text-muted-foreground">{t("routing.midiThruHint")}</p>
              </div>
              <Switch
                checked={config?.midi.thruEnabled ?? false}
                onCheckedChange={(value) => onUpdateConfig({ midi: { thruEnabled: value } as AppConfig["midi"] })}
              />
            </div>

            <div className="flex items-center justify-between">
              <div className="flex flex-col gap-1">
                <Label>{t("routing.hotplug")}</Label>
                <p className="text-xs text-muted-foreground">{t("routing.hotplugHint")}</p>
              </div>
              <Switch
                checked={config?.midi.hotplug ?? false}
                onCheckedChange={(value) => onUpdateConfig({ midi: { hotplug: value } as AppConfig["midi"] })}
              />
            </div>
          </div>

          <div className="flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <div className="flex flex-col gap-1">
                <Label>{t("routing.audioGain")}</Label>
                <p className="text-xs text-muted-foreground">{t("routing.audioGainHint")}</p>
              </div>
              <Input
                className="w-28"
                type="number"
                min="-24"
                max="12"
                step="1"
                value={config?.audio.gainDb ?? 0}
                onChange={(event) =>
                  onUpdateConfig({ audio: { gainDb: Number(event.target.value || 0) } as AppConfig["audio"] })
                }
              />
            </div>

            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={onRefreshLists}>
                <RefreshCcw className="mr-2 h-4 w-4" />
                {t("routing.refreshPorts")}
              </Button>
              <Button variant="outline" size="sm" onClick={onReset}>
                {t("common.reset")}
              </Button>
              <Button variant="destructive" size="sm" onClick={onPanic}>
                <Zap className="mr-2 h-4 w-4" />
                {t("routing.panic")}
              </Button>
            </div>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
