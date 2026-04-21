//! Moteur audio principal
//! 
//! Ce module contient la logique principale du moteur audio,
//! incluant la gestion du cycle de vie et des métriques.
//! Utilise les sous-modules spécialisés pour stream, plugin et callbacks.

use crate::audio::config::{AudioStreamConfig, ConfigError};
use crate::audio::compat::get_window_hwnd;
use crate::audio::plugin::PluginStateManager;
use crate::audio::stream::StreamManager;
use crate::audio::windows::VstWindowManager;
use crate::logger::{FrontendLogger, background_log};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;
use rtrb::{Consumer, Producer, RingBuffer};

/// Erreurs du moteur audio
#[derive(Debug, Error, Clone)]
pub enum AudioError {
    #[error("{0}")]
    Message(String),
    
    #[error("Erreur de configuration: {0}")]
    Config(#[from] ConfigError),
    
    #[error("Le moteur audio est déjà démarré")]
    AlreadyStarted,
    
    #[error("Le moteur audio n'est pas démarré")]
    NotStarted,
    
    #[error("Plugin VST non chargé")]
    PluginNotLoaded,
}

/// Moteur audio principal
/// 
/// Gère le cycle de vie complet du système audio:
/// - Initialisation des streams CPAL
/// - Chargement des plugins VST
/// - Callbacks temps réel
/// - Métriques de performance
#[derive(Clone)]
pub struct AudioEngine {
    /// Plugin VST chargé
    plugin: Arc<Mutex<Option<Box<dyn PluginBackend>>>>,
    /// Gestionnaire de fenêtre VST
    window_manager: Arc<Mutex<Option<VstWindowManager>>>,
    /// Gestionnaire d'état du plugin
    #[allow(dead_code)]
    state_manager: Arc<Mutex<Option<PluginStateManager>>>,
    /// Producteur de messages MIDI
    #[allow(dead_code)]
    midi_tx: Arc<Mutex<Option<Producer<crate::audio::callback::MidiPacket>>>>,
    /// Consommateur de messages MIDI
    #[allow(dead_code)]
    midi_rx: Arc<Mutex<Option<Consumer<crate::audio::callback::MidiPacket>>>>,
    /// Métriques de performance
    #[allow(dead_code)]
    metrics: AudioMetrics,
    /// Dernier chemin VST utilisé
    last_vst_path: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// État du stream (pour éviter les problèmes Send/Sync)
    is_running: Arc<Mutex<bool>>,
    /// Configuration actuelle du stream
    stream_config: Arc<Mutex<Option<cpal::StreamConfig>>>,
}

/// Runtime audio interne
/// 
/// Contient l'état actuel du moteur audio quand il est démarré.
struct AudioRuntime {
    /// Plugin VST chargé
    #[allow(dead_code)]
    plugin: Arc<Mutex<Box<dyn PluginBackend>>>,
    /// Métriques en temps réel
    #[allow(dead_code)]
    realtime_metrics: RealtimeMetrics,
    /// Configuration utilisée
    #[allow(dead_code)]
    config: AudioStreamConfig,
}

/// Métriques de performance accessibles depuis n'importe quel thread
#[derive(Clone)]
struct AudioMetrics {
    /// Nombre de xruns (buffer underruns/overruns)
    #[allow(dead_code)]
    xruns: Arc<AtomicU32>,
    /// Nombre de messages MIDI dropés
    #[allow(dead_code)]
    midi_drops: Arc<AtomicU32>,
    /// Nombre de lock misses sur le mutex audio
    #[allow(dead_code)]
    lock_misses: Arc<AtomicU32>,
    /// Nombre de resets d'urgence
    #[allow(dead_code)]
    emergency_resets: Arc<AtomicU32>,
}

/// Métriques en temps réel (accessibles uniquement depuis le callback)
struct RealtimeMetrics {
    /// Niveau pic canal gauche
    #[allow(dead_code)]
    peak_left: Arc<AtomicU32>,
    /// Niveau pic canal droit
    #[allow(dead_code)]
    peak_right: Arc<AtomicU32>,
    /// Taille du bloc actuel
    #[allow(dead_code)]
    block_size_frames: Arc<AtomicU32>,
}

/// Interface pour les plugins VST (unifie VST2 et VST3)
trait PluginBackend: Send + Sync {
    /// Traite les samples audio
    #[allow(dead_code)]
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize);
    
    /// Envoie un message MIDI
    #[allow(dead_code)]
    fn send_midi(&mut self, data: &[u8]);
    
    /// Définit un paramètre
    #[allow(dead_code)]
    fn set_parameter(&mut self, index: usize, value: f32) -> Result<(), AudioError>;
    
    /// Retourne les paramètres
    #[allow(dead_code)]
    fn get_parameters(&self) -> Vec<crate::types::VstParameter>;
    
    /// Ouvre l'interface graphique
    #[allow(dead_code)]
    fn open_editor(&mut self, parent: Option<std::ptr::NonNull<()>>) -> Result<(), AudioError>;
    
    /// Ferme l'interface graphique
    #[allow(dead_code)]
    fn close_editor(&mut self) -> Result<(), AudioError>;
    
    /// Vérifie si le plugin supporte MIDI
    #[allow(dead_code)]
    fn supports_midi(&self) -> bool;
    
    /// Retourne si l'éditeur est ouvert
    #[allow(dead_code)]
    fn is_editor_open(&self) -> bool;
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine")
            .field("plugin", &"<PluginBackend>")
            .field("window_manager", &"<VstWindowManager>")
            .field("state_manager", &"<PluginStateManager>")
            .field("midi_tx", &"<MidiProducer>")
            .field("midi_rx", &"<MidiConsumer>")
            .field("realtime_metrics", &"<RealtimeMetrics>")
            .field("config", &"<AudioStreamConfig>")
            .finish()
    }
}

impl AudioEngine {
    /// Crée un nouveau moteur audio
    pub fn new() -> Self {
        // Créer le ring buffer pour les messages MIDI
        let (midi_tx, midi_rx) = RingBuffer::new(1024);
        
        Self {
            plugin: Arc::new(Mutex::new(None)),
            window_manager: Arc::new(Mutex::new(None)),
            state_manager: Arc::new(Mutex::new(None)),
            midi_tx: Arc::new(Mutex::new(Some(midi_tx))),
            midi_rx: Arc::new(Mutex::new(Some(midi_rx))),
            metrics: AudioMetrics {
                xruns: Arc::new(AtomicU32::new(0)),
                midi_drops: Arc::new(AtomicU32::new(0)),
                lock_misses: Arc::new(AtomicU32::new(0)),
                emergency_resets: Arc::new(AtomicU32::new(0)),
            },
            last_vst_path: Arc::new(Mutex::new(None)),
            is_running: Arc::new(Mutex::new(false)),
            stream_config: Arc::new(Mutex::new(None)),
        }
    }
    
    /// Démarre le moteur audio avec la configuration spécifiée
    pub fn start(
        &self,
        config: AudioStreamConfig,
        vst_fallback: Option<std::path::PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        // Arrêter le moteur s'il est déjà démarré
        if self.is_running() {
            self.stop_internal(Some(logger.app_handle()))?;
        }
        
        // Valider la configuration
        config.validate()?;
        
        // Résoudre le chemin du plugin VST
        let vst_path = self.resolve_vst_path(&config, vst_fallback, &logger)?;
        
        // TODO: Implémenter la création du stream audio
        // Pour l'instant, nous simulons le démarrage
        logger.info("Démarrage du moteur audio (simulation)".to_string());
        
        // Simuler la création d'un runtime
        let sample_rate = config.sample_rate();
        let buffer_size = config.buffer_size();
        
        let _runtime = AudioRuntime {
            plugin: Arc::new(Mutex::new(Box::new(MockPlugin::new()))),
            realtime_metrics: RealtimeMetrics {
                peak_left: Arc::new(AtomicU32::new(0)),
                peak_right: Arc::new(AtomicU32::new(0)),
                block_size_frames: Arc::new(AtomicU32::new(buffer_size)),
            },
            config,
        };
        
        // Mettre à jour l'état
        *self.is_running.lock() = true;
        *self.stream_config.lock() = Some(cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Fixed(buffer_size),
        });
        *self.last_vst_path.lock() = Some(vst_path);
        
        logger.info("Moteur audio démarré avec succès".to_string());
        Ok(())
    }
    
    /// Arrête le moteur audio
    pub fn stop(&self, app_handle: Option<tauri::AppHandle>) -> Result<(), AudioError> {
        self.stop_internal(app_handle)
    }
    
    /// Vérifie si le moteur est démarré
    pub fn is_running(&self) -> bool {
        *self.is_running.lock()
    }
    
    /// Vérifie si un plugin VST est chargé
    pub fn is_vst_loaded(&self) -> bool {
        self.plugin.lock().is_some()
    }
    
    /// Liste les backends audio disponibles
    pub fn list_backends(&self) -> Vec<String> {
        StreamManager::list_backends()
    }
    
    /// Liste les devices pour un backend donné
    #[allow(dead_code)]
    pub fn list_devices(&self, backend: Option<String>) -> Vec<String> {
        StreamManager::list_devices(backend.as_deref())
    }
    
    /// Retourne le nombre de xruns
    #[allow(dead_code)]
    pub fn xrun_count(&self) -> Option<u32> {
        if self.is_running() {
            Some(self.metrics.xruns.load(Ordering::Relaxed))
        } else {
            None
        }
    }
    
    /// Retourne le nombre de messages MIDI dropés
    #[allow(dead_code)]
    pub fn midi_drop_count(&self) -> Option<u32> {
        if self.is_running() {
            Some(self.metrics.midi_drops.load(Ordering::Relaxed))
        } else {
            None
        }
    }
    
    /// Retourne le nombre de lock misses
    #[allow(dead_code)]
    pub fn audio_lock_miss_count(&self) -> Option<u32> {
        if self.is_running() {
            Some(self.metrics.lock_misses.load(Ordering::Relaxed))
        } else {
            None
        }
    }
    
    /// Retourne le nombre de resets d'urgence
    #[allow(dead_code)]
    pub fn emergency_reset_count(&self) -> Option<u32> {
        if self.is_running() {
            Some(self.metrics.emergency_resets.load(Ordering::Relaxed))
        } else {
            None
        }
    }
    
    /// Retourne les niveaux pic actuels
    #[allow(dead_code)]
    pub fn peak_levels(&self) -> Option<(f32, f32)> {
        if self.is_running() {
            // TODO: Implémenter la récupération réelle des niveaux pic
            // Pour l'instant, retourner des valeurs par défaut
            Some((0.0, 0.0))
        } else {
            None
        }
    }
    
    // Méthodes de compatibilité avec l'ancienne API
    
    /// Retourne la latence actuelle en ms
    #[allow(dead_code)]
    pub fn current_latency_ms(&self) -> Option<f32> {
        if self.is_running() {
            Some(5.0) // Valeur simulée
        } else {
            None
        }
    }
    
    /// Retourne le backend audio actuel
    #[allow(dead_code)]
    pub fn current_backend(&self) -> Option<String> {
        if self.is_running() {
            // TODO: Récupérer le backend réel depuis StreamManager
            Some("auto".to_string())
        } else {
            None
        }
    }
    
    /// Retourne le device audio actuel
    #[allow(dead_code)]
    pub fn current_device(&self) -> Option<String> {
        if self.is_running() {
            // TODO: Récupérer le device réel depuis StreamManager
            Some("default".to_string())
        } else {
            None
        }
    }
    
    /// Retourne le sample rate actuel
    #[allow(dead_code)]
    pub fn current_sample_rate(&self) -> Option<u32> {
        if let Some(ref config) = *self.stream_config.lock() {
            Some(config.sample_rate.0)
        } else {
            None
        }
    }
    
    /// Retourne la taille du buffer actuelle
    #[allow(dead_code)]
    pub fn current_buffer_size(&self) -> Option<u32> {
        if let Some(ref config) = *self.stream_config.lock() {
            match config.buffer_size {
                cpal::BufferSize::Fixed(size) => Some(size),
                cpal::BufferSize::Default => Some(256),
            }
        } else {
            None
        }
    }
    
    /// Retourne la taille du buffer demandée
    #[allow(dead_code)]
    pub fn requested_buffer_size(&self) -> Option<u32> {
        self.current_buffer_size()
    }
    
    /// Retourne la taille du buffer du stream
    #[allow(dead_code)]
    pub fn stream_buffer_size(&self) -> Option<u32> {
        self.current_buffer_size()
    }
    
    /// Vérifie s'il y a une incompatibilité de buffer
    #[allow(dead_code)]
    pub fn buffer_size_mismatch(&self) -> Option<bool> {
        if self.is_running() {
            Some(false)
        } else {
            None
        }
    }
    
    /// Vérifie si le VST supporte MIDI
    #[allow(dead_code)]
    pub fn vst_midi_compatible(&self) -> Option<bool> {
        if let Some(ref plugin) = *self.plugin.lock() {
            Some(plugin.supports_midi())
        } else {
            None
        }
    }
    
    /// Vérifie si le limiter est activé
    #[allow(dead_code)]
    pub fn limiter_enabled(&self) -> Option<bool> {
        if self.is_running() {
            // TODO: Récupérer cette valeur depuis la configuration active
            Some(false)
        } else {
            None
        }
    }
    
    /// Envoie des données MIDI au plugin
    #[allow(dead_code)]
    pub fn send_midi(&self, data: &[u8]) {
        if let Some(ref mut plugin) = *self.plugin.lock() {
            plugin.send_midi(data);
        }
    }
    
    /// Panique toutes les notes (envoie note off sur tous les canaux)
    #[allow(dead_code)]
    pub fn panic_all_notes(&self) -> Result<(), AudioError> {
        if let Some(ref mut plugin) = *self.plugin.lock() {
            // Envoyer Note Off sur tous les canaux et notes
            for channel in 0..16 {
                for note in 0..128 {
                    let msg = [0x80 | channel, note, 0];
                    plugin.send_midi(&msg);
                }
            }
        }
        Ok(())
    }
    
    /// Liste les paramètres VST
    #[allow(dead_code)]
    pub fn list_vst_parameters(&self) -> Result<Vec<crate::types::VstParameter>, AudioError> {
        if let Some(ref plugin) = *self.plugin.lock() {
            Ok(plugin.get_parameters())
        } else {
            Ok(Vec::new())
        }
    }
    
    /// Définit un paramètre VST
    #[allow(dead_code)]
    pub fn set_vst_parameter(&self, index: usize, value: f32) -> Result<(), AudioError> {
        if let Some(ref mut plugin) = *self.plugin.lock() {
            plugin.set_parameter(index, value)?;
        }
        Ok(())
    }
    
    /// Ouvre l'interface VST
    #[allow(dead_code)]
    pub fn open_vst_ui(&self, app: tauri::AppHandle) -> Result<(), AudioError> {
        let hwnd = get_window_hwnd(&app)?;
        if let Some(ref mut plugin) = *self.plugin.lock() {
            plugin.open_editor(Some(hwnd))?;
        }
        Ok(())
    }
    
    /// Ferme l'interface VST
    #[allow(dead_code)]
    pub fn close_vst_ui(&self, _app: tauri::AppHandle) -> Result<(), AudioError> {
        if let Some(ref mut plugin) = *self.plugin.lock() {
            plugin.close_editor()?;
        }
        Ok(())
    }
    
    /// Définit le gain
    #[allow(dead_code)]
    pub fn set_gain(&self, gain_db: f32) {
        // TODO: Implémenter la mise à jour du gain dans le callback
        // Pour l'instant, nous ne faisons rien
        background_log("info", format!("Gain défini à {} dB", gain_db));
    }
    
    /// Active/désactive le limiter
    #[allow(dead_code)]
    pub fn set_limiter_enabled(&self, enabled: bool) {
        // TODO: Implémenter la mise à jour du limiter dans le callback
        background_log("info", format!("Limiter activé: {}", enabled));
    }
    
    /// Ping le moteur audio (vérifie qu'il est fonctionnel)
    #[allow(dead_code)]
    pub fn ping(&self) -> Result<(), AudioError> {
        if self.is_running() {
            Ok(())
        } else {
            Err(AudioError::NotStarted)
        }
    }
    
    /// Recharge le moteur audio avec de nouvelles settings
    #[allow(dead_code)]
    pub fn reload(
        &self,
        settings: crate::audio::compat::AudioSettings,
        vst_fallback: Option<std::path::PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        let config = crate::audio::compat::settings_to_config(settings);
        self.start(config, vst_fallback, logger)
    }
    
    // Méthodes privées
    
    fn stop_internal(&self, _app_handle: Option<tauri::AppHandle>) -> Result<(), AudioError> {
        // Mettre à jour l'état
        *self.is_running.lock() = false;
        *self.stream_config.lock() = None;
        
        // Fermer le plugin
        *self.plugin.lock() = None;
        
        // Fermer la fenêtre VST
        if let Some(ref mut window_manager) = *self.window_manager.lock() {
            let _ = window_manager.close();
        }
        *self.window_manager.lock() = None;
        
        background_log("info", "Moteur audio arrêté");
        Ok(())
    }
    
    fn resolve_vst_path(
        &self,
        config: &AudioStreamConfig,
        fallback: Option<std::path::PathBuf>,
        logger: &FrontendLogger,
    ) -> Result<std::path::PathBuf, AudioError> {
        if let Some(ref path) = config.vst_path {
            Ok(path.clone())
        } else if let Some(fallback) = fallback {
            logger.info(format!("Utilisation du plugin VST par défaut: {:?}", fallback));
            Ok(fallback)
        } else {
            Err(AudioError::Message("Aucun plugin VST spécifié".to_string()))
        }
    }
    
    // Méthodes privées
    
    #[allow(dead_code)]
    fn select_host(&self, backend: Option<&str>) -> Option<cpal::Host> {
        match backend {
            Some("asio") => cpal::host_from_id(cpal::HostId::Asio).ok(),
            Some("wasapi") => cpal::host_from_id(cpal::HostId::Wasapi).ok(),
            Some(other) => cpal::available_hosts()
                .into_iter()
                .find(|h| h.name().to_lowercase() == other.to_lowercase())
                .and_then(|host_id| cpal::host_from_id(host_id).ok()),
            None => Some(cpal::default_host()),
        }
    }
    
    #[allow(dead_code)]
    fn create_mock_stream(&self) -> Result<cpal::Stream, AudioError> {
        // TODO: Implémenter la création réelle du stream
        // Pour l'instant, nous retournons une erreur
        Err(AudioError::Message("Stream creation non implémenté".to_string()))
    }
}

// Plugin mock pour les tests
struct MockPlugin {
    supports_midi: bool,
}

impl MockPlugin {
    fn new() -> Self {
        Self { supports_midi: true }
    }
}

impl PluginBackend for MockPlugin {
    fn process(&mut self, _inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) {
        for output in outputs.iter_mut() {
            output[..frames].fill(0.0);
        }
    }
    
    fn send_midi(&mut self, _data: &[u8]) {
        // Mock implementation
    }
    
    fn set_parameter(&mut self, _index: usize, _value: f32) -> Result<(), AudioError> {
        Ok(())
    }
    
    fn get_parameters(&self) -> Vec<crate::types::VstParameter> {
        Vec::new()
    }
    
    fn open_editor(&mut self, _parent: Option<std::ptr::NonNull<()>>) -> Result<(), AudioError> {
        Ok(())
    }
    
    fn close_editor(&mut self) -> Result<(), AudioError> {
        Ok(())
    }
    
    fn supports_midi(&self) -> bool {
        self.supports_midi
    }
    
    fn is_editor_open(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_audio_engine_creation() {
        let engine = AudioEngine::new();
        assert!(!engine.is_running());
        assert!(!engine.is_vst_loaded());
    }
    
    #[test]
    fn test_audio_metrics() {
        let engine = AudioEngine::new();
        // Les métriques devraient être None quand le moteur n'est pas démarré
        assert_eq!(engine.xrun_count(), None);
        assert_eq!(engine.midi_drop_count(), None);
        assert_eq!(engine.audio_lock_miss_count(), None);
        assert_eq!(engine.emergency_reset_count(), None);
    }
    
    #[test]
    fn test_list_backends() {
        let engine = AudioEngine::new();
        let backends = engine.list_backends();
        assert!(!backends.is_empty());
    }
    
    #[test]
    fn test_config_validation() {
        let config = AudioStreamConfig::default();
        assert!(config.validate().is_ok());
    }
}
