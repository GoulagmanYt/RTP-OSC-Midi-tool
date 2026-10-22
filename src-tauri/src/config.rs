use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

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
#[serde(default)]
pub struct OscConfig {
    pub input_enabled: bool,
    pub listen_ip: String,
    pub listen_port: u16,
    pub enabled: bool,
    pub target_ip: String,
    pub target_port: u16,
    pub log_messages: bool,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            input_enabled: false,
            listen_ip: "127.0.0.1".into(),
            listen_port: 9001,
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
    #[serde(default)]
    pub thru_enabled: bool,
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
            thru_enabled: false,
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
pub struct EqShelfSettings {
    pub frequency_hz: f32,
    pub gain_db: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EqPeakSettings {
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VstEqSettings {
    pub enabled: bool,
    pub low_shelf: EqShelfSettings,
    pub mid_peak: EqPeakSettings,
    pub high_shelf: EqShelfSettings,
}

impl Default for VstEqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            low_shelf: EqShelfSettings {
                frequency_hz: 120.0,
                gain_db: 0.0,
            },
            mid_peak: EqPeakSettings {
                frequency_hz: 1_000.0,
                gain_db: 0.0,
                q: 1.0,
            },
            high_shelf: EqShelfSettings {
                frequency_hz: 8_000.0,
                gain_db: 0.0,
            },
        }
    }
}

impl VstEqSettings {
    pub fn normalized(&self, sample_rate: u32) -> Self {
        let nyquist_safe = (sample_rate.max(1) as f32 * 0.45).max(40.0);
        Self {
            enabled: self.enabled,
            low_shelf: EqShelfSettings {
                frequency_hz: finite_clamp(
                    self.low_shelf.frequency_hz,
                    120.0,
                    40.0,
                    500.0f32.min(nyquist_safe),
                ),
                gain_db: finite_clamp(self.low_shelf.gain_db, 0.0, -18.0, 18.0),
            },
            mid_peak: EqPeakSettings {
                frequency_hz: finite_clamp(
                    self.mid_peak.frequency_hz,
                    1_000.0f32.min(nyquist_safe),
                    100.0,
                    10_000.0f32.min(nyquist_safe),
                ),
                gain_db: finite_clamp(self.mid_peak.gain_db, 0.0, -18.0, 18.0),
                q: finite_clamp(self.mid_peak.q, 1.0, 0.2, 10.0),
            },
            high_shelf: EqShelfSettings {
                frequency_hz: finite_clamp(
                    self.high_shelf.frequency_hz,
                    8_000.0f32.min(nyquist_safe),
                    2_000.0f32.min(nyquist_safe),
                    16_000.0f32.min(nyquist_safe),
                ),
                gain_db: finite_clamp(self.high_shelf.gain_db, 0.0, -18.0, 18.0),
            },
        }
    }
}

fn finite_clamp(value: f32, default: f32, minimum: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum.max(minimum))
    } else {
        default.clamp(minimum, maximum.max(minimum))
    }
}

pub const BUNDLED_VST_EQ_KEY: &str = "__bundled_default__";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfig {
    #[serde(default = "default_game_mode")]
    pub game_mode: bool,
    pub enabled: bool,
    pub backend: Option<String>,
    pub device: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    #[serde(
        default = "crate::audio::default_render_ahead_blocks",
        deserialize_with = "crate::audio::deserialize_render_ahead_blocks"
    )]
    pub render_ahead_blocks: u32,
    #[serde(default)]
    pub dsp_thread_priority: crate::audio::DspThreadPriority,
    pub gain_db: f32,
    pub limiter_enabled: bool,
    #[serde(default)]
    pub vst_plugin_id: Option<String>,
    pub vst_path: Option<String>,
    #[serde(default)]
    pub vst_scan_paths: Vec<String>,
    #[serde(default)]
    pub vst_eq_by_plugin: BTreeMap<String, VstEqSettings>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Some("auto".to_string()),
            device: None,
            device_id: None,
            sample_rate: 48_000,
            buffer_size: 512,
            render_ahead_blocks: crate::audio::default_render_ahead_blocks(),
            dsp_thread_priority: crate::audio::DspThreadPriority::default(),
            game_mode: true,
            gain_db: 0.0,
            limiter_enabled: false,
            vst_plugin_id: None,
            vst_path: None,
            vst_scan_paths: Vec::new(),
            vst_eq_by_plugin: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    pub theme: Theme,
    pub theme_palette: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default = "default_background_tone")]
    pub light_background: String,
    #[serde(default = "default_background_tone")]
    pub dark_background: String,
    pub corner_radius: f32,
    pub auto_start: bool,
    pub developer_mode: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            theme_palette: "light".to_string(),
            accent_color: default_accent_color(),
            light_background: default_background_tone(),
            dark_background: default_background_tone(),
            corner_radius: 12.0,
            auto_start: false,
            developer_mode: false,
        }
    }
}

fn default_accent_color() -> String {
    "blue".to_string()
}

fn default_background_tone() -> String {
    "default".to_string()
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

    pub fn parse_yaml(raw: &str) -> Result<Self, String> {
        let mut value =
            serde_yaml::from_str::<serde_yaml::Value>(raw).map_err(|e| e.to_string())?;
        let version = value
            .get("version")
            .and_then(serde_yaml::Value::as_u64)
            .unwrap_or_default();
        if matches!(version, 2 | 3) {
            if let Some(mapping) = value.as_mapping_mut() {
                mapping.insert(
                    serde_yaml::Value::String("version".into()),
                    serde_yaml::Value::Number(AppConfig::CURRENT_VERSION.into()),
                );
            }
        } else if version != u64::from(AppConfig::CURRENT_VERSION) {
            return Err(format!(
                "Unsupported configuration version {version}; expected {}",
                AppConfig::CURRENT_VERSION
            ));
        }
        let mut cfg = serde_yaml::from_value::<AppConfig>(value).map_err(|e| e.to_string())?;
        for settings in cfg.audio.vst_eq_by_plugin.values_mut() {
            *settings = settings.normalized(cfg.audio.sample_rate);
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject values that cannot be published unchanged to the UI and reloaded.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != Self::CURRENT_VERSION {
            return Err(format!(
                "Unsupported configuration version {}; expected {}",
                self.version,
                Self::CURRENT_VERSION
            ));
        }
        if !self.audio.gain_db.is_finite() || !self.ui.corner_radius.is_finite() {
            return Err("Audio gain and UI corner radius must be finite".into());
        }
        for (plugin, settings) in &self.audio.vst_eq_by_plugin {
            if *settings != settings.normalized(self.audio.sample_rate) {
                return Err(format!("Invalid EQ settings for {plugin}"));
            }
        }
        Ok(())
    }
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
    io: parking_lot::Mutex<()>,
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
        let store = Self {
            path,
            io: parking_lot::Mutex::new(()),
        };
        store.ensure_exists();
        store
    }

    pub fn load(&self) -> AppConfig {
        self.load_checked().unwrap_or_else(|error| {
            log::error!("{error}; using defaults for this session");
            AppConfig::default()
        })
    }

    pub fn load_checked(&self) -> Result<AppConfig, String> {
        self.load_checked_with(|path| fs::read_to_string(path))
    }

    fn load_checked_with(
        &self,
        read: impl FnOnce(&PathBuf) -> std::io::Result<String>,
    ) -> Result<AppConfig, String> {
        // Keep a migration's read and write in one transaction so a newer save
        // cannot commit between them and then be replaced by the old snapshot.
        let _io = self.io.lock();
        let raw = match read(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let cfg = AppConfig::default();
                self.save_locked(&cfg)?;
                return Ok(cfg);
            }
            Err(error) => {
                return Err(format!(
                    "Unable to read configuration {:?}: {error}",
                    self.path
                ));
            }
        };
        let raw_version = serde_yaml::from_str::<serde_yaml::Value>(&raw)
            .ok()
            .and_then(|v| v.get("version").and_then(serde_yaml::Value::as_u64))
            .unwrap_or_default();
        let cfg = AppConfig::parse_yaml(&raw).map_err(|error| {
            format!(
                "Invalid configuration preserved at {:?}: {error}",
                self.path
            )
        })?;
        if matches!(raw_version, 2 | 3) {
            self.save_locked(&cfg)
                .map_err(|error| format!("Unable to persist migrated configuration: {error}"))?;
        }
        Ok(cfg)
    }

    fn backup_if_unreadable(&self) -> Result<(), String> {
        let raw = match fs::read(&self.path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                let msg = format!("Failed to read existing config file for backup: {e}");
                log::warn!("{msg}");
                return Err(msg);
            }
        };
        if std::str::from_utf8(&raw)
            .ok()
            .is_some_and(|raw| AppConfig::parse_yaml(raw).is_ok())
        {
            return Ok(());
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut backup_path = self.path.with_extension(format!("yaml.invalid-{stamp}"));
        let mut backup = loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup_path)
            {
                Ok(file) => break file,
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && backup_path.is_file() =>
                {
                    backup_path = self
                        .path
                        .with_extension(format!("yaml.invalid-{stamp}-{}", uuid::Uuid::new_v4()));
                }
                Err(error) => {
                    return Err(format!("Failed to back up unreadable config file: {error}"))
                }
            }
        };
        let written = backup.write_all(&raw).and_then(|()| backup.sync_all());
        drop(backup);
        if let Err(error) = written {
            let _ = fs::remove_file(&backup_path);
            return Err(format!("Failed to back up unreadable config file: {error}"));
        }
        log::warn!(
            "Unreadable or invalid configuration backed up to {:?}",
            backup_path
        );
        Ok(())
    }

    pub fn save(&self, cfg: &AppConfig) -> Result<(), String> {
        let _io = self.io.lock();
        self.save_locked(cfg)
    }

    fn save_locked(&self, cfg: &AppConfig) -> Result<(), String> {
        cfg.validate()?;
        self.backup_if_unreadable()?;
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
                || cfg.rtp.thru_enabled
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
            realtime: Default::default(),
            audio_mmcss_enabled: None,
            audio_power_throttling_disabled: None,
            audio_limiter_enabled: None,
            x86_bridge_underruns: None,
            x86_bridge_overruns: None,
            x86_worker_alive: None,
        }
    }
}

fn default_game_mode() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_delayed_migration_cannot_overwrite_a_newer_successful_save() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut legacy = serde_yaml::to_value(super::AppConfig::default()).unwrap();
        legacy["version"] = 3.into();
        std::fs::write(&path, serde_yaml::to_string(&legacy).unwrap()).unwrap();
        let store = Arc::new(super::ConfigStore::with_path(path));
        let (read_tx, read_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reader = {
            let store = store.clone();
            std::thread::spawn(move || {
                store.load_checked_with(|path| {
                    let raw = std::fs::read_to_string(path)?;
                    read_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(raw)
                })
            })
        };
        read_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut next = super::AppConfig::default();
        next.osc.listen_port = 9200;
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let writer = {
            let store = store.clone();
            let next = next.clone();
            std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                done_tx.send(store.save(&next)).unwrap();
            })
        };
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let premature_save = done_rx.recv_timeout(Duration::from_millis(250));
        release_tx.send(()).unwrap();
        reader.join().unwrap().unwrap();
        writer.join().unwrap();
        assert_eq!(store.load_checked().unwrap(), next);
        assert!(matches!(
            premature_save,
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
    }

    #[test]
    fn checked_loading_reports_corruption_instead_of_accepting_defaults() {
        for broken in [b"version: 4\naudio: [".as_slice(), b"version: 4\nui: \xff"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.yaml");
            std::fs::write(&path, broken).unwrap();
            let store = super::ConfigStore::with_path(path.clone());
            assert!(store.load_checked().is_err());
            assert_eq!(std::fs::read(path).unwrap(), broken);
        }
    }

    #[test]
    fn checked_loading_preserves_a_configuration_missing_a_required_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut value = serde_yaml::to_value(super::AppConfig::default()).unwrap();
        value["midi"].as_mapping_mut().unwrap().remove("hotplug");
        let raw = serde_yaml::to_string(&value).unwrap();
        std::fs::write(&path, &raw).unwrap();
        let store = super::ConfigStore::with_path(path.clone());
        assert!(store.load_checked().is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), raw);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn checked_loading_reports_a_failed_migration_and_preserves_the_legacy_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut value = serde_yaml::to_value(super::AppConfig::default()).unwrap();
        value["version"] = 3.into();
        value["osc"]["listenPort"] = 9100.into();
        let raw = serde_yaml::to_string(&value).unwrap();
        std::fs::write(&path, &raw).unwrap();
        let store = super::ConfigStore::with_path(path.clone());
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        assert!(store.load_checked().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        drop(held);
        let migrated = store.load_checked().unwrap();
        assert_eq!(migrated.osc.listen_port, 9100);
        assert_eq!(migrated.version, 4);
        assert_eq!(store.load_checked().unwrap(), migrated);
    }

    #[test]
    fn oversized_versions_are_not_migrated_or_overwritten() {
        for version in [4_294_967_298u64, 4_294_967_299] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.yaml");
            let mut value = serde_yaml::to_value(super::AppConfig::default()).unwrap();
            value["version"] = version.into();
            let raw = serde_yaml::to_string(&value).unwrap();
            std::fs::write(&path, &raw).unwrap();
            super::ConfigStore::with_path(path.clone()).load();
            assert_eq!(std::fs::read_to_string(path).unwrap(), raw);
            assert!(super::AppConfig::parse_yaml(&raw).is_err());
        }
    }

    #[test]
    fn saving_preserves_an_existing_invalid_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Cover a second boundary without relying on timing or sleeps.
        let backups: Vec<_> = (stamp..=stamp + 2)
            .map(|stamp| {
                let backup = path.with_extension(format!("yaml.invalid-{stamp}"));
                std::fs::write(&backup, "first damaged configuration").unwrap();
                backup
            })
            .collect();
        std::fs::write(&path, "second: [truncated").unwrap();
        super::ConfigStore::with_path(path)
            .save(&super::AppConfig::default())
            .unwrap();
        for backup in backups {
            assert_eq!(
                std::fs::read_to_string(backup).unwrap(),
                "first damaged configuration"
            );
        }
    }

    #[test]
    fn saving_archives_non_utf8_bytes_before_replacing_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let broken = b"version: 4\nui: \xff\xfe";
        std::fs::write(&path, broken).unwrap();
        let store = super::ConfigStore::with_path(path.clone());
        let next = super::AppConfig::default();
        store.save(&next).unwrap();
        assert_eq!(store.load(), next);
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .find(|entry| entry.path() != path)
            .unwrap();
        assert_eq!(std::fs::read(backup.path()).unwrap(), broken);
    }

    #[test]
    fn saving_rejects_eq_values_that_would_change_on_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let store = super::ConfigStore::with_path(path.clone());
        let before = std::fs::read(&path).unwrap();
        let mut next = super::AppConfig::default();
        let mut eq = super::VstEqSettings::default();
        eq.mid_peak.q = 50.0;
        next.audio.vst_eq_by_plugin.insert("piano".into(), eq);
        assert!(store.save(&next).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn parsing_rejects_non_finite_gain_and_corner_radius() {
        for (section, field) in [("audio", "gainDb"), ("ui", "cornerRadius")] {
            for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut value = serde_yaml::to_value(super::AppConfig::default()).unwrap();
                value[section][field] = serde_yaml::to_value(non_finite).unwrap();
                let raw = serde_yaml::to_string(&value).unwrap();
                assert!(
                    super::AppConfig::parse_yaml(&raw).is_err(),
                    "{section}.{field}"
                );
            }
        }
    }

    #[test]
    fn appearance_preferences_preserve_old_configs_and_round_trip() {
        let mut legacy = serde_json::to_value(super::UiConfig::default()).unwrap();
        for key in ["accentColor", "lightBackground", "darkBackground"] {
            legacy.as_object_mut().unwrap().remove(key);
        }
        let mut config: super::UiConfig = serde_json::from_value(legacy).unwrap();
        assert_eq!(config.accent_color, "blue");
        assert_eq!(config.light_background, "default");
        assert_eq!(config.dark_background, "default");
        config.accent_color = "purple".into();
        config.light_background = "white".into();
        config.dark_background = "graphite".into();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let restored: super::UiConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(restored, config);
    }

    #[test]
    fn legacy_audio_config_enables_game_mode_without_changing_explicit_audio_format() {
        let mut legacy = serde_json::to_value(super::AudioConfig::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("gameMode");
        legacy["sampleRate"] = 48_000.into();
        legacy["bufferSize"] = 512.into();
        let config: super::AudioConfig = serde_json::from_value(legacy.clone()).unwrap();
        assert!(config.game_mode);
        assert_eq!((config.sample_rate, config.buffer_size), (48_000, 512));
        legacy["gameMode"] = false.into();
        legacy["bufferSize"] = 256.into();
        let explicit: super::AudioConfig = serde_json::from_value(legacy).unwrap();
        assert!(!explicit.game_mode);
        assert_eq!(explicit.buffer_size, 256);
    }

    #[test]
    fn legacy_audio_config_defaults_to_one_prepared_block_and_mmcss() {
        let mut legacy = serde_json::to_value(super::AudioConfig::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("renderAheadBlocks");
        legacy.as_object_mut().unwrap().remove("dspThreadPriority");
        let config: super::AudioConfig = serde_json::from_value(legacy).unwrap();
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["renderAheadBlocks"], 1);
        assert_eq!(value["dspThreadPriority"], "mmcss");
    }

    #[test]
    fn audio_config_rejects_unbounded_render_ahead() {
        let mut value = serde_json::to_value(super::AudioConfig::default()).unwrap();
        value["renderAheadBlocks"] = 3.into();
        assert!(serde_json::from_value::<super::AudioConfig>(value).is_err());
    }

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
    fn store_backs_up_invalid_config_on_save() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let broken = "invalid_yaml: [missing bracket";
        std::fs::write(&path, broken).expect("write broken");

        let store = ConfigStore::with_path(path.clone());
        let _ = store.load();

        let default_cfg = AppConfig::default();
        store.save(&default_cfg).expect("save valid");

        // The active config file now has the valid config
        let new_raw = std::fs::read_to_string(&path).expect("read new");
        assert!(AppConfig::parse_yaml(&new_raw).is_ok());

        // An invalid backup file was created containing the broken content
        let entries = std::fs::read_dir(dir.path()).expect("read_dir");
        let backup = entries.filter_map(|e| e.ok()).find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("config.yaml.invalid-")
        });
        assert!(backup.is_some(), "Backup file should exist");
        let backup_content = std::fs::read_to_string(backup.unwrap().path()).expect("read backup");
        assert_eq!(backup_content, broken);
    }

    #[test]
    fn store_save_fails_if_backup_of_invalid_config_fails() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let broken = "invalid_yaml: [missing bracket";
        std::fs::write(&path, broken).expect("write broken");

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup_path = path.with_extension(format!("yaml.invalid-{stamp}"));
        // Create a directory where the backup file would be written, causing fs::copy to fail
        std::fs::create_dir(&backup_path).expect("create dir collision");

        let store = ConfigStore::with_path(path.clone());
        let default_cfg = AppConfig::default();
        let result = store.save(&default_cfg);

        assert!(result.is_err(), "save should fail when backup fails");
        // Verify the original broken file is preserved and not overwritten
        let preserved = std::fs::read_to_string(&path).expect("read preserved");
        assert_eq!(preserved, broken);
    }

    #[test]
    fn v2_and_v3_audio_configs_migrate_without_losing_legacy_vst_path() {
        for version in [2, 3] {
            let dir = tempdir().expect("tempdir");
            let path = dir.path().join("config.yaml");
            let mut legacy = serde_yaml::to_string(&AppConfig::default()).expect("serialize");
            legacy = legacy
                .replace("version: 4", &format!("version: {version}"))
                .replace("  vstPluginId: null\n", "")
                .replace("  vstScanPaths: []\n", "")
                .replace("  vstPath: null", "  vstPath: C:\\Legacy\\Organ.dll");
            std::fs::write(&path, legacy).expect("write legacy");

            let migrated = ConfigStore::with_path(path.clone()).load();
            assert_eq!(migrated.version, AppConfig::CURRENT_VERSION);
            assert_eq!(migrated.audio.vst_plugin_id, None);
            assert!(migrated.audio.vst_scan_paths.is_empty());
            assert_eq!(
                migrated.audio.vst_path.as_deref(),
                Some("C:\\Legacy\\Organ.dll")
            );
            let persisted: AppConfig =
                serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(persisted, migrated);
        }
    }

    #[test]
    fn removed_v2_fields_are_ignored_and_removed_on_next_save() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let mut value = serde_yaml::to_value(AppConfig::default()).expect("serialize config");
        value["version"] = 2.into();
        value["audio"]["vstWorkerEnabled"] = false.into();
        value["ui"]["themePreset"] = "studio".into();
        value["ui"]["accent"] = "auto".into();
        value["ui"]["alwaysOnTop"] = false.into();
        value["logging"]["liveLogs"] = false.into();
        let legacy = serde_yaml::to_string(&value).unwrap();
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

    #[test]
    fn current_config_without_eq_profiles_loads_as_neutral() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let raw = serde_yaml::to_string(&AppConfig::default())
            .expect("serialize")
            .replace("  vstEqByPlugin: {}\n", "");
        std::fs::write(&path, raw).expect("write config without EQ");

        let loaded = ConfigStore::with_path(path).load();
        assert!(loaded.audio.vst_eq_by_plugin.is_empty());
    }

    #[test]
    fn eq_profiles_round_trip_independently_by_plugin() {
        let mut config = AppConfig::default();
        let piano = VstEqSettings {
            enabled: true,
            low_shelf: EqShelfSettings {
                gain_db: -6.0,
                ..VstEqSettings::default().low_shelf
            },
            ..VstEqSettings::default()
        };
        config
            .audio
            .vst_eq_by_plugin
            .insert("piano".into(), piano.clone());
        config.audio.vst_eq_by_plugin.insert(
            "organ".into(),
            VstEqSettings {
                enabled: true,
                mid_peak: EqPeakSettings {
                    gain_db: 3.0,
                    ..VstEqSettings::default().mid_peak
                },
                ..VstEqSettings::default()
            },
        );

        let raw = serde_yaml::to_string(&config).expect("serialize EQ profiles");
        let loaded: AppConfig = serde_yaml::from_str(&raw).expect("deserialize EQ profiles");
        assert_eq!(loaded.audio.vst_eq_by_plugin["piano"], piano);
        assert_eq!(loaded.audio.vst_eq_by_plugin["organ"].mid_peak.gain_db, 3.0);
    }
}
