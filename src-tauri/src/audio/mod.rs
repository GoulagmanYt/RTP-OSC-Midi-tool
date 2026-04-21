//! Module audio - Moteur audio et gestion VST
//! 
//! Ce module contient toute la logique audio du projet:
//! - Configuration audio unifiée
//! - Moteur audio principal
//! - Gestion des streams CPAL
//! - Interface VST2/3
//! - Callbacks temps réel

pub mod compat;
pub mod config;
pub mod engine;

// Réexporter les types publics pour faciliter l'utilisation
pub use compat::{AudioSettings, settings_to_config};
pub use config::{AudioStreamConfig, ConfigError, db_to_linear, linear_to_db};
pub use engine::AudioEngine;
