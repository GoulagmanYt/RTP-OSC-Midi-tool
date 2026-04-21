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
pub use runtime::BridgeHandle;
