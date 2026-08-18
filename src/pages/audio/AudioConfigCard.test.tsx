import { render, screen } from "@testing-library/react";
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
    osc: { enabled: false, targetIp: "127.0.0.1", targetPort: 9000, logMessages: false },
    rtp: { enabled: false, sessionName: "OSCMidi", port: 5004, remoteEnabled: false, remotes: [], logMessages: false },
    audio: {
      enabled: false,
      sampleRate: 48_000,
      bufferSize: 512,
      gainDb: 0,
      limiterEnabled: true,
      vstPluginId: plugin.id,
      vstScanPaths: [],
    },
    ui: { theme: "dark", themePalette: "default", cornerRadius: 8, autoStart: false, developerMode },
    logging: { enabled: true, verbose: false, logAllToFile: false },
  };
}

function renderCard(developerMode: boolean) {
  const noop = vi.fn(async () => undefined);
  render(
    <AudioConfigCard
      audioBackends={[]}
      audioDevices={[]}
      audioReloading={false}
      bridgeRunning={false}
      bufferMismatch={false}
      canOpenSelectedVstUi
      canOpenVstParameterFallback
      config={config(developerMode)}
      currentLatencyMs={null}
      midiMessagesPerSec={null}
      selectedPluginKindLabel="Instrument"
      selectedPluginStatusLabel="Compatible"
      selectedVstPlugin={plugin}
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
}

describe("AudioConfigCard VST diagnostics", () => {
  it("hides vendor, channel layout, and bridge diagnostics outside developer mode", () => {
    renderCard(false);

    expect(screen.queryByText("C.Hackl")).toBeNull();
    expect(screen.queryByText(/0 in \/ 2 out/)).toBeNull();
    expect(screen.queryByText("audio.vstAllVendors")).toBeNull();
    expect(screen.queryByText(/audio\.vstBridgeWorker/)).toBeNull();
    expect(screen.queryByText(/audio\.vstBridgeLatency/)).toBeNull();
    expect(screen.queryByText(/audio\.vstPluginLatency/)).toBeNull();
    expect(screen.queryByText(/audio\.vstBridgeUnderruns/)).toBeNull();
    expect(screen.getByText(plugin.path)).toBeTruthy();
  });

  it("shows the same technical details in developer mode", () => {
    renderCard(true);

    expect(screen.getByText("C.Hackl")).toBeTruthy();
    expect(screen.getByText(/0 in \/ 2 out/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstBridgeWorker/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstBridgeLatency/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstPluginLatency/)).toBeTruthy();
    expect(screen.getByText(/audio\.vstBridgeUnderruns/)).toBeTruthy();
  });
});
