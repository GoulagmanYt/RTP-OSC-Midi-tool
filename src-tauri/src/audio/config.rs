//! Configuration unifiée pour le moteur audio
//! Centralise tous les paramètres audio pour réduire la complexité des fonctions

use cpal::{BufferSize, SampleRate, StreamConfig};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AudioStreamConfig {
    /// Backend audio (ASIO, WASAPI, etc.)
    #[allow(dead_code)]
    pub backend: Option<String>,
    /// Nom du device audio
    #[allow(dead_code)]
    pub device: Option<String>,
    /// Configuration CPAL du stream
    pub cpal_config: StreamConfig,
    /// Configuration du plugin VST
    pub plugin_config: PluginConfig,
    /// Configuration du gain
    pub gain_config: GainConfig,
    /// Chemin vers le plugin VST
    pub vst_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    /// Sample rate pour le plugin
    pub sample_rate: u32,
    /// Taille du buffer pour le plugin
    pub buffer_size: u32,
    /// Limiter activé
    pub limiter_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GainConfig {
    /// Gain en décibels
    pub gain_db: f32,
}

#[derive(Debug, Error, Clone)]
pub enum ConfigError {
    #[error("Sample rate invalide: {0}")]
    InvalidSampleRate(u32),
    
    #[error("Buffer size invalide: {0}")]
    InvalidBufferSize(u32),
    
    #[error("Gain invalide: {0} dB")]
    InvalidGain(f32),
    
    #[error("Backend audio non supporté: {0}")]
    UnsupportedBackend(String),
    
    #[error("Device audio non trouvé: {0}")]
    DeviceNotFound(String),
}

impl AudioStreamConfig {
    /// Crée une configuration par défaut
    pub fn default() -> Self {
        Self {
            backend: Some("auto".to_string()),
            device: None,
            cpal_config: StreamConfig {
                channels: 2,
                sample_rate: SampleRate(48_000),
                buffer_size: BufferSize::Fixed(256),
            },
            plugin_config: PluginConfig {
                sample_rate: 48_000,
                buffer_size: 256,
                limiter_enabled: false,
            },
            gain_config: GainConfig { gain_db: 0.0 },
            vst_path: None,
        }
    }
    
    /// Valide la configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Valider sample rate
        if self.cpal_config.sample_rate.0 < 8000 || self.cpal_config.sample_rate.0 > 192000 {
            return Err(ConfigError::InvalidSampleRate(self.cpal_config.sample_rate.0));
        }
        
        // Valider buffer size
        let buffer_size = match self.cpal_config.buffer_size {
            BufferSize::Fixed(size) => size,
            BufferSize::Default => 256,
        };
        if buffer_size < 32 || buffer_size > 8192 {
            return Err(ConfigError::InvalidBufferSize(buffer_size));
        }
        
        // Valider gain
        if self.gain_config.gain_db < -60.0 || self.gain_config.gain_db > 20.0 {
            return Err(ConfigError::InvalidGain(self.gain_config.gain_db));
        }
        
        // Valider cohérence sample rate
        if self.cpal_config.sample_rate.0 != self.plugin_config.sample_rate {
            return Err(ConfigError::InvalidSampleRate(self.plugin_config.sample_rate));
        }
        
        Ok(())
    }
    
    /// Convertit la configuration en configuration CPAL
    #[allow(dead_code)]
    pub fn to_cpal_config(&self) -> &StreamConfig {
        &self.cpal_config
    }
    
    /// Retourne le sample rate
    #[allow(dead_code)]
    pub fn sample_rate(&self) -> u32 {
        self.cpal_config.sample_rate.0
    }
    
    /// Retourne la taille du buffer
    #[allow(dead_code)]
    pub fn buffer_size(&self) -> u32 {
        match self.cpal_config.buffer_size {
            BufferSize::Fixed(size) => size,
            BufferSize::Default => 256,
        }
    }
    
    /// Retourne le nombre de canaux
    #[allow(dead_code)]
    pub fn channels(&self) -> u16 {
        self.cpal_config.channels
    }
    
    /// Retourne le gain linéaire (converti depuis dB)
    #[allow(dead_code)]
    pub fn gain_linear(&self) -> f32 {
        db_to_linear(self.gain_config.gain_db)
    }
}

/// Convertit les décibels en valeur linéaire
#[allow(dead_code)]
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Convertit la valeur linéaire en décibels
#[allow(dead_code)]
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -60.0
    } else {
        20.0 * linear.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = AudioStreamConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.sample_rate(), 48_000);
        assert_eq!(config.buffer_size(), 256);
        assert_eq!(config.channels(), 2);
        assert_eq!(config.gain_config.gain_db, 0.0);
    }
    
    #[test]
    fn test_invalid_sample_rate() {
        let mut config = AudioStreamConfig::default();
        config.cpal_config.sample_rate = SampleRate(1000); // Trop bas
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_invalid_buffer_size() {
        let mut config = AudioStreamConfig::default();
        config.cpal_config.buffer_size = BufferSize::Fixed(16); // Trop petit
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_invalid_gain() {
        let mut config = AudioStreamConfig::default();
        config.gain_config.gain_db = 100.0; // Trop élevé
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_db_conversion() {
        assert_eq!(db_to_linear(0.0), 1.0);
        assert_eq!(db_to_linear(-6.0), 10.0_f32.powf(-6.0/20.0));
        assert_eq!(linear_to_db(1.0), 0.0);
        assert_eq!(linear_to_db(0.5), 20.0 * 0.5_f32.log10());
    }
}
