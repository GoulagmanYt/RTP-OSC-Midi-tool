//! Module d'erreurs unifié pour l'application OSC-MIDI Bridge.

use std::collections::BTreeMap;

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub domain: String,
    pub message: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub details: BTreeMap<String, String>,
}

impl CommandError {
    pub fn new(
        code: impl Into<String>,
        domain: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            domain: domain.into(),
            message: message.into(),
            details: BTreeMap::new(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::new("app.failure", "app", message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::new("app.failure", "app", message)
    }
}

#[derive(Error, Debug, Clone, Serialize, PartialEq, Eq)]
pub enum AppError {
    #[error("Erreur audio: {0}")]
    Audio(#[from] AudioError),
    #[error("Erreur bridge: {0}")]
    Bridge(#[from] BridgeError),
    #[error("Erreur configuration: {0}")]
    Config(#[from] ConfigError),
    #[error("Erreur interface: {0}")]
    Tauri(#[from] TauriError),
    #[error("Erreur: {0}")]
    Generic(String),
}

#[derive(Error, Debug, Clone, Serialize, PartialEq, Eq)]
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

#[derive(Error, Debug, Clone, Serialize, PartialEq, Eq)]
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

#[derive(Error, Debug, Clone, Serialize, PartialEq, Eq)]
pub enum ConfigError {
    #[error("Fichier de configuration non trouvé")]
    FileNotFound,
    #[error("Erreur de lecture du fichier: {0}")]
    ReadError(String),
    #[error("Erreur d'écriture du fichier: {0}")]
    WriteError(String),
    #[error("Erreur de parsing YAML: {0}")]
    ParseError(String),
    #[error("Configuration invalide: {0}")]
    Invalid(String),
    #[error("Chemin de configuration invalide: {0}")]
    InvalidPath(String),
}

#[derive(Error, Debug, Clone, Serialize, PartialEq, Eq)]
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

impl From<AppError> for CommandError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Audio(err) => CommandError::new("audio.failure", "audio", err.to_string()),
            AppError::Bridge(err) => CommandError::new("bridge.failure", "bridge", err.to_string()),
            AppError::Config(err) => CommandError::new("config.failure", "config", err.to_string()),
            AppError::Tauri(err) => CommandError::new("tauri.failure", "tauri", err.to_string()),
            AppError::Generic(message) => CommandError::new("app.failure", "app", message),
        }
    }
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

impl From<crate::audio::AudioError> for AudioError {
    fn from(err: crate::audio::AudioError) -> Self {
        match err {
            crate::audio::AudioError::Message(msg) => AudioError::StreamError(msg),
        }
    }
}

impl From<crate::audio::AudioError> for AppError {
    fn from(err: crate::audio::AudioError) -> Self {
        AppError::Audio(AudioError::from(err))
    }
}

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

    #[test]
    fn command_error_serializes_details() {
        let mut err = CommandError::new("config.invalid", "config", "Configuration invalide");
        err.details
            .insert("field".to_string(), "audio.sampleRate".to_string());
        let json = serde_json::to_string(&err).expect("json");
        assert!(json.contains("\"field\":\"audio.sampleRate\""));
    }
}
