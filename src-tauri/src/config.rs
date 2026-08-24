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
    #[serde(default)]
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub gain_db: f32,
    pub limiter_enabled: bool,
    #[serde(default)]
    pub vst_plugin_id: Option<String>,
    pub vst_path: Option<String>,
    #[serde(default)]
    pub vst_scan_paths: Vec<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Some("auto".to_string()),
            device: None,
            device_id: None,
            sample_rate: 48_000,
            buffer_size: 256,
            gain_db: 0.0,
            limiter_enabled: false,
            vst_plugin_id: None,
            vst_path: None,
            vst_scan_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    pub theme: Theme,
    pub theme_palette: String,
    pub corner_radius: f32,
    pub auto_start: bool,
    pub developer_mode: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            theme_palette: "light".to_string(),
            corner_radius: 12.0,
            auto_start: false,
            developer_mode: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoggingConfig {
    pub enabled: bool,
    pub verbose: bool,
    pub log_all_to_file: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            verbose: false,
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
    pub const CURRENT_VERSION: u32 = 4;
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
        let cfg_dir = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
            .map(|dirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| {
                let fallback = std::env::temp_dir().join("OSCMidi").join("config");
                log::warn!(
                    "User configuration directory is unavailable; falling back to {:?}",
                    fallback
                );
                fallback
            });
        if let Err(error) = fs::create_dir_all(&cfg_dir) {
            log::error!(
                "Unable to create configuration directory {:?}: {error}",
                cfg_dir
            );
        }
        Self::with_path(cfg_dir.join("config.yaml"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        let store = Self { path };
        store.ensure_exists();
        store
    }

    pub fn load(&self) -> AppConfig {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return self.write_default();
            }
            Err(error) => {
                log::error!("Unable to read configuration {:?}: {error}", self.path);
                return AppConfig::default();
            }
        };
        let Ok(mut value) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
            log::error!(
                "Invalid YAML configuration preserved at {:?}; using defaults for this session",
                self.path
            );
            return AppConfig::default();
        };
        let version = value
            .get("version")
            .and_then(serde_yaml::Value::as_u64)
            .unwrap_or_default() as u32;
        if matches!(version, 2 | 3) {
            if let Some(mapping) = value.as_mapping_mut() {
                mapping.insert(
                    serde_yaml::Value::String("version".into()),
                    serde_yaml::Value::Number(AppConfig::CURRENT_VERSION.into()),
                );
            }
        } else if version != AppConfig::CURRENT_VERSION {
            log::error!(
                "Unsupported configuration version {version} preserved at {:?}; using defaults for this session",
                self.path
            );
            return AppConfig::default();
        }
        let Ok(cfg) = serde_yaml::from_value::<AppConfig>(value) else {
            log::error!(
                "Invalid configuration schema preserved at {:?}; using defaults for this session",
                self.path
            );
            return AppConfig::default();
        };
        if matches!(version, 2 | 3) {
            if let Err(error) = self.save(&cfg) {
                log::warn!("Unable to persist migrated configuration: {error}");
            }
        }
        cfg
    }

    pub fn save(&self, cfg: &AppConfig) -> Result<(), String> {
        let raw = serde_yaml::to_string(cfg).map_err(|e| e.to_string())?;
        crate::atomic_file::write_atomically(&self.path, raw.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn reset_to_default(&self) -> Result<AppConfig, String> {
        let cfg = AppConfig::default();
        self.save(&cfg)?;
        Ok(cfg)
    }

    fn ensure_exists(&self) {
        if !self.path.exists() {
            if let Err(error) = self.save(&AppConfig::default()) {
                log::error!(
                    "Unable to initialize configuration {:?}: {error}",
                    self.path
                );
            }
        }
    }

    fn write_default(&self) -> AppConfig {
        let default_cfg = AppConfig::default();
        if let Err(error) = self.save(&default_cfg) {
            log::error!(
                "Unable to persist default configuration {:?}: {error}",
                self.path
            );
        }
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
            audio_backend_latency_ms: None,
            plugin_latency_samples: None,
            bridge_latency_samples: None,
            vst_hosting_mode: None,
            audio_backend: None,
            audio_device: None,
            audio_device_id: None,
            audio_sample_rate: None,
            audio_buffer_size: None,
            audio_requested_buffer_size: None,
            audio_stream_buffer_size: None,
            audio_buffer_mismatch: None,
            vst_loaded: false,
            vst_midi_compatible: None,
            audio_xruns: None,
            audio_stream_recovery_requests: None,
            audio_stream_route_changes: None,
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
            x86_bridge_underruns: None,
            x86_bridge_overruns: None,
            x86_worker_alive: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_preserves_invalid_payload_while_using_defaults() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "legacy: true").expect("write legacy");

        let store = ConfigStore::with_path(path.clone());
        let config = store.load();

        assert_eq!(config.version, AppConfig::CURRENT_VERSION);
        let raw = std::fs::read_to_string(path).expect("read preserved");
        assert_eq!(raw, "legacy: true");
    }

    #[test]
    fn store_preserves_future_version_while_using_defaults() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let legacy = AppConfig {
            version: AppConfig::CURRENT_VERSION + 1,
            ..AppConfig::default()
        };
        std::fs::write(&path, serde_yaml::to_string(&legacy).expect("yaml")).expect("write");

        let original = serde_yaml::to_string(&legacy).expect("yaml");
        let store = ConfigStore::with_path(path.clone());
        let config = store.load();

        assert_eq!(config.version, AppConfig::CURRENT_VERSION);
        assert_eq!(std::fs::read_to_string(path).expect("preserved"), original);
    }

    #[test]
    fn v2_audio_config_migrates_without_losing_legacy_vst_path() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let mut legacy = serde_yaml::to_string(&AppConfig::default()).expect("serialize");
        legacy = legacy
            .replace("version: 3", "version: 2")
            .replace("  vstPluginId: null\n", "")
            .replace("  vstScanPaths: []\n", "")
            .replace("  vstPath: null", "  vstPath: C:\\Legacy\\Organ.dll");
        std::fs::write(&path, legacy).expect("write legacy");

        let migrated = ConfigStore::with_path(path).load();
        assert_eq!(migrated.version, AppConfig::CURRENT_VERSION);
        assert_eq!(migrated.audio.vst_plugin_id, None);
        assert!(migrated.audio.vst_scan_paths.is_empty());
        assert_eq!(
            migrated.audio.vst_path.as_deref(),
            Some("C:\\Legacy\\Organ.dll")
        );
    }

    #[test]
    fn removed_v2_fields_are_ignored_and_removed_on_next_save() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let current = serde_yaml::to_string(&AppConfig::default()).expect("serialize config");
        let legacy = current
            .replace(
                "audio:\n  enabled: true",
                "audio:\n  enabled: true\n  vstWorkerEnabled: false",
            )
            .replace(
                "ui:\n  theme:",
                "ui:\n  themePreset: studio\n  accent: auto\n  alwaysOnTop: false\n  theme:",
            )
            .replace(
                "logging:\n  enabled: true",
                "logging:\n  enabled: true\n  liveLogs: false",
            );
        std::fs::write(&path, legacy).expect("write legacy config");

        let store = ConfigStore::with_path(path.clone());
        let config = store.load();
        assert!(config.audio.enabled);

        store.save(&config).expect("save migrated config");
        let migrated = std::fs::read_to_string(path).expect("read migrated config");
        assert!(!migrated.contains("vstWorkerEnabled"));
        assert!(!migrated.contains("themePreset"));
        assert!(!migrated.contains("alwaysOnTop"));
        assert!(!migrated.contains("liveLogs"));
    }
}
