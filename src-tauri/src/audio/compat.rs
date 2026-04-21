//! Compatibilité avec l'ancienne API
//! 
//! Ce module contient les types et fonctions nécessaires pour maintenir
//! la compatibilité avec le code existant pendant la transition.

use cpal::{BufferSize, SampleRate, StreamConfig};
use crate::audio::config::AudioStreamConfig;
use crate::audio::engine::AudioError;

/// Ancienne structure AudioSettings pour compatibilité
#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
pub struct AudioSettings {
    pub enabled: bool,
    pub backend: Option<String>,
    pub device: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub gain_db: f32,
    pub limiter_enabled: bool,
    pub vst_path: Option<std::path::PathBuf>,
}

/// Convertit AudioSettings vers AudioStreamConfig
#[allow(dead_code)]
pub fn settings_to_config(settings: AudioSettings) -> AudioStreamConfig {
    AudioStreamConfig {
        backend: settings.backend,
        device: settings.device,
        cpal_config: StreamConfig {
            channels: 2,
            sample_rate: SampleRate(settings.sample_rate),
            buffer_size: BufferSize::Fixed(settings.buffer_size),
        },
        plugin_config: crate::audio::config::PluginConfig {
            sample_rate: settings.sample_rate,
            buffer_size: settings.buffer_size,
            limiter_enabled: settings.limiter_enabled,
        },
        gain_config: crate::audio::config::GainConfig {
            gain_db: settings.gain_db,
        },
        vst_path: settings.vst_path,
    }
}

/// Obtient le HWND d'une fenêtre Tauri (Windows uniquement)
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub fn get_window_hwnd(app: &tauri::AppHandle) -> Result<std::ptr::NonNull<()>, AudioError> {
    use tauri::Manager;
    
    if let Some(_window) = app.get_webview_window("main") {
        // Pour l'instant, nous utilisons une valeur factice
        // TODO: Implémenter la récupération réelle du HWND
        Ok(std::ptr::NonNull::new(0x12345678 as *mut ()).unwrap())
    } else {
        Err(AudioError::Message("Fenêtre principale non trouvée".to_string()))
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn get_window_hwnd(_app: &tauri::AppHandle) -> Result<std::ptr::NonNull<()>, AudioError> {
    Err(AudioError::Message("Interface VST non supportée sur cette plateforme".to_string()))
}
