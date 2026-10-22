import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig } from "../api-types";
import SettingsPage from "./SettingsPage";

const mocks = vi.hoisted(() => ({
  config: { ui: { developerMode: true }, logging: { enabled: true } } as AppConfig,
  updateConfig: vi.fn(), getAppPaths: vi.fn(), save: vi.fn(), join: vi.fn(),
  exportConfig: vi.fn(), exportDiagnostics: vi.fn(),
  importConfig: vi.fn(), resetConfigDefaults: vi.fn(), open: vi.fn(), reloadConfig: vi.fn(),
}));
vi.mock("../providers/BridgeProvider", () => ({ useBridge: () => ({
  config: mocks.config, updateConfig: mocks.updateConfig,
  saveConfig: vi.fn(), clearLogs: vi.fn(), reloadConfig: mocks.reloadConfig,
}) }));
vi.mock("../providers/LanguageProvider", () => ({ useI18n: () => ({ t: translate }) }));
vi.mock("../api", () => ({
  getAppPaths: mocks.getAppPaths, exportConfig: mocks.exportConfig,
  exportDiagnostics: mocks.exportDiagnostics, clearLogFile: vi.fn(),
  importConfig: mocks.importConfig, openAppDir: vi.fn(), resetConfigDefaults: mocks.resetConfigDefaults,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save, open: mocks.open }));
vi.mock("@tauri-apps/api/path", () => ({ join: mocks.join }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
const translate = (key: string) => key;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.config = { ui: { developerMode: true }, logging: { enabled: true }, osc: { logMessages: false } } as AppConfig;
  mocks.updateConfig.mockResolvedValue(undefined);
  mocks.getAppPaths.mockResolvedValue({ configDir: "/home/user/config", logFile: "/home/user/config/app.log", logDir: "/home/user/config" });
  mocks.join.mockImplementation(async (dir: string, file: string) => `${dir}/${file}`);
  mocks.save.mockResolvedValue("/chosen/export");
});

it("lets people follow the system appearance", async () => {
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: true })));
  try {
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("radio", { name: "settings.theme.palette.system" }));
    await waitFor(() => expect(mocks.updateConfig).toHaveBeenCalledWith({ ui: { themePalette: "system", theme: "dark" } }));
  } finally {
    vi.unstubAllGlobals();
  }
});

it.each([
  ["settings.theme.accent.purple", "accentColor", "purple"],
  ["settings.theme.background.white", "lightBackground", "white"],
  ["settings.theme.background.graphite", "darkBackground", "graphite"],
])("saves the selected appearance preference %s", async (label, field, value) => {
  render(<SettingsPage />);
  fireEvent.click(screen.getByRole("radio", { name: label }));
  await waitFor(() => expect(mocks.updateConfig).toHaveBeenCalledWith({ ui: { [field]: value } }));
});

it.each(["reset", "import"])("publishes the accepted %s snapshot directly instead of querying configuration again", async (operation) => {
  const accepted = { version: 4, ui: { developerMode: false } } as AppConfig;
  mocks.resetConfigDefaults.mockResolvedValue(accepted);
  mocks.importConfig.mockResolvedValue(accepted);
  mocks.open.mockResolvedValue("C:/import/config.yaml");
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  try {
    render(<SettingsPage />);
    const label = operation === "reset" ? "common.reset" : "settings.files.importYaml";
    fireEvent.click(screen.getByRole("button", { name: label }));
    await waitFor(() => expect(mocks.reloadConfig).toHaveBeenCalledWith(accepted));
  } finally {
    confirm.mockRestore();
  }
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
