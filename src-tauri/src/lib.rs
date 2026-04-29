// Bibliothèque OSC-MIDI Bridge
// Expose les modules publics pour les tests et réutilisation

pub mod application;
pub mod audio;
pub mod bridge;
pub mod config;
pub mod error;
pub mod logger;
pub mod midi;
pub mod osc;
pub mod plugin_probe;
pub mod reliable_playback;
pub mod rtp;
pub mod tauri;
pub mod types;
pub mod vst_scan;

// Réexporter les types les plus courants pour faciliter l'utilisation
pub use audio::AudioEngine;
pub use bridge::BridgeHandle;
pub use config::{
    AppConfig, AudioConfig, AvatarConfig, Config, ConfigStore, LoggingConfig, MidiConfig,
    OscConfig, RoutingAssignment, RoutingMapping, RoutingProfile, RtpConfig, RtpRemoteEntry, Theme,
    UiConfig,
};
pub use error::{
    AppError, AudioError, BridgeError, CommandError, ConfigError as AppConfigError, TauriError,
};
pub use midi::MidiFrame;
pub use types::{
    AppPaths, BridgeMetrics, BridgeStatus, LogEntry, MidiActivityInfo, MidiActivitySnapshot,
    PreflightReport, RuntimeMetrics, RuntimeStatus, VstParameter, VstPluginEntry,
};
