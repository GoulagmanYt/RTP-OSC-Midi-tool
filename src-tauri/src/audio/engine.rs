//! Moteur audio principal
//! 
//! Ce module contient la logique principale du moteur audio,
//! incluant la gestion du cycle de vie et des métriques.

use cpal::traits::{DeviceTrait, HostTrait};
use crate::audio::config::{AudioStreamConfig, ConfigError};
use crate::audio::compat::get_window_hwnd;
use crate::logger::{FrontendLogger, background_log};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

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
    /// Runtime audio (Option pour permettre le démarrage/arrêt)
    runtime: Arc<Mutex<Option<AudioRuntime>>>,
    /// Dernier chemin VST utilisé (pour le rechargement)
    last_vst: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// État des métriques de performance
    metrics: AudioMetrics,
}

/// Runtime audio interne
/// 
/// Contient l'état actuel du moteur audio quand il est démarré.
struct AudioRuntime {
    /// Plugin VST chargé
    plugin: Arc<Mutex<Box<dyn PluginBackend>>>,
    /// Métriques en temps réel
    realtime_metrics: RealtimeMetrics,
    /// Configuration utilisée
    config: AudioStreamConfig,
}

/// Métriques de performance accessibles depuis n'importe quel thread
#[derive(Clone)]
struct AudioMetrics {
    /// Nombre de xruns (buffer underruns/overruns)
    xruns: Arc<AtomicU32>,
    /// Nombre de messages MIDI dropés
    midi_drops: Arc<AtomicU32>,
    /// Nombre de lock misses sur le mutex audio
    lock_misses: Arc<AtomicU32>,
    /// Nombre de resets d'urgence
    emergency_resets: Arc<AtomicU32>,
}

/// Métriques en temps réel (accessibles uniquement depuis le callback)
struct RealtimeMetrics {
    /// Niveau pic canal gauche
    peak_left: Arc<AtomicU32>,
    /// Niveau pic canal droit
    peak_right: Arc<AtomicU32>,
    /// Taille du bloc actuel
    block_size_frames: Arc<AtomicU32>,
}

/// Interface pour les plugins VST (unifie VST2 et VST3)
trait PluginBackend: Send + Sync {
    /// Traite les samples audio
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize);
    
    /// Envoie un message MIDI
    fn send_midi(&mut self, data: &[u8]);
    
    /// Définit un paramètre
    fn set_parameter(&mut self, index: usize, value: f32) -> Result<(), AudioError>;
    
    /// Retourne les paramètres
    fn get_parameters(&self) -> Vec<crate::types::VstParameter>;
    
    /// Ouvre l'interface graphique
    fn open_editor(&mut self, parent: Option<std::ptr::NonNull<()>>) -> Result<(), AudioError>;
    
    /// Ferme l'interface graphique
    fn close_editor(&mut self) -> Result<(), AudioError>;
    
    /// Vérifie si le plugin supporte MIDI
    fn supports_midi(&self) -> bool;
}

impl AudioEngine {
    /// Crée un nouveau moteur audio
    pub fn new() -> Self {
        Self {
            runtime: Arc::new(Mutex::new(None)),
            last_vst: Arc::new(Mutex::new(None)),
            metrics: AudioMetrics {
                xruns: Arc::new(AtomicU32::new(0)),
                midi_drops: Arc::new(AtomicU32::new(0)),
                lock_misses: Arc::new(AtomicU32::new(0)),
                emergency_resets: Arc::new(AtomicU32::new(0)),
            },
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
        let runtime = AudioRuntime {
            plugin: Arc::new(Mutex::new(Box::new(MockPlugin::new()))),
            realtime_metrics: RealtimeMetrics {
                peak_left: Arc::new(AtomicU32::new(0)),
                peak_right: Arc::new(AtomicU32::new(0)),
                block_size_frames: Arc::new(AtomicU32::new(config.buffer_size())),
            },
            config,
        };
        
        // Stocker le runtime
        *self.runtime.lock() = Some(runtime);
        *self.last_vst.lock() = Some(vst_path);
        
        logger.info("Moteur audio démarré avec succès".to_string());
        Ok(())
    }
    
    /// Arrête le moteur audio
    pub fn stop(&self, app_handle: Option<tauri::AppHandle>) -> Result<(), AudioError> {
        self.stop_internal(app_handle)
    }
    
    /// Vérifie si le moteur est démarré
    pub fn is_running(&self) -> bool {
        self.runtime.lock().is_some()
    }
    
    /// Vérifie si un plugin VST est chargé
    pub fn is_vst_loaded(&self) -> bool {
        self.runtime.lock().is_some()
    }
    
    /// Liste les backends audio disponibles
    pub fn list_backends(&self) -> Vec<String> {
        cpal::available_hosts()
            .into_iter()
            .map(|h| h.name().to_string())
            .collect()
    }
    
    /// Liste les devices pour un backend donné
    pub fn list_devices(&self, backend: Option<String>) -> Vec<String> {
        let host = self.select_host(backend.as_deref());
        match host {
            Some(h) => match h.output_devices() {
                Ok(devices) => devices.filter_map(|d| d.name().ok()).collect(),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        }
    }
    
    /// Retourne le nombre de xruns
    pub fn xrun_count(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|r| r.realtime_metrics.block_size_frames.load(Ordering::Relaxed))
    }
    
    /// Retourne le nombre de messages MIDI dropés
    pub fn midi_drop_count(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|_| self.metrics.midi_drops.load(Ordering::Relaxed))
    }
    
    /// Retourne le nombre de lock misses
    pub fn audio_lock_miss_count(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|_| self.metrics.lock_misses.load(Ordering::Relaxed))
    }
    
    /// Retourne le nombre de resets d'urgence
    pub fn emergency_reset_count(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|_| self.metrics.emergency_resets.load(Ordering::Relaxed))
    }
    
    /// Retourne les niveaux pic actuels
    pub fn peak_levels(&self) -> Option<(f32, f32)> {
        self.runtime.lock().as_ref().map(|r| {
            let left = f32::from_bits(r.realtime_metrics.peak_left.load(Ordering::Relaxed));
            let right = f32::from_bits(r.realtime_metrics.peak_right.load(Ordering::Relaxed));
            (left, right)
        })
    }
    
    // Méthodes de compatibilité avec l'ancienne API
    
    /// Retourne la latence actuelle en ms
    pub fn current_latency_ms(&self) -> Option<f32> {
        self.runtime.lock().as_ref().map(|_| 5.0) // Valeur simulée
    }
    
    /// Retourne le backend audio actuel
    pub fn current_backend(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.config.backend.clone().unwrap_or("auto".to_string()))
    }
    
    /// Retourne le device audio actuel
    pub fn current_device(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.config.device.clone().unwrap_or("default".to_string()))
    }
    
    /// Retourne le sample rate actuel
    pub fn current_sample_rate(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|r| r.config.sample_rate())
    }
    
    /// Retourne la taille du buffer actuelle
    pub fn current_buffer_size(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|r| r.config.buffer_size())
    }
    
    /// Retourne la taille du buffer demandée
    pub fn requested_buffer_size(&self) -> Option<u32> {
        self.current_buffer_size()
    }
    
    /// Retourne la taille du buffer du stream
    pub fn stream_buffer_size(&self) -> Option<u32> {
        self.current_buffer_size()
    }
    
    /// Vérifie s'il y a une incompatibilité de buffer
    pub fn buffer_size_mismatch(&self) -> Option<bool> {
        self.runtime.lock().as_ref().map(|_| false)
    }
    
    /// Vérifie si le VST supporte MIDI
    pub fn vst_midi_compatible(&self) -> Option<bool> {
        self.runtime.lock().as_ref().map(|r| r.plugin.lock().supports_midi())
    }
    
    /// Vérifie si le limiter est activé
    pub fn limiter_enabled(&self) -> Option<bool> {
        self.runtime.lock().as_ref().map(|r| r.config.plugin_config.limiter_enabled)
    }
    
    /// Envoie des données MIDI au plugin
    pub fn send_midi(&self, data: &[u8]) {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            if let Some(mut plugin) = runtime.plugin.try_lock() {
                plugin.send_midi(data);
            }
        }
    }
    
    /// Panique toutes les notes (envoie note off sur tous les canaux)
    pub fn panic_all_notes(&self) -> Result<(), AudioError> {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            if let Some(mut plugin) = runtime.plugin.try_lock() {
                // Envoyer Note Off sur tous les canaux et notes
                for channel in 0..16 {
                    for note in 0..128 {
                        let msg = [0x80 | channel, note, 0];
                        plugin.send_midi(&msg);
                    }
                }
            }
        }
        Ok(())
    }
    
    /// Liste les paramètres VST
    pub fn list_vst_parameters(&self) -> Result<Vec<crate::types::VstParameter>, AudioError> {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            Ok(runtime.plugin.lock().get_parameters())
        } else {
            Ok(Vec::new())
        }
    }
    
    /// Définit un paramètre VST
    pub fn set_vst_parameter(&self, index: usize, value: f32) -> Result<(), AudioError> {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            runtime.plugin.lock().set_parameter(index, value)?;
        }
        Ok(())
    }
    
    /// Ouvre l'interface VST
    pub fn open_vst_ui(&self, app: tauri::AppHandle) -> Result<(), AudioError> {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            let hwnd = get_window_hwnd(&app)?;
            runtime.plugin.lock().open_editor(Some(hwnd))?;
        }
        Ok(())
    }
    
    /// Ferme l'interface VST
    pub fn close_vst_ui(&self, _app: tauri::AppHandle) -> Result<(), AudioError> {
        if let Some(runtime) = self.runtime.lock().as_ref() {
            runtime.plugin.lock().close_editor()?;
        }
        Ok(())
    }
    
    /// Définit le gain
    pub fn set_gain(&self, gain_db: f32) {
        // TODO: Implémenter la mise à jour du gain
        // Pour l'instant, nous stockons dans la configuration
        if let Some(runtime) = self.runtime.lock().as_mut() {
            runtime.config.gain_config.gain_db = gain_db;
        }
    }
    
    /// Active/désactive le limiter
    pub fn set_limiter_enabled(&self, enabled: bool) {
        if let Some(runtime) = self.runtime.lock().as_mut() {
            runtime.config.plugin_config.limiter_enabled = enabled;
        }
    }
    
    /// Ping le moteur audio (vérifie qu'il est fonctionnel)
    pub fn ping(&self) -> Result<(), AudioError> {
        if self.is_running() {
            Ok(())
        } else {
            Err(AudioError::NotStarted)
        }
    }
    
    /// Recharge le moteur audio avec de nouvelles settings
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
    
    fn stop_internal(&self, app_handle: Option<tauri::AppHandle>) -> Result<(), AudioError> {
        let runtime = self.runtime.lock().take();
        if runtime.is_some() {
            background_log("info", "Moteur audio arrêté");
        }
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
