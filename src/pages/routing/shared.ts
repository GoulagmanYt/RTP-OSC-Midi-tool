import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../../constants";

export type Translate = (key: string, params?: Record<string, string | number>) => string;

export function formatNote(note?: number | null) {
  if (note === null || note === undefined) return "--";
  const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
  const octave = Math.floor(note / 12) - 1;
  return `${names[note % 12]}${octave}`;
}

export function displayPortName(name: string, t: Translate) {
  if (name === RTP_VIRTUAL_INPUT) return t("devices.rtpVirtualInput");
  if (name === VST_OUTPUT) return t("devices.vstInternalOutput");
  return name;
}
