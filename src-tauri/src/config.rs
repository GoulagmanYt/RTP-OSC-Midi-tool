use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

use crate::types::RuntimeStatus;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RtpRemoteEntry {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingMapping {
    pub from: u8,
    pub to: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingProfile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub channel_filter: Option<u8>,
    pub note_min: Option<u8>,
    pub note_max: Option<u8>,
    pub cc_map: Vec<RoutingMapping>,
    pub program_map: Vec<RoutingMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingAssignment {
    pub source: String,
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarConfig {
    pub path: Option<String>,
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
}

impl Default for AvatarConfig {
    fn default() -> Self {
        Self {
            path: None,
            offset_x: 50.0,
            offset_y: 50.0,
            scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MidiConfig {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub channel_filter: Option<u8>,
    pub thru_enabled: bool,
    pub hotplug: bool,
    pub routing_profiles: Vec<RoutingProfile>,
    pub routing_assignments: Vec<RoutingAssignment>,
}

impl Default for MidiConfig {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            channel_filter: None,
            thru_enabled: true,
            hotplug: true,
            routing_profiles: Vec::new(),
            routing_assignments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OscConfig {
    pub enabled: bool,
    pub target_ip: String,
    pub target_port: u16,
    pub log_messages: bool,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_ip: "127.0.0.1".to_string(),
            target_port: 9000,
            log_messages: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RtpConfig {
    pub enabled: bool,
    pub session_name: String,
    pub port: u16,
    pub remote_enabled: bool,
    pub remotes: Vec<RtpRemoteEntry>,
    pub log_messages: bool,
}

impl Default for RtpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            session_name: "OSCMidi".to_string(),
            port: 5004,
            remote_enabled: false,
            remotes: Vec::new(),
            log_messages: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfig {
    pub enabled: bool,
    pub backend: Option<String>,
    pub device: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub gain_db: f32,
    pub limiter_enabled: bool,
    pub vst_path: Option<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Some("auto".to_string()),
            device: None,
            sample_rate: 48_000,
            buffer_size: 256,
            gain_db: 0.0,
            limiter_enabled: false,
            vst_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    pub theme_preset: String,
    pub theme: Theme,
    pub theme_palette: String,
    pub accent: String,
    pub corner_radius: f32,
    pub scale: f32,
    pub content_padding: f32,
    pub sidebar_width: f32,
    pub surface_opacity: f32,
    pub card_opacity: f32,
    pub aurora_intensity: f32,
    pub grain_intensity: f32,
    pub reduce_motion: bool,
    pub auto_start: bool,
    pub always_on_top: bool,
    pub developer_mode: bool,
    pub avatar: AvatarConfig,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme_preset: "studio".to_string(),
            theme: Theme::Light,
            theme_palette: "light".to_string(),
            accent: "auto".to_string(),
            corner_radius: 12.0,
            scale: 1.0,
            content_padding: 24.0,
            sidebar_width: 256.0,
            surface_opacity: 0.5,
            card_opacity: 1.0,
            aurora_intensity: 0.75,
            grain_intensity: 0.16,
            reduce_motion: false,
            auto_start: false,
            always_on_top: false,
            developer_mode: false,
            avatar: AvatarConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoggingConfig {
    pub enabled: bool,
    pub verbose: bool,
    pub live_logs: bool,
    pub log_all_to_file: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            verbose: false,
            live_logs: false,
            log_all_to_file: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub version: u32,
    pub midi: MidiConfig,
    pub osc: OscConfig,
    pub rtp: RtpConfig,
    pub audio: AudioConfig,
    pub ui: UiConfig,
    pub logging: LoggingConfig,
}

impl AppConfig {
    pub const CURRENT_VERSION: u32 = 2;
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            midi: MidiConfig::default(),
            osc: OscConfig::default(),
            rtp: RtpConfig::default(),
            audio: AudioConfig::default(),
            ui: UiConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

pub type Config = AppConfig;

pub const RTP_VIRTUAL_INPUT: &str = "RTP-MIDI (Serveur)";
pub const VST_INTERNAL_OUTPUT: &str = "VST (Interne)";
pub const VST_INTERNAL_OUTPUT_LEGACY: &str = "VST (Keyzone interne)";

pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Self {
        let dirs = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
            .expect("Impossible de déterminer le répertoire utilisateur");
        let cfg_dir = dirs.config_dir();
        fs::create_dir_all(cfg_dir).ok();
        Self::with_path(cfg_dir.join("config.yaml"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        let store = Self { path };
        store.ensure_exists();
        store
    }

    pub fn load(&self) -> AppConfig {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return self.write_default();
        };
        let Ok(cfg) = serde_yaml::from_str::<AppConfig>(&raw) else {
            return self.write_default();
        };
        if cfg.version != AppConfig::CURRENT_VERSION {
            return self.write_default();
        }
        cfg
    }

    pub fn save(&self, cfg: &AppConfig) -> Result<(), String> {
        let raw = serde_yaml::to_string(cfg).map_err(|e| e.to_string())?;
        fs::write(&self.path, raw).map_err(|e| e.to_string())
    }

    pub fn reset_to_default(&self) -> Result<AppConfig, String> {
        let cfg = AppConfig::default();
        self.save(&cfg)?;
        Ok(cfg)
    }

    fn ensure_exists(&self) {
        if !self.path.exists() {
            let _ = self.save(&AppConfig::default());
        }
    }

    fn write_default(&self) -> AppConfig {
        let default_cfg = AppConfig::default();
        let _ = self.save(&default_cfg);
        default_cfg
    }
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&AppConfig> for RuntimeStatus {
    fn from(cfg: &AppConfig) -> Self {
        Self {
            running: false,
            midi_input: cfg.midi.input_device.clone(),
            midi_output: cfg.midi.output_device.clone(),
            osc_target: format!("{}:{}", cfg.osc.target_ip, cfg.osc.target_port),
            rtp_active: cfg.rtp.enabled
                || cfg.rtp.remote_enabled
                || cfg
                    .midi
                    .input_device
                    .as_ref()
                    .map(|s| s == RTP_VIRTUAL_INPUT)
                    .unwrap_or(false),
            rtp_bound_port: None,
            rtp_advertised_host: None,
            rtp_advertised_addresses: Vec::new(),
            rtp_network_warning: None,
            last_error: None,
            audio_running: false,
            audio_latency_ms: None,
            audio_buffer_period_ms: None,
            plugin_latency_samples: None,
            audio_backend: None,
            audio_device: None,
            audio_sample_rate: None,
            audio_buffer_size: None,
            audio_requested_buffer_size: None,
            audio_stream_buffer_size: None,
            audio_buffer_mismatch: None,
            vst_loaded: false,
            vst_midi_compatible: None,
            audio_xruns: None,
            audio_midi_drops: None,
            audio_lock_misses: None,
            audio_emergency_resets: None,
            audio_callback_max_us: None,
            audio_callback_last_us: None,
            audio_callback_over_budget_count: None,
            consecutive_deadline_misses: None,
            dsp_process_last_us: None,
            dsp_process_p95_us: None,
            dsp_process_p99_us: None,
            dsp_process_max_us: None,
            audio_midi_queue_depth: None,
            audio_midi_queue_max_depth: None,
            audio_midi_oldest_us: None,
            audio_lifecycle_state: "stopped".to_string(),
            vst_worker_state: "disabled".to_string(),
            vst_worker_restarts: 0,
            vst_worker_last_exit: None,
            audio_mmcss_enabled: None,
            audio_power_throttling_disabled: None,
            audio_limiter_enabled: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_resets_invalid_payload_to_defaults() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "legacy: true").expect("write legacy");

        let store = ConfigStore::with_path(path.clone());
        let config = store.load();

        assert_eq!(config.version, AppConfig::CURRENT_VERSION);
        let raw = std::fs::read_to_string(path).expect("read rewritten");
        assert!(raw.contains("version:"));
    }

    #[test]
    fn store_resets_wrong_version_to_defaults() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let legacy = AppConfig {
            version: AppConfig::CURRENT_VERSION + 1,
            ..AppConfig::default()
        };
        std::fs::write(&path, serde_yaml::to_string(&legacy).expect("yaml")).expect("write");

        let store = ConfigStore::with_path(path);
        let config = store.load();

        assert_eq!(config.version, AppConfig::CURRENT_VERSION);
    }
}
