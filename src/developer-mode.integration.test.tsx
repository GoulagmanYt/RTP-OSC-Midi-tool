import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { AppConfig, LogEntry, RuntimeMetrics, RuntimeStatus, VstPluginEntry } from "./api-types";
import { LanguageProvider } from "./providers/LanguageProvider";

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  open: vi.fn(),
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  toastError: vi.fn(),
}));

// Keep routing, providers, persistence, translations and all pages real. Only the
// desktop IPC, event bus and native file picker are replaced at their boundary.
vi.mock("./api-client", () => ({ invokeCommand: native.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    const handlers = native.listeners.get(event) ?? new Set();
    handlers.add(handler);
    native.listeners.set(event, handlers);
    return () => { handlers.delete(handler); };
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: native.open, save: vi.fn() }));
vi.mock("sonner", () => ({
  Toaster: () => null,
  toast: { error: native.toastError, success: vi.fn() },
}));

const instrument: VstPluginEntry = {
  id: "test-organ", name: "Test Organ", path: "C:\\VST\\Test Organ.dll",
  format: "VST2", kind: "instrument", architecture: "x86", availableArchitectures: ["x86"],
  vendor: "Private vendor", status: "compatible", supported: true, midiCompatible: true,
  hasEditor: true, channelLayout: "0 in / 2 out", hostingMode: "bridgedX86",
};

function configuration(developerMode: boolean, loggingEnabled = true): AppConfig {
  return {
    version: 4,
    midi: { inputDevice: "Keyboard", outputDevice: "Synth", channelFilter: null,
      thruEnabled: true, hotplug: true, routingProfiles: [], routingAssignments: [] },
    osc: { inputEnabled: false, listenIp: "127.0.0.1", listenPort: 9001,
      enabled: true, targetIp: "127.0.0.1", targetPort: 9000, logMessages: developerMode },
    rtp: { enabled: true, thruEnabled: false, sessionName: "Studio", port: 5004,
      remoteEnabled: false, remotes: [], logMessages: developerMode },
    audio: { enabled: true, backend: "WASAPI", device: "Speakers", sampleRate: 48_000,
      bufferSize: 512, gainDb: 0, limiterEnabled: true, gameMode: false,
      renderAheadBlocks: 1, dspThreadPriority: "mmcss", vstPluginId: instrument.id,
      vstPath: instrument.path, vstScanPaths: [], vstEqByPlugin: {} },
    ui: { theme: "light", themePalette: "light", cornerRadius: 12, autoStart: false, developerMode },
    logging: { enabled: loggingEnabled, verbose: developerMode, logAllToFile: developerMode },
  };
}

const runtime: RuntimeStatus = {
  running: true, midiInput: "Keyboard", midiOutput: "Synth", oscTarget: "127.0.0.1:9000",
  rtpActive: true, rtpBoundPort: 5004, rtpAdvertisedAddresses: ["192.168.1.20"],
  audioRunning: true, vstLoaded: true, vstMidiCompatible: true, vstWorkerState: "ready",
  vstWorkerRestarts: 0, audioBackend: "WASAPI", audioDevice: "Speakers",
  audioSampleRate: 48_000, audioBufferSize: 512, audioRequestedBufferSize: 512,
  audioStreamBufferSize: 512, audioLatencyMs: 21.33, audioXruns: 0,
};

const metrics: RuntimeMetrics = {
  audioXruns: 0, audioMidiDrops: 0, audioLockMisses: 0, audioEmergencyResets: 0,
  audioCallbackOverBudgetCount: 0, vstWorkerState: "ready", vstWorkerRestarts: 0,
  midiMessagesPerSec: 12, oscMessagesPerSec: 12, bridgeQueueDepth: 0,
  bridgeQueueMaxDepth: 1, bridgeMessagesIn: 20, bridgeMessagesOut: 20,
  bridgeMessagesDropped: 0, rtpMidiDrops: 0, reliablePlaybackMessagesIn: 0,
  reliablePlaybackMessagesOut: 0, reliablePlaybackDropped: 0, reliablePlaybackMaxLateUs: 0,
};

let persisted: AppConfig;
let imported: AppConfig;
let rejectSave = false;
const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

beforeEach(() => {
  vi.clearAllMocks();
  native.listeners.clear();
  rejectSave = false;
  imported = configuration(false);
  localStorage.setItem("oscMIDI.language", "en");
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("matchMedia", vi.fn(() => ({
    matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn(),
  })));
  HTMLElement.prototype.scrollIntoView = vi.fn();
  native.open.mockResolvedValue("C:/incoming/config.yaml");
  native.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "get_config": return structuredClone(persisted);
      case "save_config":
        if (rejectSave) throw new Error("disk write rejected");
        persisted = structuredClone(args!.config as AppConfig);
        return;
      case "import_config": persisted = structuredClone(imported); return structuredClone(imported);
      case "get_status": return structuredClone(runtime);
      case "preflight_check": return { midiInOk: true, midiOutOk: true, audioBackendOk: true, rtpPortOk: true, messages: [] };
      case "list_midi_inputs": return ["Keyboard"];
      case "list_midi_outputs": return ["Synth"];
      case "list_audio_backends": return ["WASAPI", "ASIO"];
      case "list_audio_devices": return [{ name: "Speakers", id: "speakers" }];
      case "list_vst_plugins": case "refresh_vst_plugins": return [structuredClone(instrument)];
      case "get_rtp_participants": case "refresh_rtp_sessions": case "list_vst_parameters": return [];
      case "get_app_paths": return { configDir: "C:\\OSCMidi", logDir: "C:\\OSCMidi", logFile: "C:\\OSCMidi\\app.log" };
      case "open_vst_ui": case "close_vst_ui": case "set_audio_limiter": case "set_master_gain": return;
      default: throw new Error(`Unexpected native command in integration test: ${command}`);
    }
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
});

async function mount(developerMode: boolean, path = "/", enabled = true) {
  persisted = configuration(developerMode, enabled);
  window.history.replaceState({}, "", path);
  render(<LanguageProvider><App /></LanguageProvider>);
  await screen.findByRole("navigation", { name: "Navigation" });
  await waitFor(() => expect(native.listeners.get("log:entry")?.size).toBe(1));
}

async function visit(label: string, title = label) {
  fireEvent.click(within(screen.getByRole("navigation")).getByRole("link", { name: label }));
  await screen.findByRole("heading", { level: 1, name: title });
}

async function emit(event: string, payload: LogEntry | RuntimeMetrics) {
  await act(async () => {
    for (const listener of native.listeners.get(event) ?? []) listener({ payload });
    // The real log buffer batches state publication with requestAnimationFrame.
    await new Promise<void>((resolve) => window.setTimeout(resolve, 25));
  });
}

function assertNormalLogging(config: AppConfig, enabled: boolean) {
  expect(config.ui.developerMode).toBe(false);
  expect(config.logging).toEqual({ enabled, verbose: false, logAllToFile: false });
  expect(config.osc.logMessages).toBe(false);
  expect(config.rtp.logMessages).toBe(false);
}

describe("developer mode across the real application", () => {
  it.each([false, true])("keeps routes, navigation and essential controls coherent (developer=%s)", async (developerMode) => {
    await mount(developerMode);
    await screen.findByRole("heading", { level: 1, name: "Dashboard" });
    await visit("Routing & MIDI");
    expect(document.querySelector("#midi-input")).toBeTruthy();
    await visit("RTP-MIDI");
    expect(screen.getByRole("switch", { name: "Enable RTP-MIDI server" })).toBeTruthy();
    expect(document.querySelector("#rtp-verbose")).toBeNull();
    await visit("OSC", "OSC Configuration");
    expect(document.querySelector("#osc-ip")).toBeTruthy();
    expect(document.querySelector("#osc-log-messages")).toBeNull();
    expect(document.querySelector("#osc-verbose")).toBeNull();
    await visit("Audio & VST");
    await screen.findByRole("combobox", { name: "VST instrument" });
    expect(screen.getByRole("combobox", { name: "Sample Rate" })).toBeTruthy();
    expect(screen.getByRole("combobox", { name: "Buffer Size" })).toBeTruthy();
    expect(Boolean(document.querySelector("#audio-dsp-priority"))).toBe(developerMode);
    expect(Boolean(screen.queryByText(instrument.path))).toBe(developerMode);
    expect((screen.getByRole("button", { name: "EQ" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Open VST UI" }));
    await waitFor(() => expect(native.invoke).toHaveBeenCalledWith("open_vst_ui"));
    await visit("Settings");
    expect(screen.getByRole("switch", { name: "Developer Mode" }).getAttribute("aria-checked")).toBe(String(developerMode));
    expect(Boolean(document.querySelector("#settings-verbose"))).toBe(developerMode);
    expect(Boolean(within(screen.getByRole("navigation")).queryByRole("link", { name: "Logs" }))).toBe(developerMode);

    fireEvent.keyDown(document, { key: "k", ctrlKey: true });
    const commands = await screen.findByRole("dialog", { name: "Search and commands" });
    expect(Boolean(within(commands).queryByRole("option", { name: "Logs" }))).toBe(developerMode);
    fireEvent.keyDown(commands, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    await act(async () => {
      window.history.pushState({}, "", "/logs");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    await screen.findByRole("heading", { level: 1, name: developerMode ? "System Logs" : "Dashboard" });
    expect(window.location.pathname).toBe(developerMode ? "/logs" : "/");
  });

  it.each([true, false])("hides retained and late debug entries while preserving history and logging.enabled=%s", async (enabled) => {
    await mount(true, "/", enabled);
    const previous: LogEntry = { level: "debug", timestamp: "12:00", message: "private worker diagnostic before switch" };
    const late: LogEntry = { level: "debug", timestamp: "12:01", message: "private IPC diagnostic after switch" };
    await emit("log:entry", previous);
    expect(screen.getByText(previous.message)).toBeTruthy();
    await visit("Settings");
    fireEvent.click(screen.getByRole("switch", { name: "Developer Mode" }));
    await waitFor(() => assertNormalLogging(persisted, enabled));
    await visit("Dashboard");
    await emit("log:entry", late);
    expect(screen.queryByText(previous.message)).toBeNull();
    expect(screen.queryByText(late.message)).toBeNull();
    await emit("runtime:metrics", metrics);
    await emit("runtime:metrics", { ...metrics, audioLockMisses: 1 });
    expect(native.toastError).toHaveBeenLastCalledWith("Audio playback was interrupted. Increase the safety margin in audio settings if it happens again.");

    await visit("Settings");
    fireEvent.click(screen.getByRole("switch", { name: "Developer Mode" }));
    await waitFor(() => expect(persisted.ui.developerMode).toBe(true));
    await visit("Logs", "System Logs");
    expect(screen.getByText(previous.message)).toBeTruthy();
    expect(screen.getByText(late.message)).toBeTruthy();
    expect(persisted.logging.enabled).toBe(enabled);
  });

  it("normalizes a normal-mode import before publishing it and preserves the imported functional settings", async () => {
    await mount(true, "/settings");
    imported = configuration(false, false);
    imported.logging.verbose = true;
    imported.logging.logAllToFile = true;
    imported.osc.logMessages = true;
    imported.rtp.logMessages = true;
    imported.audio.gainDb = -9;
    fireEvent.click(await screen.findByRole("button", { name: "Import YAML" }));
    await waitFor(() => expect(screen.getByRole("switch", { name: "Developer Mode" }).getAttribute("aria-checked")).toBe("false"));
    expect(document.querySelector("#settings-verbose")).toBeNull();
    expect(within(screen.getByRole("navigation")).queryByRole("link", { name: "Logs" })).toBeNull();
    await visit("Audio & VST");
    expect((document.querySelector("#audio-master-gain") as HTMLInputElement).value).toBe("-9");
    await visit("Settings");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => assertNormalLogging(persisted, false));
    expect(persisted.audio.gainDb).toBe(-9);
    expect(persisted.audio.vstPluginId).toBe(instrument.id);
  });

  it("restores developer navigation and logging controls when saving the mode change fails", async () => {
    await mount(true, "/settings");
    const initial = structuredClone(persisted);
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    rejectSave = true;
    fireEvent.click(await screen.findByRole("switch", { name: "Developer Mode" }));
    expect(within(screen.getByRole("navigation")).queryByRole("link", { name: "Logs" })).toBeNull();
    await waitFor(() => expect(native.toastError).toHaveBeenCalledWith("disk write rejected"));
    await waitFor(() => expect(screen.getByRole("switch", { name: "Developer Mode" }).getAttribute("aria-checked")).toBe("true"));
    for (const label of ["Verbose mode", "OSC logs", "RTP-MIDI logs", "Log everything to file"]) {
      expect(screen.getByRole("switch", { name: label }).getAttribute("aria-checked")).toBe("true");
    }
    expect(persisted).toEqual(initial);
    await visit("Logs", "System Logs");
    expect(window.location.pathname).toBe("/logs");
    expect(error).toHaveBeenCalled();
  });

  it("reports the first incident after a clean baseline and returns to stable when counters reset", async () => {
    await mount(true);
    await emit("runtime:metrics", metrics);
    expect(screen.getByText("Stable")).toBeTruthy();
    await emit("runtime:metrics", { ...metrics, audioXruns: 1 });
    expect(screen.getByText("Dropouts detected")).toBeTruthy();
    await emit("runtime:metrics", metrics);
    expect(screen.getByText("Stable")).toBeTruthy();
    expect(screen.queryByText("Dropouts detected")).toBeNull();
  });
});
