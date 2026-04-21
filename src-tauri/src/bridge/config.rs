//! Configuration du Bridge
//! 
//! Ce module contient les structures de configuration pour le bridge MIDI/RTP/OSC
//! et les fonctions de conversion depuis la configuration principale.

use crate::{
    config::{
        Config, RoutingAssignment, RoutingMapping, RoutingProfile,
    },
    logger::FrontendLogger,
    rtp::RtpRemoteTarget,
};
use std::collections::HashMap;

/// Configuration du bridge pour le runtime
#[derive(Clone)]
pub struct BridgeConfig {
    /// Configuration MIDI
    pub midi_in: Option<String>,
    pub midi_out: Option<String>,
    /// Configuration RTP
    pub rtp_enabled: bool,
    pub rtp_port: u16,
    pub rtp_remote_enabled: bool,
    pub rtp_remote_targets: Vec<RtpRemoteTarget>,
    pub rtp_log: bool,
    /// Configuration OSC
    pub osc_enabled: bool,
    pub osc_target: Option<String>,
    /// Profils de routage
    pub routing_profiles: Vec<RoutingProfileRuntime>,
    /// Assignations de routage par source
    pub routing_assignments: HashMap<String, usize>,
}

/// Profil de routage au runtime
#[derive(Clone)]
pub struct RoutingProfileRuntime {
    pub id: String,
    pub channel_filter: Option<u8>,
    pub note_min: Option<u8>,
    pub note_max: Option<u8>,
    pub cc_map: [u8; 128],
    pub program_map: [u8; 128],
    pub enabled: bool,
}

/// Snapshot de configuration pour le traitement
#[derive(Clone)]
pub struct ConfigSnapshot {
    pub midi_in: Option<String>,
    pub midi_out: Option<String>,
    pub rtp_enabled: bool,
    pub rtp_port: u16,
    pub rtp_remote_enabled: bool,
    pub rtp_remote_targets: Vec<RtpRemoteTarget>,
    pub rtp_log: bool,
    pub osc_enabled: bool,
    pub osc_target: Option<String>,
    pub routing_profiles: Vec<RoutingProfileRuntime>,
    pub routing_assignments: HashMap<String, usize>,
}

impl RoutingProfileRuntime {
    /// Crée un runtime de profil depuis une configuration
    pub fn from_profile(profile: &RoutingProfile) -> Self {
        let mut cc_map = [0u8; 128];
        let mut program_map = [0u8; 128];
        
        // Initialiser les maps avec l'identité
        for i in 0..128u8 {
            cc_map[i as usize] = i;
            program_map[i as usize] = i;
        }
        
        // Appliquer les mappings CC
        for RoutingMapping { from, to } in &profile.cc_map {
            if *from < 128 && *to < 128 {
                cc_map[*from as usize] = *to;
            }
        }
        
        // Appliquer les mappings de programme
        for RoutingMapping { from, to } in &profile.program_map {
            if *from < 128 && *to < 128 {
                program_map[*from as usize] = *to;
            }
        }
        
        Self {
            id: profile.id.clone(),
            channel_filter: profile.channel_filter,
            note_min: profile.note_min,
            note_max: profile.note_max,
            cc_map,
            program_map,
            enabled: profile.enabled,
        }
    }
    
    /// Applique le profil de routage à une trame MIDI
    pub fn apply_to_frame(&self, frame: &mut crate::midi::MidiFrame) -> bool {
        let Some(status) = frame.data.first().copied() else {
            return true;
        };
        
        let channel = status & 0x0F;
        let status_type = status & 0xF0;
        
        // Filtrer par canal si nécessaire
        if let Some(filter) = self.channel_filter {
            if channel != filter {
                return false;
            }
        }
        
        // Filtrer par note si nécessaire
        if matches!(status_type, 0x80 | 0x90) {
            if let Some(note) = frame.data.get(1) {
                if let Some(min) = self.note_min {
                    if *note < min {
                        return false;
                    }
                }
                if let Some(max) = self.note_max {
                    if *note > max {
                        return false;
                    }
                }
            }
        }
        
        // Appliquer les mappings CC
        if status_type == 0xB0 {
            if let Some(cc) = frame.data.get(1) {
                if *cc < 128 {
                    frame.data[1] = self.cc_map[*cc as usize];
                }
            }
        }
        
        // Appliquer les mappings de programme
        if status_type == 0xC0 {
            if let Some(program) = frame.data.get(1) {
                if *program < 128 {
                    frame.data[1] = self.program_map[*program as usize];
                }
            }
        }
        
        true
    }
}

impl From<&Config> for BridgeConfig {
    fn from(cfg: &Config) -> Self {
        Self {
            midi_in: cfg.midi_in.clone(),
            midi_out: cfg.midi_out.clone(),
            rtp_enabled: cfg.rtp_enabled,
            rtp_port: cfg.rtp_port,
            rtp_remote_enabled: cfg.rtp_remote_enabled,
            rtp_remote_targets: vec![], // Convertir depuis rtp_remotes plus tard
            rtp_log: cfg.log_rtp,
            osc_enabled: cfg.osc_enabled,
            osc_target: Some(format!("{}:{}", cfg.osc_target_ip, cfg.osc_target_port)),
            routing_profiles: cfg.routing_profiles
                .iter()
                .map(RoutingProfileRuntime::from_profile)
                .collect(),
            routing_assignments: cfg.routing_assignments
                .iter()
                .enumerate()
                .map(|(i, assignment)| (assignment.source.clone(), i))
                .collect(),
        }
    }
}

impl From<&BridgeConfig> for ConfigSnapshot {
    fn from(config: &BridgeConfig) -> Self {
        Self {
            midi_in: config.midi_in.clone(),
            midi_out: config.midi_out.clone(),
            rtp_enabled: config.rtp_enabled,
            rtp_port: config.rtp_port,
            rtp_remote_enabled: config.rtp_remote_enabled,
            rtp_remote_targets: config.rtp_remote_targets.clone(),
            rtp_log: config.rtp_log,
            osc_enabled: config.osc_enabled,
            osc_target: config.osc_target.clone(),
            routing_profiles: config.routing_profiles.clone(),
            routing_assignments: config.routing_assignments.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_routing_profile_creation() {
        let profile = RoutingProfile {
            id: "test".to_string(),
            channel_filter: Some(1),
            note_min: Some(60),
            note_max: Some(72),
            cc_map: vec![
                RoutingMapping { from: 1, to: 10 },
                RoutingMapping { from: 2, to: 20 },
            ],
            program_map: vec![
                RoutingMapping { from: 5, to: 15 },
            ],
            enabled: true,
        };
        
        let runtime = RoutingProfileRuntime::from_profile(&profile);
        
        assert_eq!(runtime.id, "test");
        assert_eq!(runtime.channel_filter, Some(1));
        assert_eq!(runtime.cc_map[1], 10);
        assert_eq!(runtime.cc_map[2], 20);
        assert_eq!(runtime.program_map[5], 15);
    }
    
    #[test]
    fn test_routing_profile_filtering() {
        let profile = RoutingProfile {
            id: "test".to_string(),
            channel_filter: Some(2),
            note_min: Some(60),
            note_max: Some(72),
            cc_map: vec![],
            program_map: vec![],
            enabled: true,
        };
        
        let runtime = RoutingProfileRuntime::from_profile(&profile);
        
        // Test de filtrage par canal
        let mut frame = crate::midi::MidiFrame {
            source: "test".into(),
            timestamp_ms: 0,
            data: vec![0x90, 60, 100], // Note On canal 0
        };
        
        assert!(!runtime.apply_to_frame(&mut frame)); // Rejeté (mauvais canal)
        
        frame.data[0] = 0x92; // Note On canal 2
        assert!(runtime.apply_to_frame(&mut frame)); // Accepté
        
        // Test de filtrage par note
        frame.data[1] = 50; // Note hors plage
        assert!(!runtime.apply_to_frame(&mut frame)); // Rejeté
        
        frame.data[1] = 65; // Note dans plage
        assert!(runtime.apply_to_frame(&mut frame)); // Accepté
    }
}
