//! Commandes Tauri - Interface entre le frontend et le backend

use std::sync::Arc;
use parking_lot::Mutex;

use crate::bridge::BridgeHandle;
use crate::config::{Config, RTP_VIRTUAL_INPUT, VST_INTERNAL_OUTPUT};
use crate::error::{AppError, ConfigError as AppConfigError, TauriError, BridgeError};
use crate::logger::{FrontendLogger, set_log_all_to_file, set_logs_enabled};
use crate::types::{
    BridgeStatus, PreflightReport, RtpParticipantInfo, VstPluginEntry,
};

use tauri::{AppHandle, Manager, State, Window};
use midir::{MidiInput, MidiOutput};

use crate::tauri::{
    state::AppState,
    utils::{audio_settings_from_config, export_diagnostics, sync_rtp_discovery, sync_runtime_logging},
};

#[cfg(test)]
mod tests;

/// Récupère la configuration actuelle
#[::tauri::command]
pub fn get_config(app: AppHandle, state: State<AppState>) -> Config {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.log_all_to_file);
    set_logs_enabled(cfg.logs_enabled);
    sync_rtp_discovery(&cfg, &state, &app);
    cfg
}

/// Sauvegarde la configuration
#[::tauri::command]
pub fn save_config(window: Window, config: Config, state: State<AppState>) -> Result<(), AppError> {
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.log_all_to_file);
    set_logs_enabled(config.logs_enabled);
    state.config_store.save(&config)?;
    sync_rtp_discovery(&config, &state, window.app_handle());
    let _logger = FrontendLogger::new(window, state.dev_logging.clone());
    // TODO: Implémenter sync_rtp et update_config dans BridgeHandle
    Ok(())
}

/// Liste les ports d'entrée MIDI disponibles
#[::tauri::command]
pub fn list_midi_inputs() -> Result<Vec<String>, String> {
    let input = MidiInput::new("OSCMidi").map_err(|e: midir::InitError| e.to_string())?;
    let mut list: Vec<String> = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();
    list.insert(0, RTP_VIRTUAL_INPUT.to_string());
    Ok(list)
}

/// Liste les ports de sortie MIDI disponibles
#[::tauri::command]
pub fn list_midi_outputs() -> Result<Vec<String>, String> {
    let output = MidiOutput::new("OSCMidi").map_err(|e: midir::InitError| e.to_string())?;
    let mut list = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<String>>();
    list.push(VST_INTERNAL_OUTPUT.to_string());
    Ok(list)
}

/// Démarre le bridge MIDI
#[::tauri::command]
pub fn start_bridge(
    _app: AppHandle,
    window: Window,
    state: State<AppState>,
) -> Result<(), AppError> {
    let config = state.config_store.load();
    let audio_settings = audio_settings_from_config(&config);
    
    // Arrêter le bridge existant s'il y en a un
    if let Some(bridge) = state.bridge_handle() {
        let logger = FrontendLogger::new(window.clone(), state.dev_logging.clone());
        let _ = bridge.stop(&logger);
    }
    
    // Démarrer le moteur audio si nécessaire
    if audio_settings.enabled {
        let audio_config = crate::audio::config::AudioStreamConfig::default();
        let logger = FrontendLogger::new(window.clone(), state.dev_logging.clone());
        let vst_path = config.vst_path.as_ref().map(|s| std::path::PathBuf::from(s));
        state.audio.start(audio_config, vst_path, logger)?;
    }
    
    // Créer et démarrer le bridge
    let bridge = BridgeHandle::new();
    let logger = FrontendLogger::new(window.clone(), state.dev_logging.clone());
    bridge.start(
        Arc::new(Mutex::new(config.clone())),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        state.audio.clone(),
        logger,
    ).map_err(|e| BridgeError::Communication(e))?;
    
    state.set_bridge(bridge);
    Ok(())
}

/// Arrête le bridge MIDI
#[::tauri::command]
pub fn stop_bridge(app: AppHandle, window: Window, state: State<AppState>) -> Result<(), AppError> {
    if let Some(bridge) = state.bridge_handle() {
        let logger = FrontendLogger::new(window.clone(), state.dev_logging.clone());
        bridge.stop(&logger).map_err(|e| BridgeError::Communication(e))?;
    }
    
    // Arrêter le moteur audio
    state.audio.stop(Some(app.clone()))?;
    
    // Effacer le bridge en mettant None dans le champ interne
    *state.bridge.lock() = None;
    Ok(())
}

/// Récupère le statut du bridge
#[::tauri::command]
pub fn get_bridge_status(state: State<AppState>) -> BridgeStatus {
    state.bridge_handle()
        .map(|b: BridgeHandle| b.status())
        .unwrap_or_default()
}

/// Récupère les métriques du bridge
#[::tauri::command]
pub fn get_bridge_metrics(state: State<AppState>) -> Option<crate::types::BridgeMetrics> {
    state.bridge_handle().map(|b: BridgeHandle| b.get_metrics())
}

/// Envoie une trame MIDI via le bridge
#[::tauri::command]
pub fn send_midi_frame(
    frame: crate::midi::MidiFrame,
    state: State<AppState>,
) -> Result<(), AppError> {
    if let Some(bridge) = state.bridge_handle() {
        bridge.send_midi_frame(frame).map_err(|e| BridgeError::Communication(e))?;
    Ok(())
    } else {
        Err(BridgeError::NotStarted.into())
    }
}

/// Liste les participants RTP
#[::tauri::command]
pub fn list_rtp_participants(state: State<AppState>) -> Vec<RtpParticipantInfo> {
    let sessions = state.rtp_manager()
        .map(|m: crate::rtp::RtpDiscoveryManager| m.cached())
        .unwrap_or_default();
    
    // Convertir RtpSessionInfo en RtpParticipantInfo
    sessions.into_iter().map(|session| RtpParticipantInfo {
        name: session.name,
        addr: format!("{}:{}", session.host, session.port),
    }).collect()
}

/// Exporte les diagnostics de l'application
#[::tauri::command]
pub fn export_app_diagnostics(state: State<'_, AppState>) -> Result<String, String> {
    export_diagnostics(&state)
}

/// Commande pour obtenir les chemins de l'application
#[::tauri::command]
pub fn get_app_paths() -> Result<crate::types::AppPaths, String> {
    crate::types::AppPaths::new()
}

/// Importe une configuration depuis un fichier
#[::tauri::command]
pub async fn import_config(
    app: AppHandle,
    window: Window,
    state: State<'_, AppState>,
) -> Result<Config, AppError> {
    use tauri_plugin_dialog::DialogExt;
    
    let file_path = window.dialog()
        .file()
        .add_filter("JSON Files", &["json"])
        .set_file_name("config.json")
        .set_title("Import Configuration")
        .blocking_pick_file()
        .ok_or(TauriError::Dialog("No file selected".to_string()))?;
    
    let content = std::fs::read_to_string(file_path.as_path().ok_or_else(|| TauriError::Dialog("Invalid file path".to_string()))?)
        .map_err(|e| AppConfigError::ReadError(e.to_string()))?;
    
    let config: Config = serde_json::from_str(&content)
        .map_err(|e| AppConfigError::ParseError(e.to_string()))?;
    
    // Sauvegarder la configuration importée
    state.config_store.save(&config)
        .map_err(|e| AppConfigError::WriteError(e.to_string()))?;
    
    // Synchroniser avec l'état actuel
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.log_all_to_file);
    set_logs_enabled(config.logs_enabled);
    sync_rtp_discovery(&config, &state, &app);
    
    Ok(config)
}

/// Exporte la configuration vers un fichier
#[::tauri::command]
pub async fn export_config(
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    use tauri_plugin_dialog::DialogExt;
    
    let config = state.config_store.load();
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| AppConfigError::ParseError(e.to_string()))?;
    
    let file_path = window.dialog()
        .file()
        .add_filter("JSON Files", &["json"])
        .set_file_name("config.json")
        .set_title("Export Configuration")
        .blocking_save_file()
        .ok_or(TauriError::Dialog("No file selected".to_string()))?;
    
    std::fs::write(file_path.as_path().ok_or(TauriError::Dialog("Invalid file path".to_string()))?, content)
        .map_err(|e| AppConfigError::WriteError(e.to_string()))?;
    
    Ok(())
}

/// Liste les racines de scan VST
#[::tauri::command]
pub fn list_vst_scan_roots() -> Result<Vec<String>, String> {
    let roots = crate::vst_scan::default_vst_scan_roots();
    Ok(roots.into_iter().map(|p| p.to_string_lossy().to_string()).collect())
}

/// Scan les plugins VST
#[::tauri::command]
pub fn scan_vst_plugins(
    roots: Vec<String>,
    window: Window,
    state: State<AppState>,
) -> Result<Vec<VstPluginEntry>, String> {
    let _logger = FrontendLogger::new(window, state.dev_logging.clone());
    
    // Convertir les chemins en PathBuf
    let path_roots: Vec<std::path::PathBuf> = roots.into_iter()
        .map(|s| std::path::PathBuf::from(s))
        .collect();
    
    let entries = crate::vst_scan::scan_vst_plugins_in_roots(&path_roots);
    
    // Sauvegarder en cache
    crate::tauri::utils::save_vst_cache_to_disk(&entries);
    
    Ok(entries)
}

/// Charge le cache VST
#[::tauri::command]
pub fn load_vst_cache() -> Option<Vec<VstPluginEntry>> {
    crate::tauri::utils::load_vst_cache_from_disk()
}

/// Ouvre un dossier dans l'explorateur
#[::tauri::command]
pub fn open_folder(path: String) -> Result<(), AppError> {
    crate::tauri::utils::open_folder_in_explorer(std::path::Path::new(&path)).map_err(|e| TauriError::Window(e.to_string()))?;
    Ok(())
}

/// Génère un rapport de pré-vol
#[::tauri::command]
pub fn generate_preflight_report(state: State<'_, AppState>) -> PreflightReport {
    let _config = state.config_store.load();
    
    PreflightReport {
        midi_in_ok: true, // Simplifié pour l'instant
        midi_out_ok: true, // Simplifié pour l'instant
        audio_backend_ok: true, // Simplifié pour l'instant
        rtp_port_ok: true, // Simplifié pour l'instant
        messages: vec![], // Simplifié pour l'instant
    }
}
