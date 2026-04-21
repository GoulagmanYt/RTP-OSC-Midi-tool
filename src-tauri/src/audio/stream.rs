//! Gestion des streams audio CPAL
//! 
//! Ce module contient toute la logique de création et gestion des streams audio
//! avec CPAL, incluant la sélection de devices, configuration et callbacks.

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, Device, FromSample, Sample, SampleFormat, SampleRate, SizedSample,
    Stream, StreamConfig, SupportedStreamConfigRange, SupportedBufferSize,
};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;

use crate::audio::config::AudioStreamConfig;
use crate::audio::callback::AudioCallbackState;
use crate::audio::windows::apply_realtime_priority;

/// Erreurs liées aux streams audio
#[derive(Debug, Error, Clone)]
pub enum StreamError {
    #[error("Device audio non trouvé: {0}")]
    DeviceNotFound(String),
    
    #[error("Configuration audio non supportée")]
    UnsupportedConfig,
    
    #[error("Erreur lors de la création du stream: {0}")]
    CreationFailed(String),
    
    #[error("Backend audio non disponible")]
    BackendUnavailable,
}

/// Gestionnaire de stream audio
pub struct StreamManager {
    /// Stream actif (stocké dans un Arc<Mutex<Option>> pour éviter les problèmes Send)
    stream: Arc<Mutex<Option<Stream>>>,
    /// Device utilisé
    device: Option<Device>,
    /// Configuration effective
    active_config: Option<StreamConfig>,
}

impl StreamManager {
    /// Crée un nouveau gestionnaire de stream
    pub fn new() -> Self {
        Self {
            stream: Arc::new(Mutex::new(None)),
            device: None,
            active_config: None,
        }
    }
    
    /// Crée un stream audio avec la configuration spécifiée
    pub fn create_stream(
        &mut self,
        config: &AudioStreamConfig,
        callback_state: AudioCallbackState,
        gain_bits: Arc<AtomicU32>,
    ) -> Result<(), StreamError> {
        // Sélectionner le device
        let device = self.select_device(&config.backend, &config.device)?;
        
        // Déterminer la configuration supportée
        let (supported_config, sample_rate) = self.select_output_config(&device, config.sample_rate())?;
        
        // Construire le stream
        let stream = self.build_stream_internal(
            &device,
            sample_rate,
            config.buffer_size(),
            callback_state,
            gain_bits,
            &supported_config,
        )?;
        
        // Stocker les informations
        *self.stream.lock() = Some(stream);
        self.device = Some(device);
        self.active_config = Some(StreamConfig {
            channels: supported_config.channels(),
            sample_rate: SampleRate(sample_rate),
            buffer_size: BufferSize::Fixed(config.buffer_size()),
        });
        
        Ok(())
    }
    
    /// Démarre le stream audio
    pub fn start(&mut self) -> Result<(), StreamError> {
        if let Some(ref stream) = *self.stream.lock() {
            stream.play()
                .map_err(|e| StreamError::CreationFailed(e.to_string()))?;
        }
        Ok(())
    }
    
    /// Arrête et détruit le stream
    pub fn stop(&mut self) -> Result<(), StreamError> {
        *self.stream.lock() = None;
        self.device = None;
        self.active_config = None;
        Ok(())
    }
    
    /// Vérifie si un stream est actif
    pub fn is_active(&self) -> bool {
        self.stream.lock().is_some()
    }
    
    /// Retourne la configuration actuelle
    pub fn active_config(&self) -> Option<&StreamConfig> {
        self.active_config.as_ref()
    }
    
    /// Liste les backends disponibles
    pub fn list_backends() -> Vec<String> {
        cpal::available_hosts()
            .into_iter()
            .map(|h| h.name().to_string())
            .collect()
    }
    
    /// Liste les devices pour un backend donné
    pub fn list_devices(backend: Option<&str>) -> Vec<String> {
        let host = Self::select_host(backend);
        match host {
            Some(h) => match h.output_devices() {
                Ok(devices) => devices.filter_map(|d| d.name().ok()).collect(),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        }
    }
    
    // Méthodes privées
    
    /// Sélectionne un host CPAL
    fn select_host(backend: Option<&str>) -> Option<cpal::Host> {
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
    
    /// Sélectionne un device audio
    fn select_device(
        &self,
        backend: &Option<String>,
        device_name: &Option<String>,
    ) -> Result<Device, StreamError> {
        let host = Self::select_host(backend.as_deref())
            .ok_or(StreamError::BackendUnavailable)?;
        
        let devices = host.output_devices()
            .map_err(|e| StreamError::CreationFailed(e.to_string()))?;
        
        if let Some(ref name) = device_name {
            // Chercher un device spécifique
            for device in devices {
                if let Ok(device_name) = device.name() {
                    if device_name.to_lowercase().contains(&name.to_lowercase()) {
                        return Ok(device);
                    }
                }
            }
            Err(StreamError::DeviceNotFound(name.clone()))
        } else {
            // Prendre le device par défaut
            devices.into_iter()
                .next()
                .ok_or(StreamError::DeviceNotFound("Aucun device trouvé".to_string()))
        }
    }
    
    /// Sélectionne la configuration de sortie supportée
    fn select_output_config(
        &self,
        device: &Device,
        requested_rate: u32,
    ) -> Result<(SupportedStreamConfigRange, u32), StreamError> {
        let supported = device.supported_output_configs()
            .map_err(|e| StreamError::CreationFailed(e.to_string()))?;
        
        let mut best_config = None;
        let mut best_score = 0;
        
        for cfg in supported {
            let (sample_rate, score) = self.pick_sample_rate(&cfg, requested_rate);
            if score > best_score {
                best_score = score;
                best_config = Some((cfg, sample_rate));
            }
        }
        
        best_config.ok_or(StreamError::UnsupportedConfig)
    }
    
    /// Choisit le meilleur sample rate
    fn pick_sample_rate(&self, cfg: &SupportedStreamConfigRange, requested: u32) -> (u32, u32) {
        let min = cfg.min_sample_rate().0;
        let max = cfg.max_sample_rate().0;
        
        if requested < min {
            (min, 1000 - (min - requested))
        } else if requested > max {
            (max, 1000 - (requested - max))
        } else {
            (requested, 1000)
        }
    }
    
    /// Construit le stream audio interne
    fn build_stream_internal(
        &self,
        device: &Device,
        sample_rate: u32,
        buffer_size: u32,
        _callback_state: AudioCallbackState,
        _gain_bits: Arc<AtomicU32>,
        supported_config: &SupportedStreamConfigRange,
    ) -> Result<Stream, StreamError> {
        let err_fn = |err| eprintln!("an error occurred on stream: {}", err);
        
        let sample_format = supported_config.sample_format();
        let config = cpal::StreamConfig {
            channels: supported_config.channels(),
            sample_rate: SampleRate(sample_rate),
            buffer_size: BufferSize::Fixed(buffer_size),
        };
        
        match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data, _info| {
                    // TODO: Implémenter le callback audio réel
                    // Pour l'instant, générer du silence
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data, _info| {
                    // TODO: Implémenter le callback audio réel
                    for sample in data.iter_mut() {
                        *sample = 0;
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data, _info| {
                    // TODO: Implémenter le callback audio réel
                    for sample in data.iter_mut() {
                        *sample = 32768; // Milieu de la plage u16
                    }
                },
                err_fn,
                None,
            ),
            _ => return Err(StreamError::UnsupportedConfig),
        }
        .map_err(|e| StreamError::CreationFailed(e.to_string()))
    }
    
    /// Choisit la taille du buffer
    fn choose_buffer_size(&self, supported: &SupportedBufferSize, requested: u32) -> u32 {
        match supported {
            SupportedBufferSize::Range { min, max } => requested.max(*min).min(*max),
            SupportedBufferSize::Unknown => requested,
        }
    }
    
    /// Calcule la taille maximale pour les plugins
    fn max_plugin_block_size(&self, supported: &SupportedBufferSize, requested: u32) -> u32 {
        match supported {
            SupportedBufferSize::Range { min, max } => requested.max(*min).max(*max),
            SupportedBufferSize::Unknown => requested.max(2048),
        }
    }
    
    /// Génère les candidats de fallback pour la taille du buffer
    fn buffer_fallback_candidates(&self, supported: &SupportedBufferSize, preferred: u32) -> Vec<u32> {
        let common_sizes = [
            64u32, 96, 128, 192, 256, 384, 480, 512, 768, 1024, 1536, 2048,
        ];
        
        let mut candidates = Vec::new();
        
        // Ajouter les tailles communes dans l'ordre de préférence
        for &size in &common_sizes {
            if size == preferred {
                continue; // Déjà traité
            }
            if self.choose_buffer_size(supported, size) == size {
                candidates.push(size);
            }
        }
        
        candidates
    }
}

impl Default for StreamManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_stream_manager_creation() {
        let manager = StreamManager::new();
        assert!(!manager.is_active());
        assert!(manager.active_config().is_none());
    }
    
    #[test]
    fn test_list_backends() {
        let backends = StreamManager::list_backends();
        assert!(!backends.is_empty());
    }
    
    #[test]
    fn test_list_devices() {
        let devices = StreamManager::list_devices(None);
        // Peut être vide sur certains systèmes, mais ne doit pas paniquer
    }
}
