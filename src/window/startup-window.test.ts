import { beforeEach, expect, it, vi } from "vitest";
import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import desktopConfig from "../../src-tauri/tauri.conf.json";
import desktopCapability from "../../src-tauri/capabilities/desktop.json";
import { fitStartupWindow, STARTUP_WINDOW } from "./startup-window";

const native = vi.hoisted(() => ({
  isTauri: vi.fn(() => true), currentMonitor: vi.fn(),
  setSize: vi.fn().mockResolvedValue(undefined), setPosition: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: native.isTauri }));
vi.mock("@tauri-apps/api/window", () => ({
  currentMonitor: native.currentMonitor,
  getCurrentWindow: () => ({ setSize: native.setSize, setPosition: native.setPosition }),
}));
beforeEach(() => { vi.clearAllMocks(); vi.resetModules(); native.isTauri.mockReturnValue(true); });
it("starts with a 1600 by 900 widescreen default and keeps resizing available", () => {
  const window = desktopConfig.app.windows[0];
  expect(window).toMatchObject(STARTUP_WINDOW);
  expect(window.width).toBe(1600);
  expect(window.height).toBe(900);
  expect(window.width / window.height).toBeCloseTo(16 / 9);
  expect(window.resizable).toBe(true);
  expect(desktopCapability.permissions).toEqual(expect.arrayContaining([
    "core:window:allow-set-size", "core:window:allow-set-position",
  ]));
});
it.each([
  [{ x: 0, y: 0, width: 1920, height: 1040, scaleFactor: 1 }, { x: 160, y: 70, width: 1600, height: 900 }],
  [{ x: 0, y: 0, width: 1920, height: 1008, scaleFactor: 1.5 }, { x: 112, y: 27, width: 1696, height: 954 }],
  [{ x: 0, y: 0, width: 3840, height: 2080, scaleFactor: 2 }, { x: 320, y: 140, width: 3200, height: 1800 }],
  [{ x: -1920, y: 48, width: 1920, height: 1032, scaleFactor: 1 }, { x: -1760, y: 114, width: 1600, height: 900 }],
])("fits 16:9 inside the work area without covering the taskbar (%j)", (area, expected) => {
  const bounds = fitStartupWindow(area)!;
  expect(bounds).toEqual(expected);
  expect(bounds.width / bounds.height).toBeCloseTo(16 / 9);
  expect(bounds.x).toBeGreaterThanOrEqual(area.x);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(area.y + area.height);
});
it("honors the minimum on a narrow monitor where 16:9 cannot fit", () => {
  expect(fitStartupWindow({ x: 0, y: 0, width: 1024, height: 728, scaleFactor: 1 }))
    .toEqual({ x: 16, y: 64, width: 992, height: 600 });
});
it("leaves unsupported or invalid monitor sizes to the window manager", () => {
  expect(fitStartupWindow({ x: 0, y: 0, width: 800, height: 600, scaleFactor: 1 })).toBeNull();
  expect(fitStartupWindow({ x: 0, y: 0, width: 1920, height: 1080, scaleFactor: NaN })).toBeNull();
});
it("configures the native window only once, without a resize listener or polling", async () => {
  native.currentMonitor.mockResolvedValue({
    scaleFactor: 1, workArea: { position: new PhysicalPosition(0, 0), size: new PhysicalSize(1920, 1040) },
  });
  const { initializeStartupWindow } = await import("./startup-window");
  await Promise.all([initializeStartupWindow(), initializeStartupWindow()]);
  expect(native.currentMonitor).toHaveBeenCalledOnce();
  expect(native.setSize).toHaveBeenCalledOnce();
  expect(native.setSize.mock.calls[0][0]).toMatchObject({ width: 1600, height: 900 });
  expect(native.setPosition.mock.calls[0][0]).toMatchObject({ x: 160, y: 70 });
});
it("does not invoke native window APIs in browser previews", async () => {
  native.isTauri.mockReturnValue(false);
  const { initializeStartupWindow } = await import("./startup-window");
  await initializeStartupWindow();
  expect(native.currentMonitor).not.toHaveBeenCalled();
});
