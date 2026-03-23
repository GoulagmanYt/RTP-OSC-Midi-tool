// Prevents additional console window on Windows in release, DO NOT REMOVE!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod bridge;
mod config;
mod logger;
mod midi;
mod osc;
mod plugin_probe;
mod rtp;
mod types;
mod vst_scan;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

use audio::{AudioEngine, AudioSettings};
use bridge::BridgeHandle;
use chrono::Utc;
use config::{
    Config, ConfigStore, RTP_VIRTUAL_INPUT, VST_INTERNAL_OUTPUT, VST_INTERNAL_OUTPUT_LEGACY,
};
use directories::ProjectDirs;
use logger::{set_log_all_to_file, set_logs_enabled, FrontendLogger};
use midir::{MidiInput, MidiOutput};
use parking_lot::Mutex;
use rtp::RtpDiscoveryManager;
use serde::Serialize;
#[cfg(target_os = "windows")]
use tauri::WindowEvent;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, Window};
use types::{
    BridgeStatus, PreflightReport, RtpParticipantInfo, RtpSessionInfo, VstParameter, VstPluginEntry,
};
use vst_scan::{default_vst_scan_roots, scan_vst_plugins_in_roots};
#[cfg(target_os = "windows")]
use window_vibrancy::{apply_acrylic, clear_blur};
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWM_WINDOW_CORNER_PREFERENCE,
    },
};

struct AppState {
    config_store: ConfigStore,
    bridge: BridgeHandle,
    dev_logging: Arc<AtomicBool>,
    audio: AudioEngine,
    vst_cache: Mutex<Option<Vec<VstPluginEntry>>>,
    rtp_discovery: RtpDiscoveryManager,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppPaths {
    config_dir: String,
    log_file: String,
    log_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticExport {
    timestamp: String,
    config: Config,
    status: BridgeStatus,
    rtp_participants: Vec<RtpParticipantInfo>,
    rtp_sessions: Vec<RtpSessionInfo>,
    logs: String,
}

fn config_dir_path() -> Result<PathBuf, String> {
    ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .ok_or_else(|| "Impossible de determiner le dossier de configuration".to_string())
}

fn log_file_path() -> Result<PathBuf, String> {
    Ok(config_dir_path()?.join("app.log"))
}

fn read_log_tail(max_bytes: usize) -> String {
    let Ok(path) = log_file_path() else {
        return String::new();
    };
    let Ok(data) = fs::read(&path) else {
        return String::new();
    };
    let slice = if data.len() > max_bytes {
        &data[data.len() - max_bytes..]
    } else {
        &data
    };
    String::from_utf8_lossy(slice).to_string()
}

fn vst_cache_path() -> Result<PathBuf, String> {
    Ok(config_dir_path()?.join("vst_cache.json"))
}

fn load_vst_cache_from_disk() -> Option<Vec<VstPluginEntry>> {
    let path = vst_cache_path().ok()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_vst_cache_to_disk(entries: &[VstPluginEntry]) {
    let Ok(path) = vst_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(raw) = serde_json::to_string(entries) else {
        return;
    };
    let _ = fs::write(path, raw);
}

fn open_folder_in_explorer(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err("Open folder not supported on this platform".to_string())
    }
}

fn with_audio_status(mut status: BridgeStatus, audio: &AudioEngine) -> BridgeStatus {
    status.audio_running = audio.is_running();
    status.vst_loaded = audio.is_vst_loaded();
    status.audio_latency_ms = audio.current_latency_ms();
    status.audio_backend = audio.current_backend();
    status.audio_device = audio.current_device();
    status.audio_sample_rate = audio.current_sample_rate();
    status.audio_buffer_size = audio.current_buffer_size();
    status.audio_requested_buffer_size = audio.requested_buffer_size();
    status.audio_stream_buffer_size = audio.stream_buffer_size();
    status.audio_buffer_mismatch = audio.buffer_size_mismatch();
    status.vst_midi_compatible = audio.vst_midi_compatible();
    status.audio_xruns = audio.xrun_count();
    status.audio_limiter_enabled = audio.limiter_enabled();
    status
}

fn sync_rtp_discovery(config: &Config, state: &AppState, app: &AppHandle) {
    if config.rtp_remote_enabled {
        state.rtp_discovery.start(app.clone());
    } else {
        state.rtp_discovery.stop();
    }
}

fn sync_runtime_logging(config: &Config, dev_logging: &Arc<AtomicBool>) {
    dev_logging.store(config.verbose, std::sync::atomic::Ordering::Relaxed);
    logger::set_global_dev_mode(config.verbose);
}

#[tauri::command]
fn get_config(app: AppHandle, state: State<AppState>) -> Config {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.log_all_to_file);
    set_logs_enabled(cfg.logs_enabled);
    sync_rtp_discovery(&cfg, &state, &app);
    cfg
}

#[tauri::command]
fn save_config(window: Window, config: Config, state: State<AppState>) -> Result<(), String> {
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.log_all_to_file);
    set_logs_enabled(config.logs_enabled);
    state.config_store.save(&config)?;
    sync_rtp_discovery(&config, &state, &window.app_handle());
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.sync_rtp(&config, &logger, false)?;
    state.bridge.update_config(config, &logger)
}

#[tauri::command]
fn list_midi_inputs() -> Result<Vec<String>, String> {
    let input = MidiInput::new("OSCMidi").map_err(|e| e.to_string())?;
    let mut list: Vec<String> = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();
    list.insert(0, RTP_VIRTUAL_INPUT.to_string());
    Ok(list)
}

#[tauri::command]
fn list_midi_outputs() -> Result<Vec<String>, String> {
    let output = MidiOutput::new("OSCMidi").map_err(|e| e.to_string())?;
    let mut list = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<String>>();
    list.push(VST_INTERNAL_OUTPUT.to_string());
    Ok(list)
}

fn audio_settings_from_config(config: &Config) -> AudioSettings {
    AudioSettings {
        enabled: config.audio_enabled,
        backend: config.audio_backend.clone(),
        device: config.audio_device.clone(),
        sample_rate: config.audio_sample_rate,
        buffer_size: config.audio_buffer_size,
        gain_db: config.audio_gain_db,
        limiter_enabled: config.audio_limiter_enabled,
        vst_path: config.vst_path.clone(),
    }
}

fn fallback_vst_path(app: &AppHandle) -> Option<PathBuf> {
    let resolver = app.path();
    resolver
        .resolve("Keyzone Classic.dll", BaseDirectory::Resource)
        .ok()
        .or_else(|| {
            resolver
                .resolve("Bitsonic/Keyzone Classic.dll", BaseDirectory::Resource)
                .ok()
        })
}

#[tauri::command]
fn start_bridge(
    app: AppHandle,
    window: Window,
    config: Config,
    state: State<AppState>,
) -> Result<BridgeStatus, String> {
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.log_all_to_file);
    set_logs_enabled(config.logs_enabled);
    state.config_store.save(&config)?;
    sync_rtp_discovery(&config, &state, &app);
    let status = state.bridge.start(
        window.clone(),
        config.clone(),
        state.dev_logging.clone(),
        state.audio.clone(),
    )?;
    if config.audio_enabled {
        let logger = FrontendLogger::new(window, state.dev_logging.clone());
        let fallback_vst = fallback_vst_path(&app);
        if let Err(err) = state.audio.start(
            audio_settings_from_config(&config),
            fallback_vst,
            logger.clone(),
        ) {
            logger.error(format!("Audio not started: {err}"));
        }
    }
    Ok(with_audio_status(status, &state.audio))
}

#[tauri::command]
fn stop_bridge(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    state.audio.stop(Some(app));
    state.bridge.stop()
}

#[tauri::command]
fn reset_keys(state: State<AppState>) -> Result<(), String> {
    state.bridge.reset_keys()
}

#[tauri::command]
fn panic_midi(state: State<AppState>) -> Result<(), String> {
    state.audio.panic_all_notes().map_err(|e| e.to_string())?;
    state.bridge.reset_keys()
}

#[tauri::command]
fn send_test_midi(
    kind: String,
    channel: u8,
    note: u8,
    velocity: u8,
    cc: u8,
    value: u8,
    state: State<AppState>,
) -> Result<(), String> {
    match kind.as_str() {
        "note" => state.bridge.send_test_note(note, velocity, channel),
        "cc" => state.bridge.send_test_cc(cc, value, channel),
        _ => Err("Unknown test MIDI kind (note|cc)".to_string()),
    }
}

#[tauri::command]
fn get_status(window: Window, state: State<AppState>) -> BridgeStatus {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    if let Err(err) = state.bridge.sync_rtp(&cfg, &logger, false) {
        logger.error(format!("RTP not started: {err}"));
    }
    let status = state.bridge.status(&cfg);
    with_audio_status(status, &state.audio)
}

#[tauri::command]
fn restart_rtp(window: Window, state: State<AppState>) -> Result<BridgeStatus, String> {
    let cfg = state.config_store.load();
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.restart_rtp(&cfg, &logger)?;
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}

#[tauri::command]
fn list_audio_backends(state: State<AppState>) -> Vec<String> {
    state.audio.list_backends()
}

#[tauri::command]
fn list_audio_devices(backend: Option<String>, state: State<AppState>) -> Vec<String> {
    state.audio.list_devices(backend)
}

#[tauri::command]
fn list_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    state.vst_cache.lock().clone().unwrap_or_default()
}

#[tauri::command]
fn refresh_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    let roots = default_vst_scan_roots();
    if roots.is_empty() {
        return state.vst_cache.lock().clone().unwrap_or_default();
    }

    let plugins = scan_vst_plugins_in_roots(&roots);
    *state.vst_cache.lock() = Some(plugins.clone());
    save_vst_cache_to_disk(&plugins);
    plugins
}

#[tauri::command]
fn list_vst_parameters(state: State<AppState>) -> Result<Vec<VstParameter>, String> {
    state.audio.list_vst_parameters().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_vst_parameter(index: usize, value: f32, state: State<AppState>) -> Result<(), String> {
    state
        .audio
        .set_vst_parameter(index, value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn start_audio(
    app: AppHandle,
    window: Window,
    settings: AudioSettings,
    state: State<AppState>,
) -> Result<(), String> {
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    let fallback_vst = fallback_vst_path(&app);
    state
        .audio
        .start(settings, fallback_vst, logger)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_audio(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    state.audio.stop(Some(app));
    Ok(())
}

#[tauri::command]
fn open_vst_ui(app: AppHandle, window: Window, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    if !cfg.audio_enabled {
        return Err("Audio engine is disabled in settings".into());
    }

    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    if !state.audio.is_running() {
        state
            .audio
            .start(
                audio_settings_from_config(&cfg),
                fallback_vst_path(&app),
                logger.clone(),
            )
            .map_err(|e| e.to_string())?;
    }

    state.audio.open_vst_ui(app).map_err(|e| e.to_string())
}

#[tauri::command]
fn close_vst_ui(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    state.audio.close_vst_ui(app).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_master_gain(gain_db: f32, state: State<AppState>) -> Result<(), String> {
    state.audio.set_gain(gain_db);
    Ok(())
}

#[tauri::command]
fn set_audio_limiter(enabled: bool, state: State<AppState>) -> Result<(), String> {
    state.audio.set_limiter_enabled(enabled);
    Ok(())
}

#[tauri::command]
fn ping_audio(app: AppHandle, window: Window, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    if !cfg.audio_enabled {
        return Err("Audio engine is disabled in settings".into());
    }
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    if !state.audio.is_running() {
        let fallback_vst = fallback_vst_path(&app);
        state
            .audio
            .start(
                audio_settings_from_config(&cfg),
                fallback_vst,
                logger.clone(),
            )
            .map_err(|e| e.to_string())?;
    }
    state.audio.ping().map_err(|e| e.to_string())
}

#[tauri::command]
fn reload_vst(
    app: AppHandle,
    window: Window,
    state: State<AppState>,
) -> Result<BridgeStatus, String> {
    let cfg = state.config_store.load();
    if !cfg.audio_enabled {
        return Err("Audio engine is disabled in settings".into());
    }
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    let fallback_vst = fallback_vst_path(&app);
    state
        .audio
        .reload(audio_settings_from_config(&cfg), fallback_vst, logger)
        .map_err(|e| e.to_string())?;
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}

#[tauri::command]
fn preflight_check(state: State<AppState>) -> Result<PreflightReport, String> {
    let cfg = state.config_store.load();
    let mut messages = Vec::new();

    let midi_inputs = list_midi_inputs()?;
    let midi_in_ok = cfg
        .midi_in
        .as_ref()
        .map(|name| {
            if name == RTP_VIRTUAL_INPUT {
                true
            } else {
                midi_inputs.iter().any(|p| p.contains(name))
            }
        })
        .unwrap_or(false);
    if !midi_in_ok {
        messages.push("MIDI IN manquant ou non configure".to_string());
    }

    let midi_outputs = list_midi_outputs()?;
    let midi_out_ok = cfg
        .midi_out
        .as_ref()
        .map(|name| {
            if name == VST_INTERNAL_OUTPUT || name == VST_INTERNAL_OUTPUT_LEGACY {
                true
            } else {
                midi_outputs.iter().any(|p| p.contains(name))
            }
        })
        .unwrap_or(false);
    if !midi_out_ok {
        messages.push("MIDI OUT manquant ou non configure".to_string());
    }

    let backends = state.audio.list_backends();
    let audio_backend_ok = if !cfg.audio_enabled {
        true
    } else if let Some(backend) = cfg.audio_backend.as_ref() {
        if backend.to_lowercase() == "auto" {
            !backends.is_empty()
        } else {
            backends.iter().any(|b| b.eq_ignore_ascii_case(backend))
        }
    } else {
        !backends.is_empty()
    };
    if !audio_backend_ok {
        messages.push(format!(
            "Backend audio indisponible ({})",
            cfg.audio_backend.clone().unwrap_or_default()
        ));
    }

    let rtp_needed = cfg.rtp_enabled
        || cfg.rtp_remote_enabled
        || cfg
            .midi_in
            .as_ref()
            .map(|s| s == RTP_VIRTUAL_INPUT)
            .unwrap_or(false);
    let rtp_port_ok = if rtp_needed {
        if state.bridge.owns_rtp_port(cfg.rtp_port) {
            true
        } else {
            rtp::ports_available(cfg.rtp_port)?
        }
    } else {
        true
    };
    if !rtp_port_ok {
        messages.push(format!("Port RTP {} deja utilise", cfg.rtp_port));
    }

    Ok(PreflightReport {
        midi_in_ok,
        midi_out_ok,
        audio_backend_ok,
        rtp_port_ok,
        messages,
    })
}

#[tauri::command]
fn export_config(path: String, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    let yaml = serde_yaml::to_string(&cfg).map_err(|e| e.to_string())?;
    if let Some(parent) = PathBuf::from(&path).parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, yaml).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_diagnostics(path: String, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    let status = with_audio_status(state.bridge.status(&cfg), &state.audio);
    let payload = DiagnosticExport {
        timestamp: Utc::now().to_rfc3339(),
        config: cfg,
        status,
        rtp_participants: state.bridge.rtp_participants(),
        rtp_sessions: state.rtp_discovery.cached(),
        logs: read_log_tail(200_000),
    };
    let json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    if let Some(parent) = PathBuf::from(&path).parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, json).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_config(
    app: AppHandle,
    window: Window,
    path: String,
    state: State<AppState>,
) -> Result<Config, String> {
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let cfg: Config = serde_yaml::from_str(&raw).map_err(|e| e.to_string())?;

    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.log_all_to_file);
    set_logs_enabled(cfg.logs_enabled);
    state.config_store.save(&cfg)?;
    sync_rtp_discovery(&cfg, &state, &app);
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.sync_rtp(&cfg, &logger, true)?;
    state.bridge.update_config(cfg.clone(), &logger)?;

    if cfg.audio_enabled {
        let fallback_vst = fallback_vst_path(&app);
        if let Err(err) = state.audio.start(
            audio_settings_from_config(&cfg),
            fallback_vst,
            logger.clone(),
        ) {
            logger.error(format!("Audio not restarted after import: {err}"));
        }
    } else {
        state.audio.stop(Some(app));
    }

    Ok(cfg)
}

#[tauri::command]
fn get_app_paths() -> Result<AppPaths, String> {
    let config_dir = config_dir_path()?;
    let log_file = log_file_path()?;
    let log_dir = log_file
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Impossible de determiner le dossier des logs".to_string())?;

    Ok(AppPaths {
        config_dir: config_dir.to_string_lossy().to_string(),
        log_file: log_file.to_string_lossy().to_string(),
        log_dir: log_dir.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn open_app_dir(target: String) -> Result<(), String> {
    let config_dir = config_dir_path()?;
    let log_dir = log_file_path()?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Impossible de determiner le dossier des logs".to_string())?;

    match target.as_str() {
        "config" => open_folder_in_explorer(&config_dir),
        "logs" => open_folder_in_explorer(&log_dir),
        _ => Err("Dossier inconnu (config|logs)".into()),
    }
}

#[tauri::command]
fn clear_log_file() -> Result<(), String> {
    logger::clear_log_file()
}

#[tauri::command]
fn refresh_rtp_sessions(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<RtpSessionInfo>, String> {
    state.rtp_discovery.refresh_now(&app, true)
}

#[tauri::command]
fn reset_config_defaults(window: Window, state: State<AppState>) -> Result<Config, String> {
    let cfg = state.config_store.reset_to_default()?;
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.log_all_to_file);
    set_logs_enabled(cfg.logs_enabled);
    let _ = state.config_store.save(&cfg);
    sync_rtp_discovery(&cfg, &state, &window.app_handle());
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.sync_rtp(&cfg, &logger, true)?;
    state.bridge.update_config(cfg.clone(), &logger)?;
    Ok(cfg)
}

#[cfg(target_os = "windows")]
fn set_round_corners(window: &Window) {
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let pref = DWM_WINDOW_CORNER_PREFERENCE(2);
            let _ = DwmSetWindowAttribute(
                HWND(hwnd.0),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &pref as *const _ as *const _,
                std::mem::size_of_val(&pref) as u32,
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn set_round_corners_webview(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let pref = DWM_WINDOW_CORNER_PREFERENCE(2);
            let _ = DwmSetWindowAttribute(
                HWND(hwnd.0),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &pref as *const _ as *const _,
                std::mem::size_of_val(&pref) as u32,
            );
        }
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = match info.payload().downcast_ref::<&str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => &s[..],
                None => "Box<Any>",
            },
        };
        let location = info
            .location()
            .map(|l| format!("file '{}' at line {}", l.file(), l.line()))
            .unwrap_or("unknown location".into());
        let err_msg = format!("PANIC: '{}' at {}", msg, location);
        eprintln!("{}", err_msg);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("panic.log")
        {
            use std::io::Write;
            let _ = writeln!(
                file,
                "[{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                err_msg
            );
        }
    }));

    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_os::init())
        .manage(AppState {
            config_store: ConfigStore::new(),
            bridge: BridgeHandle::new(),
            dev_logging: Arc::new(AtomicBool::new(false)),
            audio: AudioEngine::new(),
            vst_cache: Mutex::new(load_vst_cache_from_disk()),
            rtp_discovery: RtpDiscoveryManager::new(),
        })
        .on_window_event(|window, event| {
            #[cfg(target_os = "windows")]
            {
                if let WindowEvent::Resized(_) = event {
                    set_round_corners(window);
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = (window, event);
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            list_midi_inputs,
            list_midi_outputs,
            start_bridge,
            stop_bridge,
            reset_keys,
            panic_midi,
            get_status,
            restart_rtp,
            preflight_check,
            refresh_rtp_sessions,
            ping_audio,
            reload_vst,
            reset_config_defaults,
            export_config,
            export_diagnostics,
            import_config,
            get_app_paths,
            open_app_dir,
            clear_log_file,
            list_audio_backends,
            list_audio_devices,
            list_vst_plugins,
            refresh_vst_plugins,
            list_vst_parameters,
            set_vst_parameter,
            start_audio,
            stop_audio,
            open_vst_ui,
            close_vst_ui,
            set_master_gain,
            set_audio_limiter,
            send_test_midi
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            #[cfg(target_os = "windows")]
            {
                window.set_decorations(false)?;
                window.set_always_on_top(false)?;
                window.set_resizable(true)?;
                window.set_title("")?;
                let _ = clear_blur(&window);
                let _ = apply_acrylic(&window, Some((0, 0, 0, 0)));
                set_round_corners_webview(&window);
            }
            window
                .emit("log", types::LogEvent::new("info", "Interface ready"))
                .ok();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
