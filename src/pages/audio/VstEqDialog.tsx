import { RotateCcw } from "lucide-react";
import type { VstEqSettings } from "../../api-types";
import { Button } from "../../components/ui/Button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";
import { Switch } from "../../components/ui/Switch";
import type { TranslateFn } from "./shared";
import { EQ_RANGES, eqResponseDb, frequencyToSlider, sliderToFrequency } from "./eq";

type Props = {
  open: boolean;
  pluginName: string;
  settings: VstEqSettings;
  sampleRate: number;
  t: TranslateFn;
  onOpenChange: (open: boolean) => void;
  onChange: (settings: VstEqSettings) => void;
  onReset: () => void;
};

type Range = { min: number; max: number; step: number };

function clamp(value: number, range: Range): number {
  return Math.min(range.max, Math.max(range.min, value));
}

function FrequencyControl({
  id,
  label,
  value,
  range,
  unit,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  range: Range;
  unit: string;
  onChange: (value: number) => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={`${id}-range`}>{label}</Label>
        <div className="flex items-center gap-1">
          <Input
            id={`${id}-number`}
            type="number"
            min={range.min}
            max={range.max}
            step={range.step}
            value={value}
            onChange={(event) => {
              const next = Number(event.target.value);
              if (Number.isFinite(next)) onChange(clamp(next, range));
            }}
            className="h-8 w-24 px-2 text-right"
            aria-label={label}
          />
          <span className="w-8 text-xs text-muted-foreground">{unit}</span>
        </div>
      </div>
      <Input
        id={`${id}-range`}
        type="range"
        min={0}
        max={1_000}
        step={1}
        value={frequencyToSlider(value, range.min, range.max)}
        onChange={(event) => onChange(sliderToFrequency(Number(event.target.value), range.min, range.max))}
        className="h-7 w-full px-0 py-0"
      />
    </div>
  );
}

function LinearControl({
  id,
  label,
  value,
  range,
  unit,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  range: Range;
  unit: string;
  onChange: (value: number) => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={`${id}-range`}>{label}</Label>
        <div className="flex items-center gap-1">
          <Input
            id={`${id}-number`}
            type="number"
            min={range.min}
            max={range.max}
            step={range.step}
            value={value}
            onChange={(event) => {
              const next = Number(event.target.value);
              if (Number.isFinite(next)) onChange(clamp(next, range));
            }}
            className="h-8 w-20 px-2 text-right"
            aria-label={label}
          />
          {unit ? <span className="w-7 text-xs text-muted-foreground">{unit}</span> : null}
        </div>
      </div>
      <Input
        id={`${id}-range`}
        type="range"
        min={range.min}
        max={range.max}
        step={range.step}
        value={value}
        onChange={(event) => onChange(clamp(Number(event.target.value), range))}
        className="h-7 w-full px-0 py-0"
      />
    </div>
  );
}

function ResponseGraph({ settings, sampleRate, label }: { settings: VstEqSettings; sampleRate: number; label: string }) {
  const width = 720;
  const height = 180;
  const padding = { left: 38, right: 12, top: 12, bottom: 24 };
  const frequencies = [20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000];
  const dbLines = [-18, -12, -6, 0, 6, 12, 18];
  const xForFrequency = (frequency: number) =>
    padding.left +
    (Math.log10(frequency / 20) / Math.log10(20_000 / 20)) * (width - padding.left - padding.right);
  const yForDb = (db: number) =>
    padding.top + ((18 - Math.min(18, Math.max(-18, db))) / 36) * (height - padding.top - padding.bottom);
  const responsePath = Array.from({ length: 160 }, (_, index) => {
    const frequency = 20 * Math.pow(1_000, index / 159);
    return `${index === 0 ? "M" : "L"}${xForFrequency(frequency).toFixed(2)},${yForDb(
      eqResponseDb(settings, frequency, sampleRate)
    ).toFixed(2)}`;
  }).join(" ");

  return (
    <svg role="img" aria-label={label} viewBox={`0 0 ${width} ${height}`} className="w-full rounded-md border bg-muted/20">
      {dbLines.map((db) => (
        <g key={db}>
          <line
            x1={padding.left}
            x2={width - padding.right}
            y1={yForDb(db)}
            y2={yForDb(db)}
            className={db === 0 ? "stroke-border" : "stroke-border/50"}
          />
          <text x={padding.left - 5} y={yForDb(db) + 3} textAnchor="end" className="fill-muted-foreground text-[9px]">
            {db > 0 ? `+${db}` : db}
          </text>
        </g>
      ))}
      {frequencies.map((frequency) => (
        <g key={frequency}>
          <line
            x1={xForFrequency(frequency)}
            x2={xForFrequency(frequency)}
            y1={padding.top}
            y2={height - padding.bottom}
            className="stroke-border/30"
          />
          <text
            x={xForFrequency(frequency)}
            y={height - 7}
            textAnchor="middle"
            className="fill-muted-foreground text-[9px]"
          >
            {frequency >= 1_000 ? `${frequency / 1_000}k` : frequency}
          </text>
        </g>
      ))}
      <path d={responsePath} fill="none" className="stroke-primary" strokeWidth="2.5" />
    </svg>
  );
}

export function VstEqDialog({ open, pluginName, settings, sampleRate, t, onOpenChange, onChange, onReset }: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92vh] max-w-4xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("audio.eq.title")}</DialogTitle>
          <DialogDescription>{t("audio.eq.description", { plugin: pluginName })}</DialogDescription>
        </DialogHeader>

        <div className="flex items-center justify-between rounded-md border p-3">
          <Label className="flex flex-col gap-1">
            <span>{t("audio.eq.enabled")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.eq.enabledHint")}</span>
          </Label>
          <Switch checked={settings.enabled} onCheckedChange={(enabled) => onChange({ ...settings, enabled })} />
        </div>

        <ResponseGraph settings={settings} sampleRate={sampleRate} label={t("audio.eq.responseGraph")} />

        <div className="grid gap-4 md:grid-cols-3">
          <section className="space-y-4 rounded-md border p-4">
            <h3 className="font-semibold">{t("audio.eq.lowBand")}</h3>
            <FrequencyControl
              id="eq-low-frequency"
              label={t("audio.eq.frequency")}
              value={settings.lowShelf.frequencyHz}
              range={EQ_RANGES.lowFrequency}
              unit={t("units.hz")}
              onChange={(frequencyHz) => onChange({ ...settings, lowShelf: { ...settings.lowShelf, frequencyHz } })}
            />
            <LinearControl
              id="eq-low-gain"
              label={t("audio.eq.gain")}
              value={settings.lowShelf.gainDb}
              range={EQ_RANGES.gain}
              unit={t("units.db")}
              onChange={(gainDb) => onChange({ ...settings, lowShelf: { ...settings.lowShelf, gainDb } })}
            />
          </section>

          <section className="space-y-4 rounded-md border p-4">
            <h3 className="font-semibold">{t("audio.eq.midBand")}</h3>
            <FrequencyControl
              id="eq-mid-frequency"
              label={t("audio.eq.frequency")}
              value={settings.midPeak.frequencyHz}
              range={EQ_RANGES.midFrequency}
              unit={t("units.hz")}
              onChange={(frequencyHz) => onChange({ ...settings, midPeak: { ...settings.midPeak, frequencyHz } })}
            />
            <LinearControl
              id="eq-mid-gain"
              label={t("audio.eq.gain")}
              value={settings.midPeak.gainDb}
              range={EQ_RANGES.gain}
              unit={t("units.db")}
              onChange={(gainDb) => onChange({ ...settings, midPeak: { ...settings.midPeak, gainDb } })}
            />
            <LinearControl
              id="eq-mid-q"
              label={t("audio.eq.q")}
              value={settings.midPeak.q}
              range={EQ_RANGES.q}
              unit=""
              onChange={(q) => onChange({ ...settings, midPeak: { ...settings.midPeak, q } })}
            />
          </section>

          <section className="space-y-4 rounded-md border p-4">
            <h3 className="font-semibold">{t("audio.eq.highBand")}</h3>
            <FrequencyControl
              id="eq-high-frequency"
              label={t("audio.eq.frequency")}
              value={settings.highShelf.frequencyHz}
              range={EQ_RANGES.highFrequency}
              unit={t("units.hz")}
              onChange={(frequencyHz) => onChange({ ...settings, highShelf: { ...settings.highShelf, frequencyHz } })}
            />
            <LinearControl
              id="eq-high-gain"
              label={t("audio.eq.gain")}
              value={settings.highShelf.gainDb}
              range={EQ_RANGES.gain}
              unit={t("units.db")}
              onChange={(gainDb) => onChange({ ...settings, highShelf: { ...settings.highShelf, gainDb } })}
            />
          </section>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onReset}>
            <RotateCcw className="mr-2 h-4 w-4" />
            {t("audio.eq.reset")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
