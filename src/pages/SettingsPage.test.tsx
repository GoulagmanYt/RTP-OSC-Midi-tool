import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig } from "../api-types";
import SettingsPage from "./SettingsPage";

const mocks = vi.hoisted(() => ({
  config: { ui: { developerMode: true }, logging: { enabled: true } } as AppConfig,
  updateConfig: vi.fn(), getAppPaths: vi.fn(), save: vi.fn(), join: vi.fn(),
  exportConfig: vi.fn(), exportDiagnostics: vi.fn(),
}));
vi.mock("../providers/BridgeProvider", () => ({ useBridge: () => ({
  config: mocks.config, updateConfig: mocks.updateConfig,
  saveConfig: vi.fn(), clearLogs: vi.fn(), reloadConfig: vi.fn(),
}) }));
vi.mock("../providers/LanguageProvider", () => ({ useI18n: () => ({ t: translate }) }));
vi.mock("../api", () => ({
  getAppPaths: mocks.getAppPaths, exportConfig: mocks.exportConfig,
  exportDiagnostics: mocks.exportDiagnostics, clearLogFile: vi.fn(),
  importConfig: vi.fn(), openAppDir: vi.fn(), resetConfigDefaults: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save, open: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ join: mocks.join }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
const translate = (key: string) => key;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.updateConfig.mockResolvedValue(undefined);
  mocks.getAppPaths.mockResolvedValue({ configDir: "/home/user/config", logFile: "/home/user/config/app.log", logDir: "/home/user/config" });
  mocks.join.mockImplementation(async (dir: string, file: string) => `${dir}/${file}`);
  mocks.save.mockResolvedValue("/chosen/export");
});

it.each([true, false])("preserves logging.enabled=%s when leaving developer mode", async (enabled) => {
  mocks.config = { ui: { developerMode: true }, logging: { enabled }, osc: { logMessages: false } } as AppConfig;
  render(<SettingsPage />);
  fireEvent.click(screen.getAllByRole("switch")[0]);
  await waitFor(() => expect(mocks.updateConfig).toHaveBeenCalled());
  const patch = mocks.updateConfig.mock.calls[0][0];
  expect(patch.ui.developerMode).toBe(false);
  expect(patch.logging).toEqual({ verbose: false, logAllToFile: false });
});

it.each([
  ["settings.files.exportYaml", "config.yaml"],
  ["settings.files.exportDiagnostics", "diagnostic.json"],
])("uses the native path join for %s", async (label, filename) => {
  render(<SettingsPage />);
  await waitFor(() => expect(screen.getByDisplayValue("/home/user/config")).toBeTruthy());
  fireEvent.click(screen.getByRole("button", { name: label }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalled());
  expect(mocks.join).toHaveBeenCalledWith("/home/user/config", filename);
  expect(mocks.save.mock.calls[0][0].defaultPath).toBe(`/home/user/config/${filename}`);
});
