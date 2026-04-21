//! Fonctions utilitaires pour l'interface Tauri

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::config::Config;
use directories::ProjectDirs;
use crate::rtp::RtpDiscoveryManager;
use tauri::AppHandle;
use crate::types::VstPluginEntry;

use crate::tauri::state::AppState;

/// Retourne le chemin du répertoire de configuration
pub fn config_dir_path() -> Result<PathBuf, String> {
    let proj_dirs = ProjectDirs::from("com", "osc-midi", "OSCMidi")
        .ok_or("Failed to get project directories")?;
    Ok(proj_dirs.config_dir().to_path_buf())
}

/// Retourne le chemin du cache VST
pub fn vst_cache_path() -> Result<PathBuf, String> {
    Ok(config_dir_path()?.join("vst_cache.json"))
}

/// Vérifie si une entrée VST est un instrument
pub fn is_instrument_entry(entry: &VstPluginEntry) -> bool {
    entry.kind == "instrument" && entry.supported
}

/// Filtre les entrées VST pour ne garder que les instruments
pub fn retain_instrument_entries(entries: Vec<VstPluginEntry>) -> Vec<VstPluginEntry> {
    entries
        .into_iter()
        .filter(is_instrument_entry)
        .collect::<Vec<_>>()
}

/// Charge le cache VST depuis le disque
pub fn load_vst_cache_from_disk() -> Option<Vec<VstPluginEntry>> {
    let path = vst_cache_path().ok()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Vec<VstPluginEntry>>(&raw)
        .ok()
        .map(retain_instrument_entries)
}

/// Sauvegarde le cache VST sur le disque
pub fn save_vst_cache_to_disk(entries: &[VstPluginEntry]) {
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

/// Ouvre un dossier dans l'explorateur
pub fn open_folder_in_explorer(path: &Path) -> Result<(), String> {
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
}

/// Synchronise la configuration avec le runtime logging
pub fn sync_runtime_logging(config: &Config, dev_logging: &std::sync::atomic::AtomicBool) {
    crate::logger::set_global_dev_mode(config.verbose);
    dev_logging.store(config.verbose, std::sync::atomic::Ordering::Relaxed);
}

/// Synchronise la découverte RTP avec la configuration
pub fn sync_rtp_discovery(config: &Config, state: &AppState, _app: &AppHandle) {
    if config.rtp_remote_enabled {
        if let Some(_) = state.rtp_manager() {
            // Déjà initialisé
            return;
        }
        
        let manager = RtpDiscoveryManager::new();
        // Note: start_discovery n'existe pas, nous utilisons juste le manager
        state.set_rtp_manager(manager);
    } else {
        if let Some(_) = state.rtp_manager() {
            *state.rtp_discovery.lock() = None;
        }
    }
}

/// Convertit les paramètres audio depuis la configuration
pub fn audio_settings_from_config(config: &Config) -> crate::audio::AudioSettings {
    crate::audio::AudioSettings {
        enabled: config.audio_enabled,
        backend: config.audio_backend.clone(),
        device: config.audio_device.clone(),
        sample_rate: config.audio_sample_rate,
        buffer_size: config.audio_buffer_size,
        gain_db: config.audio_gain_db,
        limiter_enabled: config.audio_limiter_enabled,
        vst_path: config.vst_path.as_ref().map(|s| std::path::PathBuf::from(s)),
    }
}

/// Exporte les diagnostics de l'application
pub fn export_diagnostics(state: &AppState) -> Result<String, String> {
    let diagnostics = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "audio": {
            "backends": state.audio.list_backends(),
            "devices": state.audio.list_devices(None),
            "is_running": state.audio.is_running(),
            "is_vst_loaded": state.audio.is_vst_loaded(),
        },
        "bridge": state.bridge_handle().map(|b: crate::bridge::BridgeHandle| b.status()),
        "rtp_participants": state.rtp_manager()
            .map(|m: crate::rtp::RtpDiscoveryManager| m.cached())
            .unwrap_or_default(),
    });
    
    serde_json::to_string_pretty(&diagnostics).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
