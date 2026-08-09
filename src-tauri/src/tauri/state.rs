//! Etat global de l'application Tauri.

use std::sync::{atomic::AtomicBool, Arc};

use crate::{
    audio::AudioEngine, bridge::BridgeHandle, config::ConfigStore, rtp::RtpDiscoveryManager,
    types::VstPluginEntry,
};
use parking_lot::Mutex;

use crate::tauri::utils::load_vst_cache_from_disk;

pub struct AppState {
    pub config_store: ConfigStore,
    pub bridge: BridgeHandle,
    pub dev_logging: Arc<AtomicBool>,
    pub audio: AudioEngine,
    pub vst_cache: Mutex<Option<Vec<VstPluginEntry>>>,
    pub rtp_discovery: RtpDiscoveryManager,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config_store: ConfigStore::new(),
            bridge: BridgeHandle::new(),
            dev_logging: Arc::new(AtomicBool::new(false)),
            audio: AudioEngine::new(),
            vst_cache: Mutex::new(load_vst_cache_from_disk()),
            rtp_discovery: RtpDiscoveryManager::new(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
