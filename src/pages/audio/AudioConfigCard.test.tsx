import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppConfig, RuntimeStatus, VstPluginEntry } from "../../api-types";
import { AudioConfigCard } from "./AudioConfigCard";

const plugin: VstPluginEntry = {
  id: "church-organ-x86",
  name: "Church Organ",
  path: "C:\\Program Files\\VstPlugins\\Church Organ.dll",
  format: "VST2",
  kind: "instrument",
  architecture: "x86",
  availableArchitectures: ["x86"],
  vendor: "C.Hackl",
  status: "compatible",
  supported: true,
  midiCompatible: true,
  hasEditor: true,
  channelLayout: "0 in / 2 out",
  hostingMode: "bridgedX86",
};

const status = {
  x86WorkerAlive: true,
  bridgeLatencySamples: 512,
  pluginLatencySamples: 0,
  x86BridgeUnderruns: 191,
  x86BridgeOverruns: 189,
} as RuntimeStatus;

function config(developerMode: boolean): AppConfig {
  return {
    version: 1,
    midi: { thruEnabled: false, hotplug: true, routingProfiles: [], routingAssignments: [] },
    osc: { inputEnabled: false, listenIp: "127.0.0.1", listenPort: 9001, enabled: false, targetIp: "127.0.0.1", targetPort: 9000, logMessages: false },
    rtp: { thruEnabled: false, enabled: false, sessionName: "OSCMidi", port: 5004, remoteEnabled: false, remotes: [], logMessages: false },
    audio: {
      enabled: false,
      sampleRate: 48_000,
      bufferSize: 512,
      gainDb: 0,
      limiterEnabled: true,
      vstPluginId: plugin.id,
      vstScanPaths: [],
      vstEqByPlugin: {},
    },
    ui: { theme: "dark", themePalette: "default", cornerRadius: 8, autoStart: false, developerMode },
    logging: { enabled: true, verbose: false, logAllToFile: false },
  };
}

function renderCard(developerMode: boolean, vstEqEnabled = false) {
  const noop = vi.fn(async () => undefined);
  const card = (mode: boolean) => (
    <AudioConfigCard
      audioBackends={[]}
      audioDevices={[]}
      audioReloading={false}
      bridgeRunning={false}
      bufferMismatch={false}
      canOpenSelectedVstUi
      canOpenVstParameterFallback
      canOpenVstEq
      config={config(mode)}
      currentLatencyMs={null}
      midiMessagesPerSec={null}
      selectedPluginKindLabel="Instrument"
      selectedPluginStatusLabel="Compatible"
      selectedVstPlugin={plugin}
      vstEqEnabled={vstEqEnabled}
      status={status}
      streamBufferSize={512}
      t={(key, values) => values?.layout ? `${key}: ${values.layout}` : key}
      vstMidiCompatible
      vstPlugins={[plugin]}
      vstPluginsLoading={false}
      vstUiOpen={false}
      activeBufferSize={512}
      activeSampleRate={48_000}
      requestedBufferSize={512}
      onBackendChange={noop}
      onCloseVstUi={noop}
      onGainChange={noop}
      onLimiterToggle={noop}
      onOpenVstParameters={noop}
      onOpenVstEq={noop}
      onOpenVstUi={noop}
      onPingAudio={noop}
      onRefreshVstPluginsList={noop}
      onReloadVst={noop}
      onSaveConfig={noop}
      onSelectVst={noop}
      onRetestVst={noop}
      onOpenVstFolder={noop}
      onToggleAudio={noop}
      onUpdateAudioConfig={noop}
    />
  );
  const view = render(card(developerMode));
  return { action: noop, rerenderMode: (mode: boolean) => view.rerender(card(mode)) };
}

describe("AudioConfigCard VST diagnostics", () => {
  it("preserves separators while typing multiple scan paths and commits on blur", async () => {
    const { action: update } = renderCard(false);
    const librarySettings = screen.getByText("audio.vstLibrarySettings").closest("details")!;
    expect(librarySettings.open).toBe(false);
    fireEvent.click(screen.getByText("audio.vstLibrarySettings"));
    expect(librarySettings.open).toBe(true);
    const input = screen.getByLabelText("audio.vstCustomPaths") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "C:\\VST1;" } });
    expect(input.value).toBe("C:\\VST1;");
    expect(update).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: "C:\\VST1; C:\\VST2" } });
    expect(input.value).toBe("C:\\VST1; C:\\VST2");
    await act(async () => { fireEvent.blur(input); });
    expect(update).toHaveBeenCalledWith({ vstScanPaths: ["C:\\VST1", "C:\\VST2"] });
  });
  it("enables game mode for legacy configs and submits an additive toggle", () => {
    const { action: update } = renderCard(false);
    const toggle = screen.getByRole("switch", { name: /audio.gameMode/ });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(toggle);
    expect(update).toHaveBeenCalledWith({ gameMode: false });
  });

  it("hides vendor, channel layout, and bridge diagnostics outside developer mode", () => {
    renderCard(false);

    expect(screen.queryByText("C.Hackl")).toBeNull();
    expect(screen.queryByText(/0 in \/ 2 out/)).toBeNull();
    expect(screen.queryByText("audio.vstAllVendors")).toBeNull();
    expect(screen.queryByText(/audio\.vstBridgeWorker/)).toBeNull();
    expect(screen.queryByText(/audio\.vstBridgeLatency/)).toBeNull();
    expect(screen.queryByText(/audio\.vstPluginLatency/)).toBeNull();
    expect(screen.queryByText(/audio\.vstBridgeUnderruns/)).toBeNull();
    expect(screen.queryByText(plugin.path)).toBeNull();
    expect(screen.queryByText("audio.dspPriority")).toBeNull();
    expect(screen.queryByText("audio.estimatedLatency")).toBeNull();
    expect(screen.queryByText("audio.vstFormats")).toBeNull();
    expect(screen.queryByText("audio.vstModeBridged")).toBeNull();
    expect(screen.queryByText("audio.vstRetest")).toBeNull();
    expect(screen.queryByRole("combobox", { name: "audio.vstAllArchitectures" })).toBeNull();
    expect(screen.getByRole("combobox", { name: "audio.vstInstrumentLabel" }).textContent).toBe(plugin.name);
  });

  it("shows the same technical details in developer mode", () => {
    renderCard(true);

    expect(screen.getByText("C.Hackl")).toBeTruthy();
    expect(screen.getByText(/0 in \/ 2 out/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstBridgeWorker/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstBridgeLatency/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstPluginLatency/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstBridgeUnderruns/)).toBeTruthy();
    expect(screen.getByText(plugin.path)).toBeTruthy();
    expect(screen.getByText("audio.dspPriority")).toBeTruthy();
    expect(screen.getByText("audio.estimatedLatency")).toBeTruthy();
    expect(screen.getByText("audio.vstModeBridged")).toBeTruthy();
    expect(screen.getByRole("button", { name: "audio.vstRetest" })).toBeTruthy();
  });

  it("keeps essential audio and instrument actions available without diagnostics", () => {
    const { action } = renderCard(false);
    for (const label of ["audio.sampleRate", "audio.bufferSize", "audio.deviceLabel"]) {
      expect(screen.getByRole("combobox", { name: label })).toBeTruthy();
    }
    for (const label of ["audio.eq.open", "audio.openVstUi", "audio.openVstParameters", "common.refresh"]) {
      const button = screen.getByRole("button", { name: label });
      expect((button as HTMLButtonElement).disabled).toBe(false);
      fireEvent.click(button);
    }
    expect(action).toHaveBeenCalledTimes(4);
    const advanced = screen.getByText("audio.advancedSettings").closest("details")!;
    expect(advanced.open).toBe(false);
    fireEvent.click(screen.getByText("audio.advancedSettings"));
    expect(advanced.open).toBe(true);
    expect(within(advanced).getByRole("combobox", { name: "audio.renderAhead" })).toBeTruthy();
    expect(within(advanced).getByText("audio.renderAheadSimpleHint")).toBeTruthy();
  });

  it("removes diagnostics and ignores the hidden architecture filter when leaving developer mode", () => {
    const { rerenderMode } = renderCard(true);
    fireEvent.keyDown(screen.getByRole("combobox", { name: "audio.vstAllArchitectures" }), { key: " " });
    fireEvent.click(screen.getByRole("option", { name: "x64" }));
    rerenderMode(false);
    expect(screen.queryByText("audio.developerDiagnostics")).toBeNull();
    expect(screen.queryByText(plugin.path)).toBeNull();
    expect(screen.queryByText("audio.vstModeBridged")).toBeNull();
    fireEvent.keyDown(screen.getByRole("combobox", { name: "audio.vstInstrumentLabel" }), { key: " " });
    expect(screen.getByRole("option", { name: plugin.name }).getAttribute("aria-disabled")).not.toBe("true");
  });

  it("shows the active badge only for an enabled VST EQ profile", () => {
    renderCard(false, true);

    expect(screen.getByText("audio.eq.active")).toBeTruthy();
  });
});
