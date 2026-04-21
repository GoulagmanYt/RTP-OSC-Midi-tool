//! Module Bridge - Communication MIDI/RTP/OSC
//! 
//! Ce module contient toute la logique de communication bridge:
//! - Gestion MIDI (entrée/sortie, routage)
//! - Communication OSC avec VRChat
//! - Communication RTP avec les participants
//! - Configuration et métriques

pub mod config;
pub mod midi;
pub mod osc;
pub mod rtp_bridge;
pub mod activity;
pub mod runtime;

// Réexporter les types publics pour faciliter l'utilisation
pub use config::{BridgeConfig, ConfigSnapshot};
pub use midi::{MidiManager, MidiPortSelector};
pub use osc::{OscManager, OscMessageHandler};
pub use rtp_bridge::{RtpManager, RtpTargetResolver};
pub use activity::{ActivityTracker, MidiActivityInfo};
pub use runtime::{BridgeHandle, BridgeRuntime};
