import { cn } from "../../utils";

type MeterRowProps = {
  label: string;
  value?: number | null;
  max: number;
  valueLabel?: string;
  className?: string;
};

export function MeterRow({ label, value, max, valueLabel, className }: MeterRowProps) {
  const safeMax = Number.isFinite(max) && max > 0 ? max : 1;
  const safeValue = Math.max(0, Math.min(Number.isFinite(value) ? value! : 0, safeMax));
  const pct = safeValue / safeMax;
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{label}</span>
        <span className="font-medium text-foreground">{valueLabel ?? "--"}</span>
      </div>
      <div role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={safeMax} aria-valuenow={safeValue} aria-valuetext={valueLabel ?? "--"} className="h-2 overflow-hidden rounded-full bg-muted/30">
        <div className={cn("h-full transition-[width] duration-200", className)} style={{ width: `${pct * 100}%` }} />
      </div>
    </div>
  );
}
