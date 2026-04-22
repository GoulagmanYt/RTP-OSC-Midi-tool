//! Commandes Tauri: fine couche d'adaptation vers la couche application.

use tauri::{AppHandle, State, Window};

use crate::{
    application::services,
    audio::AudioSettings,
    config::Config,
    error::CommandError,
    tauri::state::AppState,
    types::{
        AppPaths, BridgeStatus, PreflightReport, RtpSessionInfo, VstParameter, VstPluginEntry,
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
    state: State<AppState>,
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
pub fn start_bridge(
    app: AppHandle,
    window: Window,
    config: Config,
    state: State<AppState>,
) -> Result<BridgeStatus, CommandError> {
    services::start_bridge(&app, &window, config, state.inner())
}

#[::tauri::command]
pub fn stop_bridge(app: AppHandle, state: State<AppState>) -> Result<(), CommandError> {
    services::stop_bridge(&app, state.inner())
}

#[::tauri::command]
pub fn reset_keys(state: State<AppState>) -> Result<(), CommandError> {
    services::reset_keys(state.inner())
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
    state: State<AppState>,
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
pub fn list_audio_devices(backend: Option<String>, state: State<AppState>) -> Vec<String> {
    services::list_audio_devices(backend, state.inner())
}

#[::tauri::command]
pub fn list_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    services::list_vst_plugins(state.inner())
}

#[::tauri::command]
pub fn refresh_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    services::refresh_vst_plugins(state.inner())
}

#[::tauri::command]
pub fn list_vst_parameters(state: State<AppState>) -> Result<Vec<VstParameter>, CommandError> {
    services::list_vst_parameters(state.inner())
}

#[::tauri::command]
pub fn set_vst_parameter(
    index: usize,
    value: f32,
    state: State<AppState>,
) -> Result<(), CommandError> {
    services::set_vst_parameter(index, value, state.inner())
}

#[::tauri::command]
pub fn start_audio(
    app: AppHandle,
    window: Window,
    settings: AudioSettings,
    state: State<AppState>,
) -> Result<(), CommandError> {
    services::start_audio(&app, &window, settings, state.inner())
}

#[::tauri::command]
pub fn stop_audio(app: AppHandle, state: State<AppState>) -> Result<(), CommandError> {
    services::stop_audio(&app, state.inner())
}

#[::tauri::command]
pub fn open_vst_ui(
    app: AppHandle,
    window: Window,
    state: State<AppState>,
) -> Result<(), CommandError> {
    services::open_vst_ui(&app, &window, state.inner())
}

#[::tauri::command]
pub fn close_vst_ui(app: AppHandle, state: State<AppState>) -> Result<(), CommandError> {
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
pub fn ping_audio(
    app: AppHandle,
    window: Window,
    state: State<AppState>,
) -> Result<(), CommandError> {
    services::ping_audio(&app, &window, state.inner())
}

#[::tauri::command]
pub fn reload_vst(
    app: AppHandle,
    window: Window,
    state: State<AppState>,
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
pub fn import_config(
    app: AppHandle,
    window: Window,
    path: String,
    state: State<AppState>,
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
