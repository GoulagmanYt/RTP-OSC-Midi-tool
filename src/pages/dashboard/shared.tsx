import { cn } from "../../utils";

type MeterRowProps = {
  label: string;
  value?: number | null;
  max: number;
  valueLabel?: string;
  className?: string;
};

export function MeterRow({ label, value, max, valueLabel, className }: MeterRowProps) {
  const safeValue = value ?? 0;
  const pct = max > 0 ? Math.min(safeValue / max, 1) : 0;
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{label}</span>
        <span className="font-medium text-foreground">{valueLabel ?? "--"}</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-muted/30">
        <div className={cn("h-full transition-all duration-200", className)} style={{ width: `${pct * 100}%` }} />
      </div>
    </div>
  );
}
