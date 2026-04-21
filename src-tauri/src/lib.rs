// Bibliothèque OSC-MIDI Bridge
// Expose les modules publics pour les tests et réutilisation

pub mod audio;
pub mod bridge;
pub mod config;
pub mod error;
pub mod logger;
pub mod midi;
pub mod osc;
pub mod plugin_probe;
pub mod rtp;
pub mod types;
pub mod vst_scan;

// Réexporter les types les plus courants pour faciliter l'utilisation
pub use audio::{AudioEngine, AudioStreamConfig, ConfigError};
pub use bridge::BridgeHandle;
pub use config::{Config, ConfigStore};
pub use error::{AppError, AudioError, BridgeError, ConfigError as AppConfigError, TauriError};
pub use midi::MidiFrame;
pub use types::{
    BridgeStatus, BridgeMetrics, MidiActivityInfo, 
    VstPluginEntry, VstParameter, PreflightReport
};
