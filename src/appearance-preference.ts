import type { ThemePalette } from "./themes";
const KEY = "oscMIDI.appearance-choice.v1";

/** A factory light default predates system appearance; explicit choices are retained. */
export function initialAppearance(palette?: string): ThemePalette {
  const valid = palette === "light" || palette === "dark" || palette === "contrast" || palette === "system" ? palette : "system";
  try {
    if (valid === "light" && !localStorage.getItem(KEY)) return "system";
  } catch {
    return valid === "light" ? "system" : valid;
  }
  return valid;
}

/** Called only after native persistence/import succeeds, so failures cannot change preferences. */
export function rememberAppearance(palette?: string): void {
  if (!palette) return;
  try {
    if (localStorage.getItem(KEY) !== palette) localStorage.setItem(KEY, palette);
  } catch { /* The current session still works when storage is unavailable. */ }
}
