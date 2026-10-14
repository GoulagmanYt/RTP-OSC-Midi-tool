import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../../constants";

export type Translate = (key: string, params?: Record<string, string | number>) => string;

export function formatNote(note?: number | null) {
  if (note === null || note === undefined || !Number.isFinite(note) || note < 0 || note > 127) return "--";
  const floored = Math.floor(note);
  const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
  const octave = Math.floor(floored / 12) - 1;
  return `${names[floored % 12]}${octave}`;
}

export function displayPortName(name: string, t: Translate) {
  if (name === RTP_VIRTUAL_INPUT) return t("devices.rtpVirtualInput");
  if (name === VST_OUTPUT) return t("devices.vstInternalOutput");
  return name;
}

export function clampMidiNumber(value: unknown, min: number, max: number, fallback = min): number {
  if (value === null || value === undefined || value === "") return fallback;
  const num = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(num)) return fallback;
  return Math.max(min, Math.min(max, Math.floor(num)));
}

export function clampNullableMidiNumber(value: unknown, min: number, max: number): number | null {
  if (value === null || value === undefined || value === "") return null;
  const num = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(num)) return null;
  return Math.max(min, Math.min(max, Math.floor(num)));
}

export function clampAudioGain(value: unknown, min = -60, max = 12, fallback = 0): number {
  if (value === null || value === undefined || value === "") return fallback;
  const num = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(num)) return fallback;
  return Math.max(min, Math.min(max, num));
}

