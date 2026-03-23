import { invoke } from "@tauri-apps/api/core";

export type Theme = "light" | "dark";

export type Config = {
  oscTargetIp: string;
  oscTargetPort: number;
  oscEnabled: boolean;
  channelFilter?: number | null;
  midiIn?: string | null;
  midiOut?: string | null;
  midiThru: boolean;
  verbose: boolean;
  logOsc: boolean;
  logRtp: boolean;
  liveLogs: boolean;
  rtpEnabled: boolean;
  rtpSessionName: string;
  rtpPort: number;
  rtpRemoteEnabled: boolean;
  rtpRemoteHost: string;
  rtpRemotePort: number;
  rtpRemotes: RtpRemoteEntry[];
  routingProfiles: RoutingProfile[];
  routingAssignments: RoutingAssignment[];
  hotplug: boolean;
  themePreset: string;
  theme: Theme;
  themePalette?: string;
  uiAccent?: string;
  cornerRadius?: number;
  uiScale?: number;
  contentPadding?: number;
  sidebarWidth?: number;
  surfaceOpacity?: number;
  cardOpacity?: number;
  auroraIntensity?: number;
  grainIntensity?: number;
  reduceMotion?: boolean;
  logsEnabled?: boolean;
  autoStart: boolean;
  alwaysOnTop: boolean;
  audioEnabled: boolean;
  audioBackend?: string | null;
  audioDevice?: string | null;
  audioSampleRate: number;
  audioBufferSize: number;
  audioGainDb: number;
  audioLimiterEnabled: boolean;
  logAllToFile: boolean;
  vstPath?: string | null;
  avatarPath?: string | null;
  avatarOffsetX?: number;
  avatarOffsetY?: number;
  avatarScale?: number;
};

export type BridgeStatus = {
  running: boolean;
  midiIn?: string | null;
  midiOut?: string | null;
  oscTarget: string;
  rtpActive: boolean;
  rtpBoundPort?: number | null;
  lastError?: string | null;
  vstLoaded?: boolean;
  audioRunning?: boolean;
  audioLatencyMs?: number | null;
  audioBackend?: string | null;
  audioDevice?: string | null;
  audioSampleRate?: number | null;
  audioBufferSize?: number | null;
  audioRequestedBufferSize?: number | null;
  audioStreamBufferSize?: number | null;
  audioBufferMismatch?: boolean | null;
  vstMidiCompatible?: boolean | null;
  audioXruns?: number | null;
  audioLimiterEnabled?: boolean | null;
};

export type BridgeMetrics = {
  audioPeakL?: number | null;
  audioPeakR?: number | null;
  audioLatencyMs?: number | null;
  audioXruns?: number | null;
  audioMidiDrops?: number | null;
  audioLockMisses?: number | null;
  audioEmergencyResets?: number | null;
  midiMessagesPerSec: number;
  oscMessagesPerSec: number;
};

export type PreflightReport = {
  midiInOk: boolean;
  midiOutOk: boolean;
  audioBackendOk: boolean;
  rtpPortOk: boolean;
  messages: string[];
};

export type RtpSessionInfo = {
  name: string;
  host: string;
  port: number;
  addresses: string[];
};

export type RtpParticipantInfo = {
  name: string;
  addr: string;
};

export type RtpRemoteEntry = {
  id: string;
  name: string;
  host: string;
  port: number;
  autoConnect: boolean;
};

export type RoutingMapping = {
  from: number;
  to: number;
};

export type RoutingProfile = {
  id: string;
  name: string;
  enabled: boolean;
  channelFilter?: number | null;
  noteMin?: number | null;
  noteMax?: number | null;
  ccMap: RoutingMapping[];
  programMap: RoutingMapping[];
};

export type RoutingAssignment = {
  source: string;
  profileId: string;
};

export type MidiActivityInfo = {
  source: string;
  messagesPerSec: number;
  lastNote?: number | null;
  lastChannel?: number | null;
  lastSeenMs?: number | null;
};

export type MidiNoteEvent = {
  source: string;
  note: number;
  channel: number;
  pressed: boolean;
  timestampMs: number;
};

export type AppPaths = {
  configDir: string;
  logFile: string;
  logDir: string;
};

export type VstPluginEntry = {
  name: string;
  path: string;
  format: string;
  kind: string;
  architecture: string;
  supported: boolean;
  unsupportedReason?: string | null;
  midiCompatible?: boolean | null;
  hasEditor: boolean;
  channelLayout?: string | null;
};

export type VstParameter = {
  index: number;
  name: string;
  min: number;
  max: number;
  default: number;
  unit: string;
  value: number;
};

export type LogEvent = {
  level: "info" | "warn" | "error" | "debug";
  message: string;
  timestamp: string;
};

export async function getConfig(): Promise<Config> {
  return invoke("get_config");
}

export async function saveConfig(config: Config): Promise<void> {
  return invoke("save_config", { config });
}

export async function listMidiInputs(): Promise<string[]> {
  return invoke("list_midi_inputs");
}

export async function listMidiOutputs(): Promise<string[]> {
  return invoke("list_midi_outputs");
}

export async function startBridge(config: Config): Promise<BridgeStatus> {
  return invoke("start_bridge", { config });
}

export async function stopBridge(): Promise<void> {
  return invoke("stop_bridge");
}

export async function resetKeys(): Promise<void> {
  return invoke("reset_keys");
}

export async function panicMidi(): Promise<void> {
  return invoke("panic_midi");
}

export async function getStatus(): Promise<BridgeStatus> {
  return invoke("get_status");
}

export async function refreshRtpSessions(): Promise<RtpSessionInfo[]> {
  return invoke("refresh_rtp_sessions");
}

export async function resetConfigDefaults(): Promise<Config> {
  return invoke("reset_config_defaults");
}

export async function restartRtp(): Promise<BridgeStatus> {
  return invoke("restart_rtp");
}

// Audio commands
export type AudioSettings = {
  enabled: boolean;
  backend?: string | null;
  device?: string | null;
  sampleRate: number;
  bufferSize: number;
  gainDb: number;
  limiterEnabled: boolean;
  vstPath?: string | null;
};

export async function listAudioBackends(): Promise<string[]> {
  return invoke("list_audio_backends");
}

export async function listAudioDevices(backend?: string | null): Promise<string[]> {
  return invoke("list_audio_devices", { backend });
}

export async function listVstPlugins(): Promise<VstPluginEntry[]> {
  return invoke("list_vst_plugins");
}

export async function refreshVstPlugins(): Promise<VstPluginEntry[]> {
  return invoke("refresh_vst_plugins");
}

export async function listVstParameters(): Promise<VstParameter[]> {
  return invoke("list_vst_parameters");
}

export async function setVstParameter(index: number, value: number): Promise<void> {
  return invoke("set_vst_parameter", { index, value });
}

export async function startAudio(settings: AudioSettings): Promise<void> {
  return invoke("start_audio", { settings });
}

export async function stopAudio(): Promise<void> {
  return invoke("stop_audio");
}

export async function openVstUi(): Promise<void> {
  return invoke("open_vst_ui");
}

export async function closeVstUi(): Promise<void> {
  return invoke("close_vst_ui");
}

export async function setMasterGain(gainDb: number): Promise<void> {
  return invoke("set_master_gain", { gainDb });
}

export async function setAudioLimiter(enabled: boolean): Promise<void> {
  return invoke("set_audio_limiter", { enabled });
}

export async function pingAudio(): Promise<void> {
  return invoke("ping_audio");
}

export async function reloadVst(): Promise<BridgeStatus> {
  return invoke("reload_vst");
}

export async function preflightCheck(): Promise<PreflightReport> {
  return invoke("preflight_check");
}

export async function exportConfig(path: string): Promise<void> {
  return invoke("export_config", { path });
}

export async function exportDiagnostics(path: string): Promise<void> {
  return invoke("export_diagnostics", { path });
}

export async function importConfig(path: string): Promise<Config> {
  return invoke("import_config", { path });
}

export async function getAppPaths(): Promise<AppPaths> {
  return invoke("get_app_paths");
}

export async function openAppDir(target: "config" | "logs"): Promise<void> {
  return invoke("open_app_dir", { target });
}

export async function clearLogFile(): Promise<void> {
  return invoke("clear_log_file");
}

export async function sendTestMidi(payload: {
  kind: "note" | "cc";
  channel: number;
  note: number;
  velocity: number;
  cc: number;
  value: number;
}): Promise<void> {
  return invoke("send_test_midi", payload);
}
