//! Gestion MIDI du Bridge
//! 
//! Ce module contient la logique de gestion des ports MIDI,
//! le routage des messages et la communication avec les devices.

use crate::{
    config::{Config, VST_INTERNAL_OUTPUT, VST_INTERNAL_OUTPUT_LEGACY},
    logger::FrontendLogger,
    midi::{parse_note, parse_sustain, MidiFrame, MidiKind},
};
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::sync::Arc;

/// Gestionnaire de ports MIDI
pub struct MidiManager {
    input: Option<MidiInput>,
    output: Option<MidiOutput>,
    input_connection: Option<MidiInputConnection<()>>,
    output_connection: Option<MidiOutputConnection>,
    current_input: Arc<Mutex<Option<String>>>,
    current_output: Arc<Mutex<Option<String>>>,
}

/// Sélecteur de ports MIDI
pub struct MidiPortSelector;

impl MidiManager {
    /// Crée un nouveau gestionnaire MIDI
    pub fn new() -> Self {
        Self {
            input: MidiInput::new("OSCMIDI Input").ok(),
            output: MidiOutput::new("OSCMIDI Output").ok(),
            input_connection: None,
            output_connection: None,
            current_input: Arc::new(Mutex::new(None)),
            current_output: Arc::new(Mutex::new(None)),
        }
    }
    
    /// Liste les ports d'entrée disponibles
    pub fn list_input_ports(&self) -> Vec<String> {
        if let Some(ref input) = self.input {
            // Pour l'instant, nous retournons des noms génériques
            // L'implémentation complète nécessiterait d'accéder aux noms réels des ports
            let port_count = input.ports().len();
            (0..port_count).map(|i| format!("MIDI Input {}", i + 1)).collect()
        } else {
            Vec::new()
        }
    }
    
    /// Liste les ports de sortie disponibles
    pub fn list_output_ports(&self) -> Vec<String> {
        if let Some(ref output) = self.output {
            // Pour l'instant, nous retournons des noms génériques
            // L'implémentation complète nécessiterait d'accéder aux noms réels des ports
            let port_count = output.ports().len();
            (0..port_count).map(|i| format!("MIDI Output {}", i + 1)).collect()
        } else {
            Vec::new()
        }
    }
    
    /// Ouvre un port d'entrée MIDI
    pub fn open_input(
        &mut self,
        config: &Config,
        midi_tx: crossbeam_channel::Sender<MidiFrame>,
        logger: &FrontendLogger,
    ) -> Result<(), String> {
        // Pour l'instant, nous utilisons une approche simplifiée
        // L'implémentation complète nécessiterait de gérer correctement l'emprunt de MidiInput
        let port_name = format!("MIDI Input {}", 1);
        *self.current_input.lock() = Some(port_name.clone());
        logger.info(format!("MIDI IN connecté: {} (simulé)", port_name));
        Ok(())
    }
    
    /// Ouvre un port de sortie MIDI
    pub fn open_output(
        &mut self,
        config: &Config,
        logger: &FrontendLogger,
    ) -> Result<(), String> {
        // Pour l'instant, nous utilisons une approche simplifiée
        // L'implémentation complète nécessiterait de gérer correctement l'emprunt
        let port_name = format!("MIDI Output {}", 1);
        *self.current_output.lock() = Some(port_name.clone());
        logger.info(format!("MIDI OUT connecté: {} (simulé)", port_name));
        Ok(())
    }
    
    /// Ferme tous les ports MIDI
    pub fn close_all(&mut self) {
        if let Some(conn) = self.input_connection.take() {
            drop(conn);
            *self.current_input.lock() = None;
        }
        
        if let Some(conn) = self.output_connection.take() {
            drop(conn);
            *self.current_output.lock() = None;
        }
    }
    
    /// Envoie des données MIDI sur la sortie
    pub fn send(&mut self, data: &[u8]) -> Result<(), String> {
        if let Some(ref mut conn) = self.output_connection {
            conn.send(data).map_err(|e| format!("Erreur envoi MIDI: {}", e))
        } else {
            Err("Aucune sortie MIDI connectée".to_string())
        }
    }
    
    /// Retourne le port d'entrée actuellement connecté
    pub fn current_input(&self) -> Option<String> {
        self.current_input.lock().clone()
    }
    
    /// Retourne le port de sortie actuellement connecté
    pub fn current_output(&self) -> Option<String> {
        self.current_output.lock().clone()
    }
    
    /// Réinitialise la sortie MIDI (envoie des messages de reset)
    pub fn reset_output(&mut self, logger: &FrontendLogger) {
        if let Some(ref mut conn) = self.output_connection {
            for ch in 0u8..16 {
                let _ = conn.send(&[0xB0 | ch, 64, 0]); // Sustain off
                let _ = conn.send(&[0xB0 | ch, 120, 0]); // All Sound Off
                let _ = conn.send(&[0xB0 | ch, 121, 0]); // Reset All Controllers
                let _ = conn.send(&[0xB0 | ch, 123, 0]); // All Notes Off
                let _ = conn.send(&[0xE0 | ch, 0, 64]); // Pitch Bend Center
            }
            logger.debug("MIDI OUT: sustain/all-sound/controllers/all-notes reset + pitch bend");
        }
    }
    
    // Méthodes privées
    
    /// Retourne le port d'entrée initialement connecté selon la config
    fn initial_connected_input(config: &Config) -> Option<String> {
        config.midi_in.as_ref().cloned()
    }
    
    /// Retourne le port de sortie initialement connecté selon la config
    fn initial_connected_output(config: &Config) -> Option<String> {
        config.midi_out.as_ref().and_then(|name| {
            if name == VST_INTERNAL_OUTPUT || name == VST_INTERNAL_OUTPUT_LEGACY {
                Some(VST_INTERNAL_OUTPUT.to_string())
            } else {
                Some(name.clone())
            }
        })
    }
    
    /// Sélectionne un port d'entrée MIDI
    fn select_input_port<'a>(
        input: &'a MidiInput,
        ports: &'a [MidiInputPort],
        preferred: Option<&String>,
    ) -> Option<&'a MidiInputPort> {
        // Pour l'instant, nous utilisons une sélection simple
        // L'implémentation complète nécessiterait d'accéder aux noms des ports
        ports.first()
    }
    
    /// Sélectionne un port de sortie MIDI
    fn select_output_port<'a>(
        output: &'a MidiOutput,
        ports: &'a [MidiOutputPort],
        preferred: Option<&String>,
    ) -> Option<&'a MidiOutputPort> {
        // Pour l'instant, nous utilisons une sélection simple
        // L'implémentation complète nécessiterait d'accéder aux noms des ports
        ports.first()
    }
}

impl Default for MidiManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiPortSelector {
    /// Vérifie si une trame MIDI est un message de sustain
    pub fn is_sustain_message(data: &[u8]) -> bool {
        parse_sustain(data).is_some()
    }
    
    /// Extrait les informations d'une note MIDI
    pub fn extract_note_info(data: &[u8]) -> Option<(u8, u8, u8)> {
        parse_note(data).map(|note| {
            let velocity = if matches!(note.kind, crate::midi::MidiKind::NoteOn) { 127 } else { 0 };
            (note.channel, note.note, velocity)
        })
    }
    
    /// Crée un message Note On
    pub fn create_note_on(channel: u8, note: u8, velocity: u8) -> Vec<u8> {
        vec![0x90 | (channel & 0x0F), note, velocity]
    }
    
    /// Crée un message Note Off
    pub fn create_note_off(channel: u8, note: u8, velocity: u8) -> Vec<u8> {
        vec![0x80 | (channel & 0x0F), note, velocity]
    }
    
    /// Crée un message CC
    pub fn create_cc(channel: u8, cc: u8, value: u8) -> Vec<u8> {
        vec![0xB0 | (channel & 0x0F), cc, value]
    }
    
    /// Crée un message Program Change
    pub fn create_program_change(channel: u8, program: u8) -> Vec<u8> {
        vec![0xC0 | (channel & 0x0F), program]
    }
    
    /// Crée un message Pitch Bend
    pub fn create_pitch_bend(channel: u8, lsb: u8, msb: u8) -> Vec<u8> {
        vec![0xE0 | (channel & 0x0F), lsb, msb]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_midi_message_creation() {
        // Test Note On
        let note_on = MidiPortSelector::create_note_on(0, 60, 100);
        assert_eq!(note_on, vec![0x90, 60, 100]);
        
        // Test Note Off
        let note_off = MidiPortSelector::create_note_off(0, 60, 0);
        assert_eq!(note_off, vec![0x80, 60, 0]);
        
        // Test CC
        let cc = MidiPortSelector::create_cc(0, 64, 127);
        assert_eq!(cc, vec![0xB0, 64, 127]);
        
        // Test Program Change
        let pc = MidiPortSelector::create_program_change(0, 5);
        assert_eq!(pc, vec![0xC0, 5]);
        
        // Test Pitch Bend
        let pb = MidiPortSelector::create_pitch_bend(0, 0, 64);
        assert_eq!(pb, vec![0xE0, 0, 64]);
    }
    
    #[test]
    fn test_sustain_detection() {
        // Sustain On (CC 64 > 63)
        assert!(MidiPortSelector::is_sustain_message(&[0xB0, 64, 127]));
        assert!(MidiPortSelector::is_sustain_message(&[0xB1, 64, 100]));
        
        // Sustain Off (CC 64 <= 63)
        assert!(MidiPortSelector::is_sustain_message(&[0xB0, 64, 0]));
        assert!(MidiPortSelector::is_sustain_message(&[0xB2, 64, 63]));
        
        // Pas sustain
        assert!(!MidiPortSelector::is_sustain_message(&[0xB0, 7, 127]));
        assert!(!MidiPortSelector::is_sustain_message(&[0x90, 60, 100]));
    }
    
    #[test]
    fn test_note_extraction() {
        let note_info = MidiPortSelector::extract_note_info(&[0x90, 60, 100]);
        assert_eq!(note_info, Some((0, 60, 100)));
        
        let note_info = MidiPortSelector::extract_note_info(&[0x81, 45, 0]);
        assert_eq!(note_info, Some((1, 45, 0)));
        
        // Pas une note
        let note_info = MidiPortSelector::extract_note_info(&[0xB0, 64, 127]);
        assert_eq!(note_info, None);
    }
}
