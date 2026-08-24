//! Commandes Tauri: fine couche d'adaptation vers la couche application.

use tauri::{AppHandle, Emitter, State, Window};

use crate::{
    application::services,
    config::Config,
    error::CommandError,
    tauri::state::AppState,
    types::{
        AppPaths, AudioDeviceEntry, BridgeStatus, PreflightReport, RtpSessionInfo, VstParameter,
        VstPluginEntry,
    },
};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

#[::tauri::command]
pub fn get_config(app: AppHandle, state: State<AppState>) -> Config {
    services::get_config(&app, state.inner())
}

#[::tauri::command]
pub fn save_config(
    window: Window,
    config: Config,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    services::save_config(&window, config, state.inner())
}

#[::tauri::command]
pub fn list_midi_inputs() -> Result<Vec<String>, CommandError> {
    services::list_midi_inputs()
}

#[::tauri::command]
pub fn list_midi_outputs() -> Result<Vec<String>, CommandError> {
    services::list_midi_outputs()
}

#[::tauri::command]
pub async fn start_bridge(
    app: AppHandle,
    window: Window,
    config: Config,
    state: State<'_, AppState>,
) -> Result<BridgeStatus, CommandError> {
    services::start_bridge(&app, &window, config, state.inner())
}

#[::tauri::command]
pub async fn stop_bridge(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    services::stop_bridge(&app, state.inner()).await
}

#[::tauri::command]
pub fn panic_midi(state: State<AppState>) -> Result<(), CommandError> {
    services::panic_midi(state.inner())
}

#[::tauri::command]
pub fn send_test_midi(
    kind: String,
    channel: u8,
    note: u8,
    velocity: u8,
    cc: u8,
    value: u8,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    services::send_test_midi(kind, channel, note, velocity, cc, value, state.inner())
}

#[::tauri::command]
pub fn get_status(window: Window, state: State<AppState>) -> BridgeStatus {
    services::get_status(&window, state.inner())
}

#[::tauri::command]
pub fn restart_rtp(window: Window, state: State<AppState>) -> Result<BridgeStatus, CommandError> {
    services::restart_rtp(&window, state.inner())
}

#[::tauri::command]
pub fn refresh_rtp_sessions(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<RtpSessionInfo>, CommandError> {
    services::refresh_rtp_sessions(&app, state.inner())
}

#[::tauri::command]
pub fn reset_config_defaults(
    window: Window,
    state: State<AppState>,
) -> Result<Config, CommandError> {
    services::reset_config_defaults(&window, state.inner())
}

#[::tauri::command]
pub fn list_audio_backends(state: State<AppState>) -> Vec<String> {
    services::list_audio_backends(state.inner())
}

#[::tauri::command]
pub fn list_audio_devices(
    backend: Option<String>,
    state: State<AppState>,
) -> Vec<AudioDeviceEntry> {
    services::list_audio_devices(backend, state.inner())
}

#[::tauri::command]
pub fn list_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    services::list_vst_plugins(state.inner())
}

#[::tauri::command]
pub async fn refresh_vst_plugins(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Vec<VstPluginEntry>, CommandError> {
    let _ = window.emit(
        "vst:scan-progress",
        serde_json::json!({ "state": "started" }),
    );
    let plugins = services::refresh_vst_plugins(state.inner());
    let _ = window.emit(
        "vst:scan-progress",
        serde_json::json!({ "state": "finished", "count": plugins.len() }),
    );
    Ok(plugins)
}

#[::tauri::command]
pub fn retest_vst_plugin(
    id: String,
    state: State<AppState>,
) -> Result<VstPluginEntry, CommandError> {
    services::retest_vst_plugin(&id, state.inner())
}

#[::tauri::command]
pub fn open_vst_folder(id: String, state: State<AppState>) -> Result<(), CommandError> {
    services::open_vst_folder(&id, state.inner())
}

#[::tauri::command]
pub fn list_vst_parameters(state: State<AppState>) -> Result<Vec<VstParameter>, CommandError> {
    services::list_vst_parameters(state.inner())
}

#[::tauri::command]
pub fn set_vst_parameter(
    index: usize,
    value: f32,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    services::set_vst_parameter(index, value, state.inner())
}

#[::tauri::command]
pub async fn open_vst_ui(
    app: AppHandle,
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    services::open_vst_ui(&app, &window, state.inner())
}

#[::tauri::command]
pub async fn close_vst_ui(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    services::close_vst_ui(&app, state.inner())
}

#[::tauri::command]
pub fn set_master_gain(gain_db: f32, state: State<AppState>) -> Result<(), CommandError> {
    services::set_master_gain(gain_db, state.inner())
}

#[::tauri::command]
pub fn set_audio_limiter(enabled: bool, state: State<AppState>) -> Result<(), CommandError> {
    services::set_audio_limiter(enabled, state.inner())
}

#[::tauri::command]
pub async fn ping_audio(
    app: AppHandle,
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    services::ping_audio(&app, &window, state.inner())
}

#[::tauri::command]
pub async fn reload_vst(
    app: AppHandle,
    window: Window,
    state: State<'_, AppState>,
) -> Result<BridgeStatus, CommandError> {
    services::reload_vst(&app, &window, state.inner())
}

#[::tauri::command]
pub fn preflight_check(state: State<AppState>) -> Result<PreflightReport, CommandError> {
    services::preflight_check(state.inner())
}

#[::tauri::command]
pub fn export_config(path: String, state: State<AppState>) -> Result<(), CommandError> {
    services::export_config(path, state.inner())
}

#[::tauri::command]
pub fn export_diagnostics(path: String, state: State<AppState>) -> Result<(), CommandError> {
    services::export_diagnostics(path, state.inner())
}

#[::tauri::command]
pub async fn import_config(
    app: AppHandle,
    window: Window,
    path: String,
    state: State<'_, AppState>,
) -> Result<Config, CommandError> {
    services::import_config(&app, &window, path, state.inner())
}

#[::tauri::command]
pub fn get_app_paths() -> Result<AppPaths, CommandError> {
    services::get_app_paths()
}

#[::tauri::command]
pub fn open_app_dir(target: String) -> Result<(), CommandError> {
    services::open_app_dir(target)
}

#[::tauri::command]
pub fn clear_log_file() -> Result<(), CommandError> {
    services::clear_log_file()
}

#[::tauri::command]
pub fn run_automated_stress_test(
    rate: u32,
    duration: u32,
    mode: Option<String>,
    state: State<AppState>,
) -> Result<crate::types::StressTestResult, CommandError> {
    let mode = mode.as_deref().unwrap_or("audio-vst");
    services::run_stress_test(mode, rate, duration, state.inner())
}
