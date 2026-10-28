import { afterEach, describe, expect, it, vi } from "vitest";
import { buttonVariants } from "./components/ui/Button";
import { ACCENT_CHOICES, DARK_BACKGROUNDS, LIGHT_BACKGROUNDS, applyTheme } from "./themes";

function rgbFor(token: string) {
  const [hue, saturation, lightness] = document.documentElement.style.getPropertyValue(`--${token}`).match(/[\d.]+/g)!.map(Number);
  const s = saturation / 100, l = lightness / 100;
  const chroma = (1 - Math.abs(2 * l - 1)) * s;
  const x = chroma * (1 - Math.abs((hue / 60) % 2 - 1));
  const m = l - chroma / 2;
  const sector = Math.floor(hue / 60) % 6;
  const rgb = [[chroma, x, 0], [x, chroma, 0], [0, chroma, x], [0, x, chroma], [x, 0, chroma], [chroma, 0, x]][sector];
  return rgb.map(c => c + m);
}
function luminance(token: string | number[]) {
  return (typeof token === "string" ? rgbFor(token) : token).map(c => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
    .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
}

it("covers feedback text and non-text controls in explicit and system palettes", () => {
  for (const [palette, resolved] of [["light", "light"], ["dark", "dark"], ["contrast", "contrast"], ["system", "light"], ["system", "dark"]]) {
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: resolved === "dark", addEventListener() {}, removeEventListener() {} })));
    for (const background of resolved === "light" ? LIGHT_BACKGROUNDS : DARK_BACKGROUNDS) {
      for (const accent of ACCENT_CHOICES) {
        const release = applyTheme(palette, 12, { accentColor: accent.id, lightBackground: background.id, darkBackground: background.id });
        const classes = buttonVariants();
        const activeAlpha = Number(classes.match(/active:bg-primary\/(\d+)/)?.[1] ?? 100) / 100;
        const disabledAlpha = Number(classes.match(/disabled:opacity-(\d+)/)?.[1] ?? 100) / 100;
        const blend = (token: string, alpha: number) => rgbFor(token).map((value, index) => value * alpha + rgbFor("card")[index] * (1 - alpha));
        for (const [state, foreground, stateBackground] of [
          ["active", rgbFor("primary-foreground"), blend("primary", activeAlpha)],
          ["disabled", blend("foreground", disabledAlpha), blend("control", disabledAlpha)],
          ["error-surface", rgbFor("destructive"), blend("destructive", 0.1)],
        ] as const) {
          const a = luminance(foreground), b = luminance(stateBackground);
          expect((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05),
            `${palette}/${resolved}/${background.id}/${accent.id}: ${state}`).toBeGreaterThanOrEqual(4.5);
        }
        for (const [text, surface, minimum] of [
          ["foreground", "background", 4.5], ["card-foreground", "card", 4.5],
          ["popover-foreground", "popover", 4.5], ["muted-foreground", "control", 4.5],
          ["secondary-foreground", "secondary", 4.5], ["accent-foreground", "accent", 4.5],
          ["primary-foreground", "primary", 4.5], ["destructive-foreground", "destructive", 4.5],
          ["success", "success-surface", 4.5], ["warning", "warning-surface", 4.5],
          ["destructive", "card", 4.5], ["primary", "card", 4.5],
          ["input", "control", 3], ["switch-off", "card", 3],
          ["ring", "card", 3], ["ring", "control", 3],
          ["success", "card", 3], ["muted-foreground", "card", 3],
          ["primary", "muted", 3],
        ] as const) {
          const a = luminance(text), b = luminance(surface);
          expect((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05),
            `${palette}/${resolved}/${background.id}/${accent.id}: ${text}/${surface}`).toBeGreaterThanOrEqual(minimum);
        }
        release();
      }
    }
  }
});

it("keeps readable text contrast across every accent and background combination", () => {
  for (const mode of ["light", "dark", "contrast"]) {
    for (const background of mode === "light" ? LIGHT_BACKGROUNDS : DARK_BACKGROUNDS) {
      for (const accent of ACCENT_CHOICES) {
        applyTheme(mode, 12, { accentColor: accent.id, lightBackground: background.id, darkBackground: background.id });
        for (const [text, surface] of [
          ["foreground", "background"], ["card-foreground", "card"],
          ["muted-foreground", "background"], ["muted-foreground", "card"],
          ["muted-foreground", "sidebar"], ["muted-foreground", "control"],
          ["primary-foreground", "primary"], ["accent-foreground", "accent"],
          ["primary", "card"], ["primary", "sidebar"],
        ]) {
          const a = luminance(text), b = luminance(surface);
          const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
          expect(ratio, `${mode}/${background.id}/${accent.id}: ${text} on ${surface}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  }
});

afterEach(() => {
  document.documentElement.removeAttribute("style");
  document.documentElement.className = "";
  vi.unstubAllGlobals();
});

describe("system appearance", () => {
  it("retains personalized colors when the system changes appearance", () => {
    const listeners = new Set<() => void>();
    const media = { matches: false,
      addEventListener: (_: string, listener: () => void) => listeners.add(listener),
      removeEventListener: (_: string, listener: () => void) => listeners.delete(listener) };
    vi.stubGlobal("matchMedia", vi.fn(() => media));
    const preferences = { accentColor: "purple", lightBackground: "white", darkBackground: "graphite" };
    const release = applyTheme("system", 12, preferences);
    const lightAccent = document.documentElement.style.getPropertyValue("--primary");
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("0 0% 100%");
    media.matches = true;
    listeners.forEach(listener => listener());
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("240 4% 16%");
    expect(document.documentElement.style.getPropertyValue("--primary")).not.toBe(lightAccent);
    release();
    expect(listeners.size).toBe(0);
  });

  it("falls back safely for unknown preferences from imported configurations", () => {
    applyTheme("light", 12);
    const defaults = document.documentElement.style.cssText;
    applyTheme("light", 12, { accentColor: "invalid", lightBackground: "invalid" });
    expect(document.documentElement.style.cssText).toBe(defaults);
  });
  it("follows the system appearance and releases its subscription", () => {
    const listeners = new Set<() => void>();
    const media = {
      matches: true,
      addEventListener: vi.fn((_event: string, listener: () => void) => listeners.add(listener)),
      removeEventListener: vi.fn((_event: string, listener: () => void) => listeners.delete(listener)),
    };
    vi.stubGlobal("matchMedia", vi.fn(() => media));
    const release = applyTheme("system", 12);
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.style.colorScheme).toBe("dark");
    media.matches = false;
    listeners.forEach(listener => listener());
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(document.documentElement.style.colorScheme).toBe("light");
    release();
    expect(listeners.size).toBe(0);
  });

  it("keeps an explicit appearance when the system uses another", () => {
    vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: true })));
    const release = applyTheme("light", 12);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    release();
  });
});
