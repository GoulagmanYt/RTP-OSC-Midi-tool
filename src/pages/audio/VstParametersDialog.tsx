import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../../components/ui/dialog";
import { Input } from "../../components/ui/Input";
import type { VstParameter } from "../../api-types";
import type { TranslateFn } from "./shared";

type Props = {
  isVst3: boolean;
  open: boolean;
  params: VstParameter[];
  loading: boolean;
  t: TranslateFn;
  onOpenChange: (open: boolean) => void;
  onParamChange: (index: number, value: number) => Promise<void>;
};

export function VstParametersDialog({ isVst3, open, params, loading, t, onOpenChange, onParamChange }: Props) {
  return (
    <Dialog open={open && isVst3} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("audio.vst3Parameters")}</DialogTitle>
          <DialogDescription>{t("audio.vst3Description")}</DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] space-y-4 overflow-y-auto pr-2">
          {loading ? <p className="text-sm text-muted-foreground">{t("audio.loadingParameters")}</p> : null}
          {!loading && params.length === 0 ? <p className="text-sm text-muted-foreground">{t("audio.noParameters")}</p> : null}
          {!loading
            ? params.map((param) => {
                const range = param.max - param.min;
                const displayValue = range !== 0 ? param.min + param.value * range : param.value;
                return (
                  <div key={param.index} className="space-y-1">
                    <div className="flex items-center justify-between text-sm">
                      <span className="font-medium">{param.name}</span>
                      <span className="text-muted-foreground">
                        {displayValue.toFixed(3)} {param.unit}
                      </span>
                    </div>
                    <Input
                      type="range"
                      min={0}
                      max={1}
                      step={0.001}
                      value={param.value}
                      onChange={(e) => onParamChange(param.index, Number(e.target.value))}
                      className="w-full"
                    />
                  </div>
                );
              })
            : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
