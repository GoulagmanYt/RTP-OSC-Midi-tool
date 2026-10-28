import { RotateCcw } from "lucide-react";
import { toast } from "sonner";
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
        aria-valuetext={`${value} ${unit}`}
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
  const maximumFrequency = Math.max(21, Math.min(20_000, sampleRate / 2));
  const frequencies = [20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000]
    .filter((frequency) => frequency < maximumFrequency).concat(maximumFrequency);
  const dbLines = [-18, -12, -6, 0, 6, 12, 18];
  const xForFrequency = (frequency: number) =>
    padding.left +
    (Math.log10(frequency / 20) / Math.log10(maximumFrequency / 20)) * (width - padding.left - padding.right);
  const yForDb = (db: number) =>
    padding.top + ((18 - Math.min(18, Math.max(-18, db))) / 36) * (height - padding.top - padding.bottom);
  const responsePath = Array.from({ length: 160 }, (_, index) => {
    const frequency = 20 * Math.pow(maximumFrequency / 20, index / 159);
    return `${index === 0 ? "M" : "L"}${xForFrequency(frequency).toFixed(2)},${yForDb(
      eqResponseDb(settings, frequency, sampleRate)
    ).toFixed(2)}`;
  }).join(" ");

  return (
    <div role="img" aria-label={label} className="w-full rounded-md border bg-muted/20 p-2">
      <div className="relative h-[10rem]">
        <svg aria-hidden="true" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className="h-full w-full">
          {dbLines.map(db => <line key={db} x1={padding.left} x2={width - padding.right} y1={yForDb(db)} y2={yForDb(db)} className={db === 0 ? "stroke-input" : "stroke-border"} />)}
          {frequencies.map(frequency => <line key={frequency} x1={xForFrequency(frequency)} x2={xForFrequency(frequency)} y1={padding.top} y2={height - padding.bottom} className="stroke-border" />)}
          <path d={responsePath} fill="none" className="stroke-primary" strokeWidth="2.5" />
        </svg>
        <div aria-hidden="true" className="absolute inset-y-0 left-0 flex w-8 flex-col justify-between pb-5 text-[0.8125rem] tabular-nums text-muted-foreground">
          <span>+18</span><span>0</span><span>−18</span>
        </div>
      </div>
      <div aria-hidden="true" className="relative h-6 text-[0.8125rem] tabular-nums text-muted-foreground">
        {[20, 200, 2000, maximumFrequency].filter((frequency, index, all) => frequency <= maximumFrequency && all.indexOf(frequency) === index).map(frequency => (
          <span key={frequency} className="absolute" style={{ left: `${xForFrequency(frequency) / width * 100}%`, transform: frequency === maximumFrequency ? "translateX(-100%)" : "translateX(-50%)" }}>
            {frequency >= 1000 ? `${frequency / 1000}k` : frequency}
          </span>
        ))}
      </div>
    </div>
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
          <Label htmlFor="eq-enabled" className="flex flex-col gap-1">
            <span>{t("audio.eq.enabled")}</span>
            <span className="text-xs font-normal text-muted-foreground">{t("audio.eq.enabledHint")}</span>
          </Label>
          <Switch id="eq-enabled" checked={settings.enabled} onCheckedChange={(enabled) => onChange({ ...settings, enabled })} />
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
          <Button variant="outline" onClick={() => { onReset(); toast.success(t("audio.eq.reset"), { duration: 10000, action: { label: t("ui.undo"), onClick: () => onChange(settings) } }); }}>
            <RotateCcw className="mr-2 h-4 w-4" />
            {t("audio.eq.reset")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
