//! État global de l'application Tauri

use std::sync::Arc;

use crate::audio::AudioEngine;
use crate::bridge::BridgeHandle;
use crate::config::ConfigStore;
use crate::rtp::RtpDiscoveryManager;
use parking_lot::Mutex;
use std::sync::atomic::AtomicBool;

/// État global partagé de l'application
pub struct AppState {
    pub config_store: ConfigStore,
    pub audio: Arc<AudioEngine>,
    pub bridge: Arc<Mutex<Option<BridgeHandle>>>,
    pub rtp_discovery: Arc<Mutex<Option<RtpDiscoveryManager>>>,
    pub dev_logging: Arc<AtomicBool>,
}

impl AppState {
    /// Crée une nouvelle instance de l'état de l'application
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let config_store = ConfigStore::new();
        let audio = Arc::new(AudioEngine::new());
        let bridge = Arc::new(Mutex::new(None));
        let rtp_discovery = Arc::new(Mutex::new(None));
        let dev_logging = Arc::new(AtomicBool::new(false));

        Ok(Self {
            config_store,
            audio,
            bridge,
            rtp_discovery,
            dev_logging,
        })
    }

    /// Retourne une référence au bridge s'il existe
    pub fn bridge_handle(&self) -> Option<BridgeHandle> {
        self.bridge.lock().clone()
    }

    /// Définit le bridge handle
    pub fn set_bridge(&self, bridge: BridgeHandle) {
        *self.bridge.lock() = Some(bridge);
    }

    /// Retourne une référence au gestionnaire RTP s'il existe
    pub fn rtp_manager(&self) -> Option<RtpDiscoveryManager> {
        self.rtp_discovery.lock().clone()
    }

    /// Définit le gestionnaire RTP
    pub fn set_rtp_manager(&self, manager: RtpDiscoveryManager) {
        *self.rtp_discovery.lock() = Some(manager);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new().expect("Failed to create app state")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_creation() {
        let _app_state = AppState::new().unwrap();
    }
}
