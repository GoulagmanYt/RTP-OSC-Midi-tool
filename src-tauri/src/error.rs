//! Module d'erreurs unifié pour l'application OSC-MIDI Bridge
//! 
//! Ce module définit des types d'erreurs spécifiques pour chaque domaine
//! de l'application, remplaçant progressivement l'utilisation de `String`
//! comme type d'erreur générique.

use std::fmt;
use thiserror::Error;

/// Erreur principale de l'application
#[derive(Error, Debug, Clone)]
pub enum AppError {
    /// Erreur liée au moteur audio
    #[error("Erreur audio: {0}")]
    Audio(#[from] AudioError),
    
    /// Erreur liée au bridge MIDI/RTP
    #[error("Erreur bridge: {0}")]
    Bridge(#[from] BridgeError),
    
    /// Erreur liée à la configuration
    #[error("Erreur configuration: {0}")]
    Config(#[from] ConfigError),
    
    /// Erreur liée à l'interface Tauri
    #[error("Erreur interface: {0}")]
    Tauri(#[from] TauriError),
    
    /// Erreur générique avec message
    #[error("Erreur: {0}")]
    Generic(String),
}

/// Erreurs spécifiques au moteur audio
#[derive(Error, Debug, Clone)]
pub enum AudioError {
    #[error("Plugin VST non trouvé: {0}")]
    PluginNotFound(String),
    
    #[error("Plugin VST non chargé")]
    PluginNotLoaded,
    
    #[error("Erreur de stream audio: {0}")]
    StreamError(String),
    
    #[error("Configuration audio invalide: {0}")]
    InvalidConfig(String),
    
    #[error("Erreur de callback audio: {0}")]
    CallbackError(String),
    
    #[error("Erreur de fenêtre VST: {0}")]
    WindowError(String),
    
    #[error("Erreur de périphérique audio: {0}")]
    DeviceError(String),
}

/// Erreurs spécifiques au bridge MIDI/RTP
#[derive(Error, Debug, Clone)]
pub enum BridgeError {
    #[error("Erreur MIDI: {0}")]
    Midi(String),
    
    #[error("Erreur RTP: {0}")]
    Rtp(String),
    
    #[error("Erreur OSC: {0}")]
    Osc(String),
    
    #[error("Bridge non démarré")]
    NotStarted,
    
    #[error("Bridge déjà en cours d'exécution")]
    AlreadyRunning,
    
    #[error("Erreur de communication: {0}")]
    Communication(String),
    
    #[error("Erreur de traitement MIDI: {0}")]
    Processing(String),
}

/// Erreurs spécifiques à la configuration
#[derive(Error, Debug, Clone)]
pub enum ConfigError {
    #[error("Fichier de configuration non trouvé")]
    FileNotFound,
    
    #[error("Erreur de lecture du fichier: {0}")]
    ReadError(String),
    
    #[error("Erreur d'écriture du fichier: {0}")]
    WriteError(String),
    
    #[error("Erreur de parsing JSON: {0}")]
    ParseError(String),
    
    #[error("Configuration invalide: {0}")]
    Invalid(String),
    
    #[error("Chemin de configuration invalide: {0}")]
    InvalidPath(String),
}

/// Erreurs spécifiques à l'interface Tauri
#[derive(Error, Debug, Clone)]
pub enum TauriError {
    #[error("Erreur de commande: {0}")]
    Command(String),
    
    #[error("Erreur d'état: {0}")]
    State(String),
    
    #[error("Erreur de fenêtre: {0}")]
    Window(String),
    
    #[error("Erreur de dialogue: {0}")]
    Dialog(String),
    
    #[error("Erreur d'événement: {0}")]
    Event(String),
}

// Implémentations de conversion pour la compatibilité
impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Generic(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Generic(s.to_string())
    }
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

// Conversions depuis les erreurs existantes
impl From<crate::audio::engine::AudioError> for AudioError {
    fn from(err: crate::audio::engine::AudioError) -> Self {
        match err {
            crate::audio::engine::AudioError::Message(msg) => 
                AudioError::StreamError(msg),
            crate::audio::engine::AudioError::Config(config_err) => 
                AudioError::InvalidConfig(config_err.to_string()),
            crate::audio::engine::AudioError::AlreadyStarted => 
                AudioError::StreamError("Le moteur audio est déjà démarré".to_string()),
            crate::audio::engine::AudioError::NotStarted => 
                AudioError::StreamError("Le moteur audio n'est pas démarré".to_string()),
            crate::audio::engine::AudioError::PluginNotLoaded => 
                AudioError::PluginNotLoaded,
        }
    }
}

impl From<crate::audio::engine::AudioError> for AppError {
    fn from(err: crate::audio::engine::AudioError) -> Self {
        AppError::Audio(AudioError::from(err))
    }
}

// Macro pour faciliter la conversion
#[macro_export]
macro_rules! err {
    ($variant:ident, $msg:expr) => {
        AppError::$variant($variant($msg.to_string()))
    };
    ($variant:ident) => {
        AppError::$variant($variant)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversions() {
        let string_err: AppError = "test error".into();
        assert!(matches!(string_err, AppError::Generic(_)));
        
        let audio_err = AudioError::PluginNotFound("test.vst".to_string());
        let app_err: AppError = audio_err.into();
        assert!(matches!(app_err, AppError::Audio(_)));
    }

    #[test]
    fn test_error_display() {
        let err = AudioError::PluginNotFound("test.vst".to_string());
        let display = format!("{}", err);
        assert!(display.contains("Plugin VST non trouvé"));
    }
}
