export type TranslateFn = (key: string, values?: Record<string, string | number>) => string;

export type AudioQuickPreset = {
  id: string;
  label: string;
  backend: string;
  bufferSize: number;
  sampleRate: number;
  hint: string;
};
