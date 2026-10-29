//! Commandes Tauri: fine couche d'adaptation vers la couche application.

use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::{
    application::services,
    config::Config,
    error::CommandError,
    tauri::state::AppState,
    types::{
        AppPaths, AudioDeviceEntry, BridgeStatus, PreflightReport, RtpParticipantInfo,
        RtpSessionInfo, VstParameter, VstPluginEntry,
    },
};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

async fn blocking_command<T: Send + 'static>(
    code: &'static str,
    domain: &'static str,
    task: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    ::tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| CommandError::new(code, domain, error.to_string()))?
}

#[::tauri::command]
pub fn get_config(app: AppHandle, state: State<AppState>) -> Result<Config, CommandError> {
    services::get_config(&app, state.inner())
}

#[::tauri::command]
pub async fn save_config(
    app: AppHandle,
    window: Window,
    config: Config,
) -> Result<(), CommandError> {
    ::tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        services::save_config(&window, config, state.inner())
    })
    .await
    .map_err(|error| CommandError::new("config.save-task-failed", "config", error.to_string()))?
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
) -> Result<BridgeStatus, CommandError> {
    blocking_command("runtime.start-task-failed", "runtime", move || {
        let state = app.state::<AppState>();
        services::start_bridge(&app, &window, config, state.inner())
    })
    .await
}

#[::tauri::command]
pub async fn stop_bridge(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    services::stop_bridge(&app, state.inner()).await
}

#[::tauri::command]
pub async fn panic_midi(app: AppHandle) -> Result<(), CommandError> {
    blocking_command("runtime.panic-task-failed", "runtime", move || {
        let state = app.state::<AppState>();
        services::panic_midi(state.inner())
    })
    .await
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
pub async fn restart_rtp(app: AppHandle, window: Window) -> Result<BridgeStatus, CommandError> {
    blocking_command("runtime.restart-rtp-task-failed", "runtime", move || {
        let state = app.state::<AppState>();
        services::restart_rtp(&window, state.inner())
    })
    .await
}

#[::tauri::command]
pub fn refresh_rtp_sessions(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<RtpSessionInfo>, CommandError> {
    services::refresh_rtp_sessions(&app, state.inner())
}

#[::tauri::command]
pub fn get_rtp_participants(state: State<AppState>) -> Vec<RtpParticipantInfo> {
    services::get_rtp_participants(state.inner())
}

#[::tauri::command]
pub fn reset_config_defaults(
    window: Window,
    state: State<AppState>,
) -> Result<Config, CommandError> {
    services::reset_config_defaults(&window, state.inner())
}

#[::tauri::command]
pub async fn list_audio_backends(app: AppHandle) -> Result<Vec<String>, CommandError> {
    blocking_command("audio.list-backends-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        Ok(services::list_audio_backends(state.inner()))
    })
    .await
}

#[::tauri::command]
pub async fn list_audio_devices(
    app: AppHandle,
    backend: Option<String>,
) -> Result<Vec<AudioDeviceEntry>, CommandError> {
    blocking_command("audio.list-devices-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        Ok(services::list_audio_devices(backend, state.inner()))
    })
    .await
}

#[::tauri::command]
pub fn list_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    services::list_vst_plugins(state.inner())
}

#[::tauri::command]
pub async fn refresh_vst_plugins(
    app: AppHandle,
    window: Window,
) -> Result<Vec<VstPluginEntry>, CommandError> {
    blocking_command(
        "audio.refresh-vst-plugins-task-failed",
        "audio",
        move || {
            let state = app.state::<AppState>();
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
        },
    )
    .await
}

#[::tauri::command]
pub async fn retest_vst_plugin(app: AppHandle, id: String) -> Result<VstPluginEntry, CommandError> {
    blocking_command("audio.retest-vst-plugin-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        services::retest_vst_plugin(&id, state.inner())
    })
    .await
}

#[::tauri::command]
pub fn open_vst_folder(id: String, state: State<AppState>) -> Result<(), CommandError> {
    services::open_vst_folder(&id, state.inner())
}

#[::tauri::command]
pub async fn list_vst_parameters(app: AppHandle) -> Result<Vec<VstParameter>, CommandError> {
    blocking_command(
        "audio.list-vst-parameters-task-failed",
        "audio",
        move || {
            let state = app.state::<AppState>();
            services::list_vst_parameters(state.inner())
        },
    )
    .await
}

#[::tauri::command]
pub async fn set_vst_parameter(
    app: AppHandle,
    index: usize,
    value: f32,
) -> Result<(), CommandError> {
    blocking_command("audio.set-vst-parameter-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        services::set_vst_parameter(index, value, state.inner())
    })
    .await
}

#[::tauri::command]
pub async fn open_vst_ui(app: AppHandle, window: Window) -> Result<(), CommandError> {
    blocking_command("audio.open-vst-ui-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        services::open_vst_ui(&app, &window, state.inner())
    })
    .await
}

#[::tauri::command]
pub async fn close_vst_ui(app: AppHandle) -> Result<(), CommandError> {
    blocking_command("audio.close-vst-ui-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        services::close_vst_ui(&app, state.inner())
    })
    .await
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
pub fn set_vst_eq(
    settings: crate::config::VstEqSettings,
    state: State<AppState>,
) -> Result<(), CommandError> {
    services::set_vst_eq(settings, state.inner())
}

#[::tauri::command]
pub async fn ping_audio(app: AppHandle, window: Window) -> Result<(), CommandError> {
    blocking_command("audio.ping-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        services::ping_audio(&app, &window, state.inner())
    })
    .await
}

#[::tauri::command]
pub async fn reload_vst(app: AppHandle, window: Window) -> Result<BridgeStatus, CommandError> {
    blocking_command("audio.reload-vst-task-failed", "audio", move || {
        let state = app.state::<AppState>();
        services::reload_vst(&app, &window, state.inner())
    })
    .await
}

#[::tauri::command]
pub async fn preflight_check(
    app: AppHandle,
    config: Option<Config>,
) -> Result<PreflightReport, CommandError> {
    blocking_command("runtime.preflight-task-failed", "runtime", move || {
        let state = app.state::<AppState>();
        services::preflight_check(config, state.inner())
    })
    .await
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
) -> Result<Config, CommandError> {
    blocking_command("config.import-task-failed", "config", move || {
        let state = app.state::<AppState>();
        services::import_config(&app, &window, path, state.inner())
    })
    .await
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
pub async fn run_automated_stress_test(
    app: AppHandle,
    rate: u32,
    duration: u32,
    mode: Option<String>,
) -> Result<crate::types::StressTestResult, CommandError> {
    blocking_command("runtime.stress-test-task-failed", "runtime", move || {
        let state = app.state::<AppState>();
        let mode = mode.as_deref().unwrap_or("audio-vst");
        services::run_stress_test(mode, rate, duration, state.inner())
    })
    .await
}
