import { isTauri } from "@tauri-apps/api/core";
import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

export const STARTUP_WINDOW = { width: 1600, height: 900, minWidth: 960, minHeight: 600 } as const;
type WorkArea = { x: number; y: number; width: number; height: number; scaleFactor: number };
type Bounds = { x: number; y: number; width: number; height: number };

/** Physical pixels preserve 16:9 at every DPI and respect the monitor's taskbar. */
export function fitStartupWindow(area: WorkArea): Bounds | null {
  if (!Object.values(area).every(Number.isFinite) || area.scaleFactor <= 0) return null;
  const scale = area.scaleFactor;
  const minWidth = Math.ceil(STARTUP_WINDOW.minWidth * scale);
  const minHeight = Math.ceil(STARTUP_WINDOW.minHeight * scale);
  if (area.width < minWidth || area.height < minHeight) return null;
  const margin = Math.max(0, Math.min(16 * scale, (area.width - minWidth) / 2, (area.height - minHeight) / 2));
  const step = Math.floor(Math.min(
    STARTUP_WINDOW.width * scale / 16, STARTUP_WINDOW.height * scale / 9,
    (area.width - 2 * margin) / 16, (area.height - 2 * margin) / 9,
  ));
  // On an unusually narrow monitor the existing minimum takes precedence.
  const width = Math.max(minWidth, step * 16);
  const height = Math.max(minHeight, step * 9);
  return {
    x: Math.round(area.x + (area.width - width) / 2),
    y: Math.round(area.y + (area.height - height) / 2),
    width, height,
  };
}

let initialization: Promise<void> | undefined;
/** Startup only: navigation, incoming events and manual resizing never refit the window. */
export function initializeStartupWindow(): Promise<void> {
  if (!isTauri()) return Promise.resolve();
  if (!initialization) {
    initialization = (async () => {
      const monitor = await currentMonitor();
      if (!monitor) return;
      const bounds = fitStartupWindow({
        x: monitor.workArea.position.x, y: monitor.workArea.position.y,
        width: monitor.workArea.size.width, height: monitor.workArea.size.height,
        scaleFactor: monitor.scaleFactor,
      });
      if (!bounds) return;
      const window = getCurrentWindow();
      await window.setSize(new PhysicalSize(bounds.width, bounds.height));
      await window.setPosition(new PhysicalPosition(bounds.x, bounds.y));
    })().catch(error => console.error("Failed to fit the startup window", error));
  }
  return initialization;
}
