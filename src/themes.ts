export type ThemePalette = "light" | "dark" | "contrast";

type PaletteTokens = Record<string, string>;

type PaletteConfig = {
  dark: boolean;
  colors: PaletteTokens;
};

const palettes: Record<ThemePalette, PaletteConfig> = {
  light: {
    dark: false,
    colors: {
      background: "0 0% 100%",
      foreground: "240 10% 3.9%",
      card: "0 0% 100%",
      cardForeground: "240 10% 3.9%",
      popover: "0 0% 100%",
      popoverForeground: "240 10% 3.9%",
      primary: "221 83% 53%",
      primaryForeground: "0 0% 98%",
      secondary: "240 4.8% 95.9%",
      secondaryForeground: "240 5.9% 10%",
      muted: "240 4.8% 95.9%",
      mutedForeground: "240 3.8% 46.1%",
      accent: "221 83% 96%",
      accentForeground: "223 47% 11%",
      destructive: "0 84.2% 60.2%",
      destructiveForeground: "0 0% 98%",
      border: "240 5.9% 90%",
      input: "240 5.9% 90%",
      ring: "221 83% 53%",
    },
  },
  dark: {
    dark: true,
    colors: {
      background: "240 10% 3.9%",
      foreground: "0 0% 98%",
      card: "240 10% 3.9%",
      cardForeground: "0 0% 98%",
      popover: "240 10% 3.9%",
      popoverForeground: "0 0% 98%",
      primary: "210 80% 60%",
      primaryForeground: "222 47% 10%",
      secondary: "240 3.7% 15.9%",
      secondaryForeground: "0 0% 98%",
      muted: "240 3.7% 15.9%",
      mutedForeground: "240 5% 64.9%",
      accent: "210 80% 20%",
      accentForeground: "0 0% 98%",
      destructive: "0 62.8% 30.6%",
      destructiveForeground: "0 0% 98%",
      border: "240 3.7% 15.9%",
      input: "240 3.7% 15.9%",
      ring: "210 80% 60%",
    },
  },
  contrast: {
    dark: true,
    colors: {
      background: "225 10% 7%",
      foreground: "210 40% 98%",
      card: "225 12% 10%",
      cardForeground: "210 40% 98%",
      popover: "225 12% 12%",
      popoverForeground: "210 40% 98%",
      primary: "35 95% 62%",
      primaryForeground: "230 21% 15%",
      secondary: "210 28% 22%",
      secondaryForeground: "210 40% 98%",
      muted: "220 16% 20%",
      mutedForeground: "210 40% 85%",
      accent: "286 62% 60%",
      accentForeground: "230 21% 15%",
      destructive: "0 72% 50%",
      destructiveForeground: "210 40% 98%",
      border: "216 19% 26%",
      input: "216 19% 26%",
      ring: "35 95% 62%",
    },
  },
};

const clampNumber = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

export function applyTheme(paletteId: string, radius: number) {
  const root = document.documentElement;
  const palette = palettes[(paletteId as ThemePalette) || "light"] ?? palettes.light;

  Object.entries(palette.colors).forEach(([token, value]) => {
    root.style.setProperty(`--${token.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`)}`, value);
  });

  root.classList.toggle("dark", palette.dark);
  root.style.colorScheme = palette.dark ? "dark" : "light";
  root.style.setProperty("--radius", `${clampNumber(radius, 8, 16)}px`);

  // Fixed UI geometry and visual density for readability and predictable layout.
  root.style.setProperty("--ui-scale", "1");
  root.style.setProperty("--page-padding", "20px");
  root.style.setProperty("--sidebar-width", "248px");
  root.style.setProperty("--surface-opacity", "0.55");
  root.style.setProperty("--panel-opacity", "0.2");
  root.style.setProperty("--card-opacity", "0.96");
  root.style.setProperty("--aurora-opacity", "0.45");
  root.style.setProperty("--grain-opacity", "0.08");
  root.classList.remove("reduce-motion");
}
