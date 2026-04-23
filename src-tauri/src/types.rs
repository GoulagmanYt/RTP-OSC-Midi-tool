use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub level: String,
    pub message: String,
    pub timestamp: String,
}

impl LogEntry {
    pub fn new(level: &str, message: impl Into<String>) -> Self {
        let now: DateTime<Utc> = Utc::now();
        Self {
            level: level.to_string(),
            message: message.into(),
            timestamp: now.format("%H:%M:%S%.3f").to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub config_dir: String,
    pub log_file: String,
    pub log_dir: String,
}

impl AppPaths {
    pub fn new() -> Result<Self, String> {
        let dirs = directories::ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
            .ok_or("Impossible de determiner le dossier de configuration")?;
        let config_dir = dirs.config_dir().to_path_buf();
        let log_dir = config_dir.join("logs");
        let log_file = log_dir.join("app.log");

        Ok(Self {
            config_dir: config_dir.to_string_lossy().to_string(),
            log_file: log_file.to_string_lossy().to_string(),
            log_dir: log_dir.to_string_lossy().to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub running: bool,
    pub midi_input: Option<String>,
    pub midi_output: Option<String>,
    pub osc_target: String,
    pub rtp_active: bool,
    pub rtp_bound_port: Option<u16>,
    pub last_error: Option<String>,
    pub audio_running: bool,
    pub audio_latency_ms: Option<f32>,
    pub audio_backend: Option<String>,
    pub audio_device: Option<String>,
    pub audio_sample_rate: Option<u32>,
    pub audio_buffer_size: Option<u32>,
    pub audio_requested_buffer_size: Option<u32>,
    pub audio_stream_buffer_size: Option<u32>,
    pub audio_buffer_mismatch: Option<bool>,
    pub vst_loaded: bool,
    pub vst_midi_compatible: Option<bool>,
    pub audio_xruns: Option<u32>,
    pub audio_limiter_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetrics {
    pub audio_peak_l: Option<f32>,
    pub audio_peak_r: Option<f32>,
    pub audio_latency_ms: Option<f32>,
    pub audio_xruns: Option<u32>,
    pub audio_midi_drops: Option<u32>,
    pub audio_lock_misses: Option<u32>,
    pub audio_emergency_resets: Option<u32>,
    pub midi_messages_per_sec: u32,
    pub osc_messages_per_sec: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RtpSessionInfo {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtpParticipantInfo {
    pub name: String,
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiActivitySnapshot {
    pub source: String,
    pub messages_per_sec: u32,
    pub last_note: Option<u8>,
    pub last_channel: Option<u8>,
    pub last_seen_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiNoteEvent {
    pub source: String,
    pub note: u8,
    pub channel: u8,
    pub pressed: bool,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub midi_in_ok: bool,
    pub midi_out_ok: bool,
    pub audio_backend_ok: bool,
    pub rtp_port_ok: bool,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VstPluginEntry {
    pub name: String,
    pub path: String,
    pub format: String,
    pub kind: String,
    pub architecture: String,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub midi_compatible: Option<bool>,
    pub has_editor: bool,
    pub channel_layout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VstParameter {
    pub index: usize,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: String,
    pub value: f32,
}

pub type LogEvent = LogEntry;
pub type BridgeStatus = RuntimeStatus;
pub type BridgeMetrics = RuntimeMetrics;
pub type MidiActivityInfo = MidiActivitySnapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentMetrics {
    pub dropped: u32,
    pub xruns: u32,
    pub latency_ms: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StressTestResult {
    pub sent_notes: u32,
    pub elapsed_ms: u64,
    // Global totals
    pub dropped_notes: u32,
    pub xruns: u32,
    // Per-segment metrics (optional based on test mode)
    pub segment_rtp: Option<SegmentMetrics>,
    pub segment_bridge: Option<SegmentMetrics>,
    pub segment_audio: Option<SegmentMetrics>,
    // End-to-end tracking
    pub received_notes: Option<u32>,
}

