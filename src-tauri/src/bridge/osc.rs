//! Gestion OSC du Bridge
//! 
//! Ce module contient la logique de communication OSC avec VRChat,
//! incluant l'envoi de messages MIDI et la gestion des paramètres.

use crate::{
    logger::FrontendLogger,
    midi::MidiFrame,
    osc::OscClient,
};
use std::sync::Arc;

/// Gestionnaire de communication OSC
pub struct OscManager {
    client: Option<OscClient>,
    enabled: bool,
    target: Option<String>,
}

/// Gestionnaire des messages OSC
pub struct OscMessageHandler;

impl OscManager {
    /// Crée un nouveau gestionnaire OSC
    pub fn new() -> Self {
        Self {
            client: None,
            enabled: false,
            target: None,
        }
    }
    
    /// Initialise le client OSC
    pub fn initialize(&mut self, enabled: bool, target: Option<String>, logger: &FrontendLogger) {
        self.enabled = enabled;
        self.target = target.clone();
        
        if enabled && target.is_some() {
            // Pour l'instant, nous utilisons une configuration par défaut
            // L'implémentation complète nécessiterait de parser l'adresse IP et le port
            match OscClient::new("127.0.0.1", 9000) {
                Ok(client) => {
                    self.client = Some(client);
                    logger.info(format!("OSC connecté à: 127.0.0.1:9000"));
                }
                Err(e) => {
                    logger.error(format!("Erreur connexion OSC: {}", e));
                    self.enabled = false;
                }
            }
        } else {
            self.client = None;
            if enabled {
                logger.warn("OSC activé mais aucune cible spécifiée");
            }
        }
    }
    
    /// Envoie une trame MIDI via OSC
    pub fn send_midi_frame(&self, frame: &MidiFrame, logger: &FrontendLogger) {
        if !self.enabled {
            return;
        }
        
        let Some(ref client) = self.client else {
            return;
        };
        
        // Traiter les différents types de messages MIDI
        self.process_midi_message(frame, client, logger);
    }
    
    /// Envoie un message de sustain OSC
    pub fn send_sustain(&self, value: f32, logger: &FrontendLogger) {
        if !self.enabled {
            return;
        }
        
        if let Some(ref client) = self.client {
            let sustain_bool = value >= 0.5;
            if let Err(e) = client.send_param("/avatar/parameters/sustain", sustain_bool) {
                logger.error(format!("Erreur envoi sustain OSC: {}", e));
            }
        }
    }
    
    /// Envoie un message de note OSC
    pub fn send_note(&self, note: u8, velocity: f32, logger: &FrontendLogger) {
        if !self.enabled {
            return;
        }
        
        if let Some(ref client) = self.client {
            let note_index = (note - crate::midi::NOTE_MIN) + 1;
            let note_path = format!("/avatar/parameters/{}", note_index);
            let note_bool = velocity > 0.0;
            
            if let Err(e) = client.send_param(&note_path, note_bool) {
                logger.error(format!("Erreur envoi note OSC: {}", e));
            }
        }
    }
    
    /// Ferme la connexion OSC
    pub fn close(&mut self, logger: &FrontendLogger) {
        if let Some(_) = self.client.take() {
            logger.info("OSC déconnecté");
        }
        self.enabled = false;
    }
    
    /// Vérifie si OSC est activé
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    /// Retourne la cible OSC actuelle
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
    
    // Méthodes privées
    
    /// Traite une trame MIDI et envoie les messages OSC appropriés
    fn process_midi_message(&self, frame: &MidiFrame, client: &OscClient, logger: &FrontendLogger) {
        if frame.data.is_empty() {
            return;
        }
        
        let status = frame.data[0];
        let channel = status & 0x0F;
        let status_type = status & 0xF0;
        
        match status_type {
            0x80 => {
                // Note Off
                if let Some(note) = frame.data.get(1) {
                    self.send_note_internal(client, *note, 0.0, logger);
                }
            }
            0x90 => {
                // Note On
                if frame.data.len() >= 3 {
                    let note = frame.data[1];
                    let velocity = frame.data[2];
                    let velocity_f32 = (velocity as f32) / 127.0;
                    self.send_note_internal(client, note, velocity_f32, logger);
                }
            }
            0xB0 => {
                // Control Change
                if frame.data.len() >= 3 {
                    let cc = frame.data[1];
                    let value = frame.data[2];
                    self.process_control_change(client, channel, cc, value, logger);
                }
            }
            0xC0 => {
                // Program Change
                if let Some(program) = frame.data.get(1) {
                    self.send_program_change_internal(client, *program, logger);
                }
            }
            0xE0 => {
                // Pitch Bend
                if frame.data.len() >= 3 {
                    let lsb = frame.data[1];
                    let msb = frame.data[2];
                    let bend_value = ((msb as u16) << 7) | (lsb as u16);
                    let bend_f32 = ((bend_value as f32) - 8192.0) / 8192.0; // -1.0 à +1.0
                    self.send_pitch_bend_internal(client, bend_f32, logger);
                }
            }
            _ => {
                // Autres messages MIDI non traités
                logger.debug(format!("Message MIDI non traité: 0x{:02X}", status));
            }
        }
    }
    
    /// Traite un message Control Change
    fn process_control_change(&self, client: &OscClient, channel: u8, cc: u8, value: u8, logger: &FrontendLogger) {
        match cc {
            64 => {
                // Sustain
                let sustain_bool = value >= 64;
                if let Err(e) = client.send_param("/avatar/parameters/sustain", sustain_bool) {
                    logger.error(format!("Erreur envoi sustain OSC: {}", e));
                }
            }
            1 => {
                // Modulation Wheel
                let mod_path = format!("/avatar/parameters/{}", 1 + 1); // CC 1 -> param 2
                let mod_bool = value > 0;
                if let Err(e) = client.send_param(&mod_path, mod_bool) {
                    logger.error(format!("Erreur envoi modulation OSC: {}", e));
                }
            }
            7 => {
                // Volume (Channel Volume) - mappé vers un paramètre
                let volume_path = format!("/avatar/parameters/{}", 7 + 1); // CC 7 -> param 8
                let volume_bool = value > 0;
                if let Err(e) = client.send_param(&volume_path, volume_bool) {
                    logger.error(format!("Erreur envoi volume OSC: {}", e));
                }
            }
            10 => {
                // Pan - mappé vers un paramètre
                let pan_path = format!("/avatar/parameters/{}", 10 + 1); // CC 10 -> param 11
                let pan_bool = value > 0;
                if let Err(e) = client.send_param(&pan_path, pan_bool) {
                    logger.error(format!("Erreur envoi pan OSC: {}", e));
                }
            }
            _ => {
                // Autres CC non traités spécifiquement
                logger.debug(format!("CC non traité: canal {}, cc {}, valeur {}", channel, cc, value));
            }
        }
    }
    
    /// Envoie un message de note (interne)
    fn send_note_internal(&self, client: &OscClient, note: u8, velocity: f32, logger: &FrontendLogger) {
        let note_index = (note - crate::midi::NOTE_MIN) + 1; // Convertir en index de paramètre
        let note_path = format!("/avatar/parameters/{}", note_index);
        let note_bool = velocity > 0.0;
        
        if let Err(e) = client.send_param(&note_path, note_bool) {
            logger.error(format!("Erreur envoi note OSC: {}", e));
        }
    }
    
    /// Envoie un message Program Change (interne)
    fn send_program_change_internal(&self, client: &OscClient, program: u8, logger: &FrontendLogger) {
        // Program change mappé vers un paramètre spécial
        let pc_path = "/avatar/parameters/program_change";
        let pc_bool = program > 0;
        
        if let Err(e) = client.send_param(pc_path, pc_bool) {
            logger.error(format!("Erreur envoi program change OSC: {}", e));
        }
    }
    
    /// Envoie un message Pitch Bend (interne)
    fn send_pitch_bend_internal(&self, client: &OscClient, bend: f32, logger: &FrontendLogger) {
        // Pitch bend mappé vers un paramètre spécial
        let pb_path = "/avatar/parameters/pitch_bend";
        let pb_bool = bend.abs() > 0.1; // Seuil pour détecter le pitch bend
        
        if let Err(e) = client.send_param(pb_path, pb_bool) {
            logger.error(format!("Erreur envoi pitch bend OSC: {}", e));
        }
    }
}

impl Default for OscManager {
    fn default() -> Self {
        Self::new()
    }
}

impl OscMessageHandler {
    /// Convertit une valeur MIDI (0-127) en valeur OSC (0.0-1.0)
    pub fn midi_to_osc(midi_value: u8) -> f32 {
        (midi_value as f32) / 127.0
    }
    
    /// Convertit une valeur OSC (0.0-1.0) en valeur MIDI (0-127)
    pub fn osc_to_midi(osc_value: f32) -> u8 {
        (osc_value.clamp(0.0, 1.0) * 127.0).round() as u8
    }
    
    /// Convertit une valeur de pitch bend (0-16383) en valeur OSC (-1.0 à +1.0)
    pub fn pitch_bend_to_osc(bend_value: u16) -> f32 {
        ((bend_value as f32) - 8192.0) / 8192.0
    }
    
    /// Convertit une valeur OSC (-1.0 à +1.0) en valeur de pitch bend (0-16383)
    pub fn osc_to_pitch_bend(osc_value: f32) -> u16 {
        let clamped = osc_value.clamp(-1.0, 1.0);
        ((clamped * 8192.0) + 8192.0).round() as u16
    }
    
    /// Convertit une valeur de pan (0-127) en valeur OSC (-1.0 à +1.0)
    pub fn pan_to_osc(pan_value: u8) -> f32 {
        ((pan_value as f32) / 127.0) * 2.0 - 1.0
    }
    
    /// Convertit une valeur OSC (-1.0 à +1.0) en valeur de pan (0-127)
    pub fn osc_to_pan(osc_value: f32) -> u8 {
        let clamped = osc_value.clamp(-1.0, 1.0);
        (((clamped + 1.0) / 2.0) * 127.0).round() as u8
    }
    
    /// Vérifie si une trame MIDI est un message pertinent pour OSC
    pub fn is_relevant_for_osc(data: &[u8]) -> bool {
        if data.is_empty() {
            return false;
        }
        
        let status = data[0];
        let status_type = status & 0xF0;
        
        matches!(status_type, 0x80 | 0x90 | 0xB0 | 0xC0 | 0xE0)
    }
    
    /// Extrait le type de message MIDI pour le logging
    pub fn get_midi_message_type(data: &[u8]) -> &'static str {
        if data.is_empty() {
            return "inconnu";
        }
        
        let status = data[0];
        let status_type = status & 0xF0;
        
        match status_type {
            0x80 => "Note Off",
            0x90 => "Note On",
            0xA0 => "Aftertouch",
            0xB0 => "Control Change",
            0xC0 => "Program Change",
            0xD0 => "Channel Pressure",
            0xE0 => "Pitch Bend",
            _ => "System",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_midi_osc_conversions() {
        // Test conversion MIDI vers OSC
        assert_eq!(OscMessageHandler::midi_to_osc(0), 0.0);
        assert_eq!(OscMessageHandler::midi_to_osc(64), 64.0 / 127.0);
        assert_eq!(OscMessageHandler::midi_to_osc(127), 1.0);
        
        // Test conversion OSC vers MIDI
        assert_eq!(OscMessageHandler::osc_to_midi(0.0), 0);
        assert_eq!(OscMessageHandler::osc_to_midi(0.5), 64);
        assert_eq!(OscMessageHandler::osc_to_midi(1.0), 127);
    }
    
    #[test]
    fn test_pitch_bend_conversions() {
        // Test conversion pitch bend vers OSC
        assert_eq!(OscMessageHandler::pitch_bend_to_osc(0), -1.0);
        assert_eq!(OscMessageHandler::pitch_bend_to_osc(8192), 0.0);
        assert_eq!(OscMessageHandler::pitch_bend_to_osc(16383), 1.0);
        
        // Test conversion OSC vers pitch bend
        assert_eq!(OscMessageHandler::osc_to_pitch_bend(-1.0), 0);
        assert_eq!(OscMessageHandler::osc_to_pitch_bend(0.0), 8192);
        assert_eq!(OscMessageHandler::osc_to_pitch_bend(1.0), 16383);
    }
    
    #[test]
    fn test_pan_conversions() {
        // Test conversion pan vers OSC
        assert_eq!(OscMessageHandler::pan_to_osc(0), -1.0);
        assert_eq!(OscMessageHandler::pan_to_osc(63), -1.0 + (63.0 / 127.0 * 2.0));
        assert_eq!(OscMessageHandler::pan_to_osc(127), 1.0);
        
        // Test conversion OSC vers pan
        assert_eq!(OscMessageHandler::osc_to_pan(-1.0), 0);
        assert_eq!(OscMessageHandler::osc_to_pan(0.0), 63);
        assert_eq!(OscMessageHandler::osc_to_pan(1.0), 127);
    }
    
    #[test]
    fn test_midi_relevance() {
        // Messages pertinents pour OSC
        assert!(OscMessageHandler::is_relevant_for_osc(&[0x90, 60, 100])); // Note On
        assert!(OscMessageHandler::is_relevant_for_osc(&[0x80, 60, 0])); // Note Off
        assert!(OscMessageHandler::is_relevant_for_osc(&[0xB0, 64, 127])); // CC
        assert!(OscMessageHandler::is_relevant_for_osc(&[0xC0, 5])); // Program Change
        assert!(OscMessageHandler::is_relevant_for_osc(&[0xE0, 0, 64])); // Pitch Bend
        
        // Messages non pertinents
        assert!(!OscMessageHandler::is_relevant_for_osc(&[])); // Vide
        assert!(!OscMessageHandler::is_relevant_for_osc(&[0xF0])); // System
    }
    
    #[test]
    fn test_midi_message_types() {
        assert_eq!(OscMessageHandler::get_midi_message_type(&[0x90, 60, 100]), "Note On");
        assert_eq!(OscMessageHandler::get_midi_message_type(&[0x80, 60, 0]), "Note Off");
        assert_eq!(OscMessageHandler::get_midi_message_type(&[0xB0, 64, 127]), "Control Change");
        assert_eq!(OscMessageHandler::get_midi_message_type(&[0xC0, 5]), "Program Change");
        assert_eq!(OscMessageHandler::get_midi_message_type(&[0xE0, 0, 64]), "Pitch Bend");
        assert_eq!(OscMessageHandler::get_midi_message_type(&[]), "inconnu");
    }
}
