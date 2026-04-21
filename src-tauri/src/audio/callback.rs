//! Callbacks audio temps réel
//! 
//! Ce module contient toute la logique exécutée dans le callback audio,
//! incluant le traitement MIDI, les plugins VST et la gestion des métriques.

use cpal::{OutputCallbackInfo, Sample, FromSample, SizedSample};
use parking_lot::Mutex;
use rack::prelude::{
    MidiEvent as RackMidiEvent, MidiEventKind as RackMidiEventKind,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use vst::{
    api::{MidiEventFlags, Supported},
    buffer::AudioBuffer,
    host::PluginInstance,
    plugin::CanDo,
};
use crate::audio::plugin::PluginBackend;

/// Paquet MIDI avec timestamp
#[derive(Debug, Clone)]
pub struct MidiPacket {
    /// Données MIDI (max 3 octets pour channel voice)
    pub data: [u8; 3],
    /// Timestamp en millisecondes
    pub timestamp_ms: u64,
}

impl MidiPacket {
    /// Crée un paquet depuis des octets
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        let mut data = [0u8; 3];
        let len = bytes.len().min(3);
        data[..len].copy_from_slice(&bytes[..len]);
        
        Some(Self {
            data,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        })
    }
    
    /// Vérifie si c'est un Note On
    pub fn is_note_on(&self) -> bool {
        self.data.len() >= 3 && (self.data[0] & 0xF0) == 0x90 && self.data[2] > 0
    }
    
    /// Vérifie si c'est un Note Off
    pub fn is_note_off(&self) -> bool {
        self.data.len() >= 3 && ((self.data[0] & 0xF0) == 0x80 || 
            ((self.data[0] & 0xF0) == 0x90 && self.data[2] == 0))
    }
    
    /// Vérifie si c'est un message critique (Note Off)
    pub fn is_critical_release(&self) -> bool {
        self.is_note_off()
    }
}

/// État du callback audio (temps réel)
pub struct AudioCallbackState {
    /// File des messages MIDI en attente
    pending_midi: VecDeque<MidiPacket>,
    /// Récepteur pour les messages MIDI entrants
    midi_rx: rtrb::Consumer<MidiPacket>,
    /// Compteur de messages MIDI dropés
    midi_drop_count: Arc<AtomicU32>,
    /// Compteur de lock misses sur le mutex audio
    audio_lock_miss_count: Arc<AtomicU32>,
    /// Compteur de resets d'urgence
    emergency_reset_count: Arc<AtomicU32>,
    /// Dernière sortie audio (pour replay en cas de lock miss)
    last_output: Vec<f32>,
    /// Flag indiquant qu'un reset d'urgence est nécessaire
    needs_emergency_reset: bool,
    /// Nombre d'entrées du plugin
    plugin_inputs: usize,
    /// Nombre de sorties du plugin  
    plugin_outputs: usize,
    /// Compteur d'erreurs
    error_count: u64,
}

unsafe impl Send for AudioCallbackState {}

impl AudioCallbackState {
    /// Crée un nouvel état de callback
    pub fn new(
        midi_rx: rtrb::Consumer<MidiPacket>,
        plugin_inputs: usize,
        plugin_outputs: usize,
        midi_drop_count: Arc<AtomicU32>,
        audio_lock_miss_count: Arc<AtomicU32>,
        emergency_reset_count: Arc<AtomicU32>,
    ) -> Self {
        Self {
            pending_midi: VecDeque::with_capacity(256),
            midi_rx,
            midi_drop_count,
            audio_lock_miss_count,
            emergency_reset_count,
            last_output: Vec::new(),
            needs_emergency_reset: false,
            plugin_inputs,
            plugin_outputs,
            error_count: 0,
        }
    }
    
    /// Traite les messages MIDI entrants
    pub fn process_incoming_midi(&mut self) {
        while let Ok(packet) = self.midi_rx.pop() {
            if self.pending_midi.len() >= 256 {
                self.drop_oldest_note_on();
                self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            
            if !packet.is_critical_release() {
                self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            
            if self.drop_oldest_non_critical() {
                self.pending_midi.push_back(packet);
                self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            
            self.pending_midi.clear();
            self.pending_midi.push_back(packet);
            self.needs_emergency_reset = true;
            self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    /// Applique les messages MIDI au plugin
    pub fn apply_midi_to_plugin(&mut self, plugin: &mut dyn PluginBackend) {
        while let Some(msg) = self.pending_midi.pop_front() {
            plugin.send_midi(&msg.data[..msg.data.len()]);
        }
    }
    
    /// Gère un lock miss sur le mutex du plugin
    pub fn handle_plugin_lock_miss(&mut self) {
        self.audio_lock_miss_count.fetch_add(1, Ordering::Relaxed);
        self.record_error();
    }
    
    /// Effectue un reset d'urgence
    pub fn emergency_reset(&mut self) {
        self.pending_midi.clear();
        self.needs_emergency_reset = false;
        self.emergency_reset_count.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Vérifie si un reset d'urgence est nécessaire
    pub fn needs_emergency_reset(&self) -> bool {
        self.needs_emergency_reset
    }
    
    /// Met à jour la dernière sortie audio
    pub fn update_last_output(&mut self, output: &[f32]) {
        self.last_output = output.to_vec();
    }
    
    /// Retourne une copie de la dernière sortie
    pub fn last_output(&self) -> &[f32] {
        &self.last_output
    }
    
    // Méthodes privées
    
    /// Supprime le plus ancien Note On
    fn drop_oldest_note_on(&mut self) -> bool {
        let pos = self.pending_midi.iter().position(|m| m.is_note_on());
        if let Some(idx) = pos {
            self.pending_midi.swap_remove_front(idx);
            return true;
        }
        false
    }
    
    /// Supprime le plus ancien message non critique
    fn drop_oldest_non_critical(&mut self) -> bool {
        let pos = self.pending_midi.iter().position(|m| !m.is_critical_release());
        if let Some(idx) = pos {
            self.pending_midi.swap(0, idx);
            self.pending_midi.pop_front();
            return true;
        }
        false
    }
    
    /// Enregistre une erreur
    fn record_error(&mut self) {
        self.error_count = self.error_count.wrapping_add(1);
    }
}

/// Callback audio principal pour f32
pub fn audio_callback_f32<T>(
    data: &mut [T],
    _info: &OutputCallbackInfo,
    mut callback_state: AudioCallbackState,
    gain_bits: Arc<AtomicU32>,
) where
    T: Sample + FromSample<f32> + SizedSample,
{
    // Traiter les messages MIDI entrants
    callback_state.process_incoming_midi();
    
    // Récupérer le gain
    let gain = f32::from_bits(gain_bits.load(Ordering::Relaxed));
    
    // TODO: Appliquer le traitement audio avec plugin
    // Pour l'instant, on génère du silence
    let silence = T::from_sample(0.0);
    
    // Remplir les buffers de sortie
    for sample in data.iter_mut() {
        *sample = silence;
    }
}

/// Callback audio pour i16
pub fn audio_callback_i16<T>(
    data: &mut [T],
    _info: &OutputCallbackInfo,
    callback_state: AudioCallbackState,
    gain_bits: Arc<AtomicU32>,
) where
    T: Sample + FromSample<i16> + SizedSample,
{
    // Implémentation similaire à audio_callback_f32 mais pour i16
    let gain = f32::from_bits(gain_bits.load(Ordering::Relaxed));
    let silence = T::from_sample(0i16);
    
    for sample in data.iter_mut() {
        *sample = silence;
    }
}

/// Callback audio pour u16
pub fn audio_callback_u16<T>(
    data: &mut [T],
    _info: &OutputCallbackInfo,
    callback_state: AudioCallbackState,
    gain_bits: Arc<AtomicU32>,
) where
    T: Sample + FromSample<u16> + SizedSample,
{
    // Implémentation similaire à audio_callback_f32 mais pour u16
    let gain = f32::from_bits(gain_bits.load(Ordering::Relaxed));
    let silence = T::from_sample(0u16);
    
    for sample in data.iter_mut() {
        *sample = silence;
    }
}

/// Convertit les données MIDI vers le format Rack
pub fn midi_to_rack_event(_data: [u8; 3]) -> Option<RackMidiEvent> {
    // TODO: Implémenter la conversion réelle vers Rack MIDI
    // Pour l'instant, retourner None pour éviter les erreurs de structure
    None
}

/// Traite les messages MIDI en attente pour un plugin VST2
pub fn process_pending_vst2_midi(state: &mut AudioCallbackState, _instance: &mut PluginInstance) {
    // TODO: Implémenter le traitement MIDI VST2 réel
    // Pour l'instant, juste consommer les messages
    while let Some(_msg) = state.pending_midi.pop_front() {
        // Les messages sont consommés mais pas traités
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_midi_packet_creation() {
        let packet = MidiPacket::from_bytes(&[0x90, 60, 100]);
        assert!(packet.is_some());
        
        let packet = packet.unwrap();
        assert!(packet.is_note_on());
        assert!(!packet.is_note_off());
        assert!(!packet.is_critical_release());
    }
    
    #[test]
    fn test_midi_packet_note_off() {
        let packet = MidiPacket::from_bytes(&[0x80, 60, 0]);
        assert!(packet.is_some());
        
        let packet = packet.unwrap();
        assert!(!packet.is_note_on());
        assert!(packet.is_note_off());
        assert!(packet.is_critical_release());
    }
    
    #[test]
    fn test_midi_to_rack_event() {
        let rack_event = midi_to_rack_event([0x90, 60, 100]);
        assert!(rack_event.is_some());
        
        let event = rack_event.unwrap();
        assert_eq!(event.channel, 0);
        assert_eq!(event.note, 60);
        assert_eq!(event.velocity, 100);
    }
}
