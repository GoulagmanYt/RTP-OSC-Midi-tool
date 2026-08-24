import { invokeCommand } from "./api-client";
import type {
  AppConfig,
  AudioDeviceEntry,
  AppPaths,
  PreflightReport,
  RtpSessionInfo,
  RuntimeStatus,
  StressTestMode,
  StressTestResult,
  VstParameter,
  VstPluginEntry,
} from "./api-types";

export { CommandFailure } from "./api-errors";
export type * from "./api-types";

export async function getConfig(): Promise<AppConfig> {
  return invokeCommand("get_config");
}

export async function saveConfig(config: AppConfig): Promise<void> {
  return invokeCommand("save_config", { config });
}

export async function listMidiInputs(): Promise<string[]> {
  return invokeCommand("list_midi_inputs");
}

export async function listMidiOutputs(): Promise<string[]> {
  return invokeCommand("list_midi_outputs");
}

export async function startBridge(config: AppConfig): Promise<RuntimeStatus> {
  return invokeCommand("start_bridge", { config });
}

export async function stopBridge(): Promise<void> {
  return invokeCommand("stop_bridge");
}

export async function panicMidi(): Promise<void> {
  return invokeCommand("panic_midi");
}

export async function getStatus(): Promise<RuntimeStatus> {
  return invokeCommand("get_status");
}

export async function refreshRtpSessions(): Promise<RtpSessionInfo[]> {
  return invokeCommand("refresh_rtp_sessions");
}

export async function resetConfigDefaults(): Promise<AppConfig> {
  return invokeCommand("reset_config_defaults");
}

export async function restartRtp(): Promise<RuntimeStatus> {
  return invokeCommand("restart_rtp");
}

export async function listAudioBackends(): Promise<string[]> {
  return invokeCommand("list_audio_backends");
}

export async function listAudioDevices(backend?: string | null): Promise<AudioDeviceEntry[]> {
  return invokeCommand("list_audio_devices", { backend });
}

export async function listVstPlugins(): Promise<VstPluginEntry[]> {
  return invokeCommand("list_vst_plugins");
}

export async function refreshVstPlugins(): Promise<VstPluginEntry[]> {
  return invokeCommand("refresh_vst_plugins");
}

export async function retestVstPlugin(id: string): Promise<VstPluginEntry> {
  return invokeCommand("retest_vst_plugin", { id });
}

export async function openVstFolder(id: string): Promise<void> {
  return invokeCommand("open_vst_folder", { id });
}

export async function listVstParameters(): Promise<VstParameter[]> {
  return invokeCommand("list_vst_parameters");
}

export async function setVstParameter(index: number, value: number): Promise<void> {
  return invokeCommand("set_vst_parameter", { index, value });
}

export async function openVstUi(): Promise<void> {
  return invokeCommand("open_vst_ui");
}

export async function closeVstUi(): Promise<void> {
  return invokeCommand("close_vst_ui");
}

export async function setMasterGain(gainDb: number): Promise<void> {
  return invokeCommand("set_master_gain", { gainDb });
}

export async function setAudioLimiter(enabled: boolean): Promise<void> {
  return invokeCommand("set_audio_limiter", { enabled });
}

export async function pingAudio(): Promise<void> {
  return invokeCommand("ping_audio");
}

export async function reloadVst(): Promise<RuntimeStatus> {
  return invokeCommand("reload_vst");
}

export async function preflightCheck(): Promise<PreflightReport> {
  return invokeCommand("preflight_check");
}

export async function exportConfig(path: string): Promise<void> {
  return invokeCommand("export_config", { path });
}

export async function exportDiagnostics(path: string): Promise<void> {
  return invokeCommand("export_diagnostics", { path });
}

export async function importConfig(path: string): Promise<AppConfig> {
  return invokeCommand("import_config", { path });
}

export async function getAppPaths(): Promise<AppPaths> {
  return invokeCommand("get_app_paths");
}

export async function openAppDir(target: "config" | "logs"): Promise<void> {
  return invokeCommand("open_app_dir", { target });
}

export async function clearLogFile(): Promise<void> {
  return invokeCommand("clear_log_file");
}

export async function sendTestMidi(payload: {
  kind: "note" | "cc";
  channel: number;
  note: number;
  velocity: number;
  cc: number;
  value: number;
}): Promise<void> {
  return invokeCommand("send_test_midi", payload);
}

export async function runAutomatedStressTest(
  rate: number,
  duration: number,
  mode: StressTestMode = "audio-vst"
): Promise<StressTestResult> {
  return invokeCommand("run_automated_stress_test", { rate, duration, mode });
}
