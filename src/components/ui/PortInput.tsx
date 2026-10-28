import { useEffect, useRef, useState } from "react";
import { Input } from "./Input";

type Props = {
  id: string;
  value?: number;
  min: number;
  max: number;
  placeholder?: string;
  resetNonce?: number;
  onCommit: (value: number) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
};

export function PortInput({ id, value, min, max, placeholder, resetNonce, onCommit, t }: Props) {
  const [draft, setDraft] = useState(value?.toString() ?? "");
  const [touched, setTouched] = useState(false);
  const committed = useRef(value);
  useEffect(() => {
    setDraft(value?.toString() ?? "");
    setTouched(false);
    committed.current = value;
  }, [value, resetNonce]);
  const number = Number(draft);
  const valid = draft.trim() !== "" && Number.isInteger(number) && number >= min && number <= max;
  const invalid = touched && !valid;
  const commit = () => {
    setTouched(true);
    if (!valid) return;
    setDraft(String(number));
    if (number !== committed.current) {
      committed.current = number;
      onCommit(number);
    }
  };
  return (
    <div className="space-y-1">
      <Input id={id} type="number" min={min} max={max} step={1} placeholder={placeholder}
        value={draft} aria-invalid={invalid || undefined} aria-describedby={invalid ? `${id}-error` : undefined}
        onChange={event => setDraft(event.target.value)} onBlur={commit}
        onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); commit(); } }} />
      {invalid ? <p id={`${id}-error`} role="alert" className="text-[0.8125rem] text-destructive">{t("ui.portRange", { min, max })}</p> : null}
    </div>
  );
}
