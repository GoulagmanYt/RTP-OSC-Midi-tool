export type ThemePalette = "system" | "light" | "dark" | "contrast";

type PaletteConfig = { dark: boolean; colors: Record<string, string> };

export type AppearancePreferences = {
  accentColor?: string;
  lightBackground?: string;
  darkBackground?: string;
};

// Separate light and dark shades keep selected controls and their labels legible.
export const ACCENT_CHOICES = [
  { id: "blue", light: "212.35 100% 42.55%", dark: "216.84 100% 77.65%" },
  { id: "purple", light: "272 65% 40%", dark: "272 90% 82%" },
  { id: "pink", light: "330 72% 38%", dark: "330 85% 80%" },
  { id: "red", light: "4 75% 38%", dark: "4 90% 80%" },
  { id: "orange", light: "25 100% 32%", dark: "30 100% 76%" },
  { id: "green", light: "145 68% 27%", dark: "145 62% 72%" },
  { id: "teal", light: "185 90% 25%", dark: "185 65% 72%" },
  { id: "graphite", light: "240 4% 36%", dark: "240 7% 80%" },
] as const;

export const LIGHT_BACKGROUNDS = [
  { id: "default", color: "240.00 11.11% 96.47%", label: "soft" },
  { id: "white", color: "0 0% 100%", label: "white" },
  { id: "pearl", color: "240 7% 94%", label: "pearl" },
] as const;
export const DARK_BACKGROUNDS = [
  { id: "default", color: "240.00 3.33% 11.76%", label: "charcoal" },
  { id: "graphite", color: "240 4% 16%", label: "graphite" },
  { id: "black", color: "240 6% 7%", label: "black" },
] as const;

const backgroundOverrides: Record<string, Record<string, string>> = {
  white: { background: "0 0% 100%", sidebar: "240 11% 96%", secondary: "240 7% 95%", muted: "240 7% 95%" },
  pearl: { background: "240 7% 94%", card: "240 14% 98%", sidebar: "240.00 10.00% 92.16%", secondary: "240 7% 92%", muted: "240 7% 92%" },
  graphite: { background: "240 4% 16%", card: "240 4% 19%", sidebar: "240 4% 17%", popover: "240 4% 22%", secondary: "240 4% 22%", muted: "240 4% 22%", control: "240 4% 22%" },
  black: { background: "240 6% 7%", card: "240 3% 11%", sidebar: "240 4% 9%", popover: "240 4% 16%", secondary: "240 4% 16%", muted: "240 4% 16%", control: "240 4% 16%" },
};

const palettes: Record<Exclude<ThemePalette, "system">, PaletteConfig> = {
  light: {
    dark: false,
    colors: {
      background: "240.00 11.11% 96.47%",
      foreground: "240.00 3.33% 11.76%",
      card: "0.00 0.00% 100.00%",
      cardForeground: "240.00 3.33% 11.76%",
      popover: "0.00 0.00% 100.00%",
      popoverForeground: "240.00 3.33% 11.76%",
      primary: "212.35 100.00% 42.55%",
      primaryForeground: "0.00 0.00% 100.00%",
      secondary: "240.00 9.09% 93.53%",
      secondaryForeground: "240.00 3.33% 11.76%",
      muted: "240.00 9.09% 93.53%",
      mutedForeground: "240.00 2.94% 40.00%",
      accent: "217.89 70.37% 94.71%",
      accentForeground: "212.35 100.00% 42.55%",
      destructive: "356.47 65.11% 46.08%",
      destructiveForeground: "0.00 0.00% 100.00%",
      border: "240.00 10.64% 90.78%",
      input: "240.00 2.20% 55.49%",
      ring: "212.35 100.00% 42.55%",
      success: "138.20 57.42% 30.39%",
      successSurface: "130.91 44.00% 95.10%",
      warning: "35.61 100.00% 30.39%",
      warningSurface: "41.67 100.00% 92.94%",
      sidebar: "240.00 10.00% 92.16%",
      control: "0.00 0.00% 100.00%",
      switchOff: "240.00 2.11% 53.53%",
    },
  },
  dark: {
    dark: true,
    colors: {
      background: "240.00 3.33% 11.76%",
      foreground: "240.00 11.11% 96.47%",
      card: "240.00 2.70% 14.51%",
      cardForeground: "240.00 11.11% 96.47%",
      popover: "240.00 3.03% 19.41%",
      popoverForeground: "240.00 11.11% 96.47%",
      primary: "216.84 100.00% 77.65%",
      primaryForeground: "240.00 3.33% 11.76%",
      secondary: "240.00 3.03% 19.41%",
      secondaryForeground: "240.00 11.11% 96.47%",
      muted: "240.00 3.03% 19.41%",
      mutedForeground: "240.00 2.53% 69.02%",
      accent: "215.45 34.92% 24.71%",
      accentForeground: "226.67 100.00% 94.71%",
      destructive: "355.26 100.00% 85.10%",
      destructiveForeground: "349.41 31.48% 21.18%",
      border: "240.00 2.22% 26.47%",
      input: "240.00 2.26% 56.67%",
      ring: "216.84 100.00% 77.65%",
      success: "135.17 53.37% 68.04%",
      successSurface: "142.94 41.46% 16.08%",
      warning: "38.27 100.00% 75.10%",
      warningSurface: "38.82 39.53% 16.86%",
      sidebar: "240.00 2.78% 14.12%",
      control: "240.00 3.03% 19.41%",
      switchOff: "240.00 2.06% 47.65%",
    },
  },
  contrast: {
    dark: true,
    colors: {
      background: "240.00 5.88% 6.67%",
      foreground: "0.00 0.00% 100.00%",
      card: "240.00 3.33% 11.76%",
      cardForeground: "0.00 0.00% 100.00%",
      popover: "240.00 3.53% 16.67%",
      popoverForeground: "0.00 0.00% 100.00%",
      primary: "218.68 100.00% 85.10%",
      primaryForeground: "240.00 5.88% 6.67%",
      secondary: "240.00 3.53% 16.67%",
      secondaryForeground: "0.00 0.00% 100.00%",
      muted: "240.00 3.53% 16.67%",
      mutedForeground: "240.00 5.75% 82.94%",
      accent: "215.45 34.92% 24.71%",
      accentForeground: "0.00 0.00% 100.00%",
      destructive: "355.26 100.00% 85.10%",
      destructiveForeground: "240.00 5.88% 6.67%",
      border: "240.00 2.26% 56.67%",
      input: "240.00 2.53% 69.02%",
      ring: "218.68 100.00% 85.10%",
      success: "135.17 53.37% 68.04%",
      successSurface: "142.94 41.46% 16.08%",
      warning: "38.27 100.00% 75.10%",
      warningSurface: "38.82 39.53% 16.86%",
      sidebar: "240.00 3.33% 11.76%",
      control: "240.00 3.53% 16.67%",
      switchOff: "240.00 2.11% 53.53%",
    },
  },
};

/** Apply an appearance and release any system-preference subscription on cleanup. */
export function applyTheme(paletteId: string, radius: number, preferences: AppearancePreferences = {}): () => void {
  const root = document.documentElement;
  const systemAppearance = paletteId === "system" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  const update = () => {
    const selected = systemAppearance ? systemAppearance.matches ? "dark" : "light" : paletteId;
    const palette = palettes[selected as keyof typeof palettes] ?? palettes.light;
    const colors = { ...palette.colors };
    const backgrounds = palette.dark ? DARK_BACKGROUNDS : LIGHT_BACKGROUNDS;
    const backgroundId = palette.dark ? preferences.darkBackground : preferences.lightBackground;
    if (backgrounds.some(choice => choice.id === backgroundId) && backgroundId !== "default") {
      Object.assign(colors, backgroundOverrides[backgroundId!]);
    }
    const accent = ACCENT_CHOICES.find(choice => choice.id === preferences.accentColor);
    if (accent && accent.id !== "blue") {
      colors.primary = palette.dark ? accent.dark : accent.light;
      colors.ring = colors.primary;
      const hue = colors.primary.split(" ")[0];
      colors.accent = `${hue} ${palette.dark ? "30% 22%" : "60% 96%"}`;
      colors.accentForeground = colors.primary;
    }
    Object.entries(colors).forEach(([token, value]) => {
      root.style.setProperty(`--${token.replace(/[A-Z]/g, m => `-${m.toLowerCase()}`)}`, value);
    });
    root.classList.toggle("dark", palette.dark);
    root.classList.toggle("high-contrast", selected === "contrast");
    root.style.colorScheme = palette.dark ? "dark" : "light";
    root.style.setProperty("--radius", `${Math.min(16, Math.max(8, radius))}px`);
  };
  update();
  systemAppearance?.addEventListener("change", update);
  return () => systemAppearance?.removeEventListener("change", update);
}
