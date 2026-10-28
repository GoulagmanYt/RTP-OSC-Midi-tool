import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { TopBar } from "./TopBar";
import desktopConfig from "../../../src-tauri/tauri.conf.json";
const updateConfig = vi.fn();
vi.mock("../../providers/BridgeProvider", () => ({
  useBridge: () => ({ config: { ui: { developerMode: true } }, status: {}, updateConfig }),
}));
vi.mock("../../providers/LanguageProvider", () => ({
  useI18n: () => ({ t: (key: string) => key, language: "fr" }),
}));
it("shows developer mode and provides a direct exit even without the sidebar", () => {
  render(<TopBar sidebarVisible={false} onToggleSidebar={vi.fn()} onOpenCommand={vi.fn()} />);
  expect(desktopConfig.app.windows[0].decorations).toBe(false);
  expect(screen.getByRole("group", { name: "window.controls" })).toBeTruthy();
  for (const action of ["minimize", "maximize", "close"]) {
    expect(screen.getByRole("button", { name: `window.${action}` })).toBeTruthy();
  }
  expect(screen.getByText("ui.developerActive")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "ui.disableDeveloper" }));
  expect(updateConfig).toHaveBeenCalledWith({ ui: { developerMode: false } });
});
