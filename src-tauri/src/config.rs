use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::types::BridgeStatus;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtpRemoteEntry {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingMapping {
    pub from: u8,
    pub to: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingAssignment {
    pub source: String,
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub osc_target_ip: String,
    pub osc_target_port: u16,
    pub osc_enabled: bool,
    pub channel_filter: Option<u8>,
    pub midi_in: Option<String>,
    pub midi_out: Option<String>,
    pub midi_thru: bool,
    pub verbose: bool,
    pub log_osc: bool,
    pub log_rtp: bool,
    pub live_logs: bool,
    pub rtp_enabled: bool,
    pub rtp_session_name: String,
    pub rtp_port: u16,
    pub rtp_remote_enabled: bool,
    pub rtp_remote_host: String,
    pub rtp_remote_port: u16,
    pub rtp_remotes: Vec<RtpRemoteEntry>,
    pub routing_profiles: Vec<RoutingProfile>,
    pub routing_assignments: Vec<RoutingAssignment>,
    pub hotplug: bool,
    pub theme_preset: String,
    pub theme: Theme,
    pub theme_palette: String,
    pub ui_accent: String,
    pub corner_radius: f32,
    pub ui_scale: f32,
    pub content_padding: f32,
    pub sidebar_width: f32,
    pub surface_opacity: f32,
    pub card_opacity: f32,
    pub aurora_intensity: f32,
    pub grain_intensity: f32,
    pub reduce_motion: bool,
    pub auto_start: bool,
    pub always_on_top: bool,
    pub audio_enabled: bool,
    pub audio_backend: Option<String>,
    pub audio_device: Option<String>,
    pub audio_sample_rate: u32,
    pub audio_buffer_size: u32,
    pub audio_gain_db: f32,
    pub audio_limiter_enabled: bool,
    pub log_all_to_file: bool,
    pub logs_enabled: bool,
    pub vst_path: Option<String>,
    pub avatar_path: Option<String>,
    pub avatar_offset_x: f32,
    pub avatar_offset_y: f32,
    pub avatar_scale: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            osc_target_ip: "127.0.0.1".to_string(),
            osc_target_port: 9000,
            osc_enabled: true,
            channel_filter: None,
            midi_in: None,
            midi_out: None,
            midi_thru: true,
            verbose: false,
            log_osc: true,
            log_rtp: false,
            live_logs: false,
            rtp_enabled: true,
            rtp_session_name: "OSCMidi".to_string(),
            rtp_port: 5004,
            rtp_remote_enabled: false,
            rtp_remote_host: "".to_string(),
            rtp_remote_port: 5004,
            rtp_remotes: Vec::new(),
            routing_profiles: Vec::new(),
            routing_assignments: Vec::new(),
            hotplug: true,
            theme_preset: "studio".to_string(),
            theme: Theme::Light,
            theme_palette: "light".to_string(),
            ui_accent: "auto".to_string(),
            corner_radius: 12.0,
            ui_scale: 1.0,
            content_padding: 24.0,
            sidebar_width: 256.0,
            surface_opacity: 0.5,
            card_opacity: 1.0,
            aurora_intensity: 0.75,
            grain_intensity: 0.16,
            reduce_motion: false,
            auto_start: false,
            always_on_top: false,
            audio_enabled: true,
            audio_backend: Some("auto".to_string()),
            audio_device: None,
            audio_sample_rate: 48_000,
            audio_buffer_size: 256,
            audio_gain_db: 0.0,
            audio_limiter_enabled: false,
            log_all_to_file: false,
            logs_enabled: true,
            vst_path: None,
            avatar_path: None,
            avatar_offset_x: 50.0,
            avatar_offset_y: 50.0,
            avatar_scale: 1.0,
        }
    }
}

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
        let path = cfg_dir.join("config.yaml");
        let store = Self { path };
        store.ensure_exists();
        store
    }

    pub fn load(&self) -> Config {
        match fs::read_to_string(&self.path) {
            Ok(raw) => {
                let cfg = serde_yaml::from_str(&raw).unwrap_or_else(|_| self.write_default());
                let (cfg, changed) = normalize_config(cfg);
                if changed {
                    let _ = self.save(&cfg);
                }
                cfg
            }
            Err(_) => self.write_default(),
        }
    }

    pub fn save(&self, cfg: &Config) -> Result<(), String> {
        let raw = serde_yaml::to_string(cfg).map_err(|e| e.to_string())?;
        fs::write(&self.path, raw).map_err(|e| e.to_string())
    }

    #[allow(dead_code)]
    pub fn reset_to_default(&self) -> Result<Config, String> {
        let cfg = Config::default();
        self.save(&cfg)?;
        Ok(cfg)
    }

    fn ensure_exists(&self) {
        if !self.path.exists() {
            let _ = self.save(&Config::default());
        }
    }

    fn write_default(&self) -> Config {
        let default_cfg = Config::default();
        let _ = self.save(&default_cfg);
        default_cfg
    }
}

fn normalize_config(mut cfg: Config) -> (Config, bool) {
    let mut changed = false;
    if cfg.rtp_remotes.is_empty() && !cfg.rtp_remote_host.trim().is_empty() {
        let host = cfg.rtp_remote_host.trim().to_string();
        let port = if cfg.rtp_remote_port == 0 {
            5004
        } else {
            cfg.rtp_remote_port
        };
        cfg.rtp_remotes.push(RtpRemoteEntry {
            id: "legacy".to_string(),
            name: host.clone(),
            host,
            port,
            auto_connect: cfg.rtp_remote_enabled,
        });
        changed = true;
    }
    (cfg, changed)
}

impl From<&Config> for BridgeStatus {
    fn from(cfg: &Config) -> Self {
        Self {
            running: false,
            midi_in: cfg.midi_in.clone(),
            midi_out: cfg.midi_out.clone(),
            osc_target: format!("{}:{}", cfg.osc_target_ip, cfg.osc_target_port),
            rtp_active: cfg.rtp_enabled
                || cfg.rtp_remote_enabled
                || cfg
                    .midi_in
                    .as_ref()
                    .map(|s| s == RTP_VIRTUAL_INPUT)
                    .unwrap_or(false),
            rtp_bound_port: None,
            last_error: None,
            vst_loaded: false,
            audio_running: false,
            audio_latency_ms: None,
            audio_backend: None,
            audio_device: None,
            audio_sample_rate: None,
            audio_buffer_size: None,
            audio_requested_buffer_size: None,
            audio_stream_buffer_size: None,
            audio_buffer_mismatch: None,
            vst_midi_compatible: None,
            audio_xruns: None,
            audio_limiter_enabled: None,
        }
    }
}
