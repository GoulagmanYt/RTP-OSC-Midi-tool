import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AppConfig } from "../api-types";
import { BridgeProvider, useBridge } from "./BridgeProvider";
import { LanguageProvider, useI18n } from "./LanguageProvider";

const mocks = vi.hoisted(() => ({ getConfig: vi.fn(), startBridge: vi.fn(), saveConfig: vi.fn() }));
vi.mock("../api", () => ({
  getConfig: mocks.getConfig, startBridge: mocks.startBridge, saveConfig: mocks.saveConfig,
  getStatus: vi.fn(async () => ({ running: true })),
  listMidiInputs: vi.fn(async () => []), listMidiOutputs: vi.fn(async () => []),
  listAudioBackends: vi.fn(async () => []), preflightCheck: vi.fn(async () => null),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

function Controls() {
  const { language, setLanguage, t } = useI18n();
  const { config, updateConfig } = useBridge();
  return (
    <>
      <button onClick={() => setLanguage(language === "fr" ? "en" : "fr")}>{t("language.english")}</button>
      <button onClick={() => updateConfig({ ui: { cornerRadius: 24 } })}>Edit</button>
      <output>{config?.ui.cornerRadius}</output>
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.setItem("oscMIDI.language", "fr");
  mocks.getConfig.mockResolvedValue({ audio: {}, ui: { autoStart: true, cornerRadius: 8 } } as AppConfig);
  mocks.startBridge.mockResolvedValue({ running: true });
  mocks.saveConfig.mockResolvedValue(true);
});

afterEach(() => vi.restoreAllMocks());

it.each([["fr-FR", "fr:Anglais"], ["en-US", "en:English"]])(
  "renders with browser language %s when stored preferences cannot be read", async (browserLanguage, expected) => {
    vi.spyOn(window.localStorage, "getItem").mockImplementation(() => {
      throw new DOMException("Storage unavailable", "SecurityError");
    });
    vi.spyOn(window.navigator, "language", "get").mockReturnValue(browserLanguage);
    function LanguageProbe() {
      const { language, t } = useI18n();
      return <output>{language}:{t("language.english")}</output>;
    }
    render(<LanguageProvider><LanguageProbe /></LanguageProvider>);
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe(expected));
  }
);

it("keeps the bridge and unsaved edits mounted while loading and switching language catalogs", async () => {
  render(<LanguageProvider><BridgeProvider><Controls /></BridgeProvider></LanguageProvider>);
  await waitFor(() => expect(mocks.startBridge).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByText("Edit"));
  await waitFor(() => expect(screen.getByRole("status").textContent).toBe("24"));
  const languageControl = screen.getByRole("button", { name: "Anglais" });
  fireEvent.click(languageControl);
  await waitFor(() => expect(screen.getByRole("button", { name: "English" })).toBe(languageControl));
  expect(screen.getByRole("status").textContent).toBe("24");
  fireEvent.click(languageControl);
  await waitFor(() => expect(screen.getByRole("button", { name: "Anglais" })).toBe(languageControl));
  expect(mocks.getConfig).toHaveBeenCalledOnce();
  expect(mocks.startBridge).toHaveBeenCalledOnce();
});
