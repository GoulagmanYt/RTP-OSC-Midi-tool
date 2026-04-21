//! Commandes Tauri - contrat public backend/frontend legacy.

use std::{fs, path::PathBuf};

use chrono::Utc;
use midir::{MidiInput, MidiOutput};
use serde::Serialize;
use tauri::{AppHandle, Manager, State, Window};

use crate::{
    audio::AudioSettings,
    config::{Config, RTP_VIRTUAL_INPUT, VST_INTERNAL_OUTPUT, VST_INTERNAL_OUTPUT_LEGACY},
    logger::{self, FrontendLogger, set_log_all_to_file, set_logs_enabled},
    rtp,
    tauri::{
        state::AppState,
        utils::{
            audio_settings_from_config, config_dir_path, fallback_vst_path, log_file_path,
            open_folder_in_explorer, read_log_tail, retain_instrument_entries,
            save_vst_cache_to_disk, sync_rtp_discovery, sync_runtime_logging,
        },
    },
    types::{
        AppPaths, BridgeStatus, PreflightReport, RtpParticipantInfo, RtpSessionInfo,
        VstParameter, VstPluginEntry,
    },
    vst_scan::{default_vst_scan_roots, scan_vst_plugins_in_roots},
};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

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

fn with_audio_status(mut status: BridgeStatus, audio: &crate::audio::AudioEngine) -> BridgeStatus {
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

#[::tauri::command]
pub fn get_config(app: AppHandle, state: State<AppState>) -> Config {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.log_all_to_file);
    set_logs_enabled(cfg.logs_enabled);
    sync_rtp_discovery(&cfg, &state, &app);
    cfg
}

#[::tauri::command]
pub fn save_config(window: Window, config: Config, state: State<AppState>) -> Result<(), String> {
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.log_all_to_file);
    set_logs_enabled(config.logs_enabled);
    state.config_store.save(&config)?;
    sync_rtp_discovery(&config, &state, window.app_handle());
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.sync_rtp(&config, &logger, false)?;
    state.bridge.update_config(config, &logger)
}

#[::tauri::command]
pub fn list_midi_inputs() -> Result<Vec<String>, String> {
    let input = MidiInput::new("OSCMidi").map_err(|e| e.to_string())?;
    let mut list: Vec<String> = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();
    list.insert(0, RTP_VIRTUAL_INPUT.to_string());
    Ok(list)
}

#[::tauri::command]
pub fn list_midi_outputs() -> Result<Vec<String>, String> {
    let output = MidiOutput::new("OSCMidi").map_err(|e| e.to_string())?;
    let mut list = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<String>>();
    list.push(VST_INTERNAL_OUTPUT.to_string());
    Ok(list)
}

#[::tauri::command]
pub fn start_bridge(
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
        if let Err(err) = state.audio.start(audio_settings_from_config(&config), fallback_vst, logger.clone()) {
            logger.error(format!("Audio not started: {err}"));
        }
    }
    Ok(with_audio_status(status, &state.audio))
}

#[::tauri::command]
pub fn stop_bridge(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    state.audio.stop(Some(app));
    state.bridge.stop()
}

#[::tauri::command]
pub fn reset_keys(state: State<AppState>) -> Result<(), String> {
    state.bridge.reset_keys()
}

#[::tauri::command]
pub fn panic_midi(state: State<AppState>) -> Result<(), String> {
    state.audio.panic_all_notes().map_err(|e| e.to_string())?;
    state.bridge.reset_keys()
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
) -> Result<(), String> {
    match kind.as_str() {
        "note" => state.bridge.send_test_note(note, velocity, channel),
        "cc" => state.bridge.send_test_cc(cc, value, channel),
        _ => Err("Unknown test MIDI kind (note|cc)".to_string()),
    }
}

#[::tauri::command]
pub fn get_status(window: Window, state: State<AppState>) -> BridgeStatus {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    if let Err(err) = state.bridge.sync_rtp(&cfg, &logger, false) {
        logger.error(format!("RTP not started: {err}"));
    }
    let status = state.bridge.status(&cfg);
    with_audio_status(status, &state.audio)
}

#[::tauri::command]
pub fn restart_rtp(window: Window, state: State<AppState>) -> Result<BridgeStatus, String> {
    let cfg = state.config_store.load();
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.restart_rtp(&cfg, &logger)?;
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}

#[::tauri::command]
pub fn refresh_rtp_sessions(
    app: AppHandle,
    state: State<AppState>,
) -> Result<Vec<RtpSessionInfo>, String> {
    state.rtp_discovery.refresh_now(&app, true)
}

#[::tauri::command]
pub fn reset_config_defaults(window: Window, state: State<AppState>) -> Result<Config, String> {
    let cfg = state.config_store.reset_to_default()?;
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.log_all_to_file);
    set_logs_enabled(cfg.logs_enabled);
    let _ = state.config_store.save(&cfg);
    sync_rtp_discovery(&cfg, &state, window.app_handle());
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state.bridge.sync_rtp(&cfg, &logger, true)?;
    state.bridge.update_config(cfg.clone(), &logger)?;
    Ok(cfg)
}

#[::tauri::command]
pub fn list_audio_backends(state: State<AppState>) -> Vec<String> {
    state.audio.list_backends()
}

#[::tauri::command]
pub fn list_audio_devices(backend: Option<String>, state: State<AppState>) -> Vec<String> {
    state.audio.list_devices(backend)
}

#[::tauri::command]
pub fn list_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    state
        .vst_cache
        .lock()
        .clone()
        .map(retain_instrument_entries)
        .unwrap_or_default()
}

#[::tauri::command]
pub fn refresh_vst_plugins(state: State<AppState>) -> Vec<VstPluginEntry> {
    let roots = default_vst_scan_roots();
    if roots.is_empty() {
        return state
            .vst_cache
            .lock()
            .clone()
            .map(retain_instrument_entries)
            .unwrap_or_default();
    }

    let plugins = retain_instrument_entries(scan_vst_plugins_in_roots(&roots));
    *state.vst_cache.lock() = Some(plugins.clone());
    save_vst_cache_to_disk(&plugins);
    plugins
}

#[::tauri::command]
pub fn list_vst_parameters(state: State<AppState>) -> Result<Vec<VstParameter>, String> {
    state.audio.list_vst_parameters().map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn set_vst_parameter(index: usize, value: f32, state: State<AppState>) -> Result<(), String> {
    state.audio.set_vst_parameter(index, value).map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn start_audio(
    app: AppHandle,
    window: Window,
    settings: AudioSettings,
    state: State<AppState>,
) -> Result<(), String> {
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    let fallback_vst = fallback_vst_path(&app);
    state.audio.start(settings, fallback_vst, logger).map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn stop_audio(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    state.audio.stop(Some(app));
    Ok(())
}

#[::tauri::command]
pub fn open_vst_ui(app: AppHandle, window: Window, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    if !cfg.audio_enabled {
        return Err("Audio engine is disabled in settings".into());
    }

    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    if !state.audio.is_running() {
        state
            .audio
            .start(audio_settings_from_config(&cfg), fallback_vst_path(&app), logger.clone())
            .map_err(|e| e.to_string())?;
    }
    state.audio.open_vst_ui(app).map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn close_vst_ui(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    state.audio.close_vst_ui(app).map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn set_master_gain(gain_db: f32, state: State<AppState>) -> Result<(), String> {
    state.audio.set_gain(gain_db);
    Ok(())
}

#[::tauri::command]
pub fn set_audio_limiter(enabled: bool, state: State<AppState>) -> Result<(), String> {
    state.audio.set_limiter_enabled(enabled);
    Ok(())
}

#[::tauri::command]
pub fn ping_audio(app: AppHandle, window: Window, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    if !cfg.audio_enabled {
        return Err("Audio engine is disabled in settings".into());
    }
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    if !state.audio.is_running() {
        state
            .audio
            .start(audio_settings_from_config(&cfg), fallback_vst_path(&app), logger.clone())
            .map_err(|e| e.to_string())?;
    }
    state.audio.ping().map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn reload_vst(
    app: AppHandle,
    window: Window,
    state: State<AppState>,
) -> Result<BridgeStatus, String> {
    let cfg = state.config_store.load();
    if !cfg.audio_enabled {
        return Err("Audio engine is disabled in settings".into());
    }
    let logger = FrontendLogger::new(window, state.dev_logging.clone());
    state
        .audio
        .reload(audio_settings_from_config(&cfg), fallback_vst_path(&app), logger)
        .map_err(|e| e.to_string())?;
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}

#[::tauri::command]
pub fn preflight_check(state: State<AppState>) -> Result<PreflightReport, String> {
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
        || cfg.midi_in.as_ref().map(|s| s == RTP_VIRTUAL_INPUT).unwrap_or(false);
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

#[::tauri::command]
pub fn export_config(path: String, state: State<AppState>) -> Result<(), String> {
    let cfg = state.config_store.load();
    let yaml = serde_yaml::to_string(&cfg).map_err(|e| e.to_string())?;
    if let Some(parent) = PathBuf::from(&path).parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, yaml).map_err(|e| e.to_string())
}

#[::tauri::command]
pub fn export_diagnostics(path: String, state: State<AppState>) -> Result<(), String> {
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

#[::tauri::command]
pub fn import_config(
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
        if let Err(err) = state.audio.start(audio_settings_from_config(&cfg), fallback_vst, logger.clone()) {
            logger.error(format!("Audio not restarted after import: {err}"));
        }
    } else {
        state.audio.stop(Some(app));
    }

    Ok(cfg)
}

#[::tauri::command]
pub fn get_app_paths() -> Result<AppPaths, String> {
    AppPaths::new()
}

#[::tauri::command]
pub fn open_app_dir(target: String) -> Result<(), String> {
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

#[::tauri::command]
pub fn clear_log_file() -> Result<(), String> {
    logger::clear_log_file()
}
