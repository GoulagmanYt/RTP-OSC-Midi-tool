use std::{io, sync::OnceLock, time::Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{audio::AudioSettings, config::VstEqSettings, types::VstParameter};

// Version 9 carries game-mode health and ingress timestamps.
pub const CONTROL_PROTOCOL_VERSION: u32 = 9;
pub const HOST_ABI_VERSION: u32 = crate::types::VST_HOST_ABI_VERSION;
// A normal 8,192-parameter catalogue is ~1.14 MB of JSON, exceeding 1 MiB.
// Allow 8 MiB for longer names/units while keeping allocations on both ends
// bounded. Unbounded third-party metadata still gets the error-reply fallback.
pub const MAX_CONTROL_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_MIDI_BYTES: usize = 3;
pub const MIDI_FRAME_BYTES: usize = 8 + 8 + 8 + 1 + MAX_MIDI_BYTES;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerAudioStatus {
    pub backend: String,
    pub device: String,
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub requested_buffer_size: u32,
    pub stream_buffer_size: u32,
    pub plugin_latency_samples: u32,
    pub bridge_latency_samples: u32,
    pub hosting_mode: String,
    pub vst_midi_compatible: bool,
    pub limiter_enabled: bool,
    pub mmcss_enabled: bool,
    pub power_throttling_disabled: bool,
    pub class_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkerMetrics {
    #[serde(default)]
    pub render_ahead_samples: u32,
    #[serde(default)]
    pub render_queue_frames: u64,
    #[serde(default)]
    pub render_underflows: u64,
    #[serde(default)]
    pub render_missing_frames: u64,
    #[serde(default)]
    pub render_reset_silence_frames: u64,
    #[serde(default)]
    pub render_block_max_us: u32,
    #[serde(default)]
    pub render_over_budget: u64,
    #[serde(default)]
    pub callback_gap_count: u64,
    /// Applied worker state. Older workers omit it, preserving the Ready value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limiter_enabled: Option<bool>,
    #[serde(default)]
    pub midi_transport_failed: bool,
    #[serde(default)]
    pub pipe_wait_max_us: u64,
    #[serde(default)]
    pub audio_wait_max_us: u64,
    #[serde(default)]
    pub midi_io_threads: Vec<crate::realtime::ThreadDiagnostics>,
    #[serde(default)]
    pub midi_delivery_max_us: u64,
    #[serde(default)]
    pub midi_delivery_p99_us: u64,
    #[serde(default)]
    pub callback_count: u64,
    #[serde(default)]
    pub callback_interval_max_us: u64,
    /// None until a callback publishes tuning, or when reading an older worker.
    #[serde(default)]
    pub audio_mmcss_owned: Option<bool>,
    /// None for driver-owned registration: the host cannot inspect its AVRT priority.
    #[serde(default)]
    pub audio_mmcss_critical: Option<bool>,
    #[serde(default)]
    pub game_mode: bool,
    #[serde(default)]
    pub maintenance: bool,
    #[serde(default)]
    pub deferred_save: bool,
    #[serde(default)]
    pub deferred_editor: Option<bool>,
    #[serde(default)]
    pub editor_open: bool,
    pub audio_peak_l: f32,
    pub audio_peak_r: f32,
    pub audio_xruns: u32,
    pub stream_recovery_requests: u32,
    pub stream_route_changes: u32,
    pub backend_latency_us: u64,
    pub audio_midi_drops: u32,
    pub audio_lock_misses: u32,
    pub audio_emergency_resets: u32,
    pub callback_last_us: u32,
    pub callback_max_us: u32,
    pub callback_over_budget_count: u32,
    pub consecutive_deadline_misses: u32,
    pub dsp_process_last_us: u32,
    pub dsp_process_p95_us: u32,
    pub dsp_process_p99_us: u32,
    pub dsp_process_max_us: u32,
    pub midi_queue_depth: u32,
    pub midi_queue_max_depth: u32,
    pub midi_oldest_us: u64,
    pub x86_bridge_underruns: u32,
    pub x86_bridge_overruns: u32,
    pub x86_worker_alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ControlMessage {
    Hello {
        protocol_version: u32,
        host_abi_version: u32,
        session_id: String,
        token: String,
        worker_pid: u32,
    },
    Load {
        request_id: u64,
        settings: AudioSettings,
        class_uid: Option<String>,
        warmup_ms: u32,
    },
    OpenEditor {
        request_id: u64,
    },
    CloseEditor {
        request_id: u64,
    },
    ListParameters {
        request_id: u64,
    },
    SetParameter {
        request_id: u64,
        index: usize,
        value: f32,
    },
    SetGain {
        request_id: u64,
        gain_db: f32,
    },
    SetLimiter {
        request_id: u64,
        enabled: bool,
    },
    SetEq {
        request_id: u64,
        settings: VstEqSettings,
    },
    SetGameMode {
        request_id: u64,
        enabled: bool,
    },
    SaveState {
        request_id: u64,
    },
    Panic {
        request_id: u64,
    },
    Stop {
        request_id: u64,
    },
    Ping {
        sequence: u64,
    },
    Pong {
        sequence: u64,
    },
    Ready {
        request_id: u64,
        status: WorkerAudioStatus,
    },
    Parameters {
        request_id: u64,
        parameters: Vec<VstParameter>,
    },
    Ack {
        request_id: u64,
    },
    Error {
        request_id: Option<u64>,
        code: String,
        message: String,
    },
    Heartbeat {
        sequence: u64,
        monotonic_qpc: u64,
        metrics: WorkerMetrics,
    },
    EditorHidden,
    Stopped {
        request_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiWireFrame {
    /// Local admission epoch, never transmitted: the in-band reset carries it across processes.
    pub generation: u64,
    pub pipe_qpc: u64,
    pub sequence: u64,
    pub monotonic_qpc: u64,
    pub len: u8,
    pub data: [u8; MAX_MIDI_BYTES],
}

impl MidiWireFrame {
    pub fn new(sequence: u64, monotonic_qpc: u64, bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_MIDI_BYTES {
            return None;
        }
        let mut data = [0; MAX_MIDI_BYTES];
        data[..bytes.len()].copy_from_slice(bytes);
        if data[0] == 0xF0 || !crate::midi::normalize_message(&mut data[..bytes.len()]) {
            return None;
        }
        Some(Self {
            generation: 0,
            pipe_qpc: 0,
            sequence,
            monotonic_qpc,
            len: bytes.len() as u8,
            data,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    pub fn encode(self) -> [u8; MIDI_FRAME_BYTES] {
        let mut encoded = [0u8; MIDI_FRAME_BYTES];
        encoded[0..8].copy_from_slice(&self.sequence.to_le_bytes());
        encoded[8..16].copy_from_slice(&self.monotonic_qpc.to_le_bytes());
        encoded[16..24].copy_from_slice(&self.pipe_qpc.to_le_bytes());
        encoded[24] = self.len;
        encoded[25..28].copy_from_slice(&self.data);
        encoded
    }

    pub fn decode(encoded: [u8; MIDI_FRAME_BYTES]) -> io::Result<Self> {
        let len = encoded[24];
        if len == 0 || len as usize > MAX_MIDI_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid MIDI frame length",
            ));
        }
        let mut sequence = [0u8; 8];
        sequence.copy_from_slice(&encoded[0..8]);
        let mut qpc = [0u8; 8];
        qpc.copy_from_slice(&encoded[8..16]);
        let mut data = [0u8; MAX_MIDI_BYTES];
        data.copy_from_slice(&encoded[25..28]);
        if data[0] == 0xF0
            || data[usize::from(len)..].iter().any(|byte| *byte != 0)
            || !crate::midi::normalize_message(&mut data[..usize::from(len)])
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid MIDI frame contents",
            ));
        }
        Ok(Self {
            generation: 0,
            pipe_qpc: u64::from_le_bytes(encoded[16..24].try_into().unwrap()),
            sequence: u64::from_le_bytes(sequence),
            monotonic_qpc: u64::from_le_bytes(qpc),
            len,
            data,
        })
    }
}

pub async fn write_control_frame<W>(writer: &mut W, message: &ControlMessage) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let payload = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if payload.len() > MAX_CONTROL_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control frame exceeds size limit",
        ));
    }
    writer.write_u32_le(payload.len() as u32).await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

/// Worker responses may contain unbounded third-party metadata.
pub async fn write_control_response<W>(writer: &mut W, message: &ControlMessage) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    match write_control_frame(writer, message).await {
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            let request_id = match message {
                ControlMessage::Parameters { request_id, .. }
                | ControlMessage::Ready { request_id, .. }
                | ControlMessage::Ack { request_id }
                | ControlMessage::Stopped { request_id } => Some(*request_id),
                ControlMessage::Error { request_id, .. } => *request_id,
                _ => None,
            };
            write_control_frame(
                writer,
                &ControlMessage::Error {
                    request_id,
                    code: "worker.frame-too-large".into(),
                    message: "Response exceeds IPC frame limit".into(),
                },
            )
            .await
        }
        result => result,
    }
}

pub async fn read_control_frame<R>(reader: &mut R) -> io::Result<ControlMessage>
where
    R: AsyncRead + Unpin,
{
    let len = reader.read_u32_le().await? as usize;
    if len == 0 || len > MAX_CONTROL_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid control frame length",
        ));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub async fn write_midi_frame<W>(writer: &mut W, frame: MidiWireFrame) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(&frame.encode()).await
}

pub async fn read_midi_frame<R>(reader: &mut R) -> io::Result<MidiWireFrame>
where
    R: AsyncRead + Unpin,
{
    let mut encoded = [0u8; MIDI_FRAME_BYTES];
    reader.read_exact(&mut encoded).await?;
    MidiWireFrame::decode(encoded)
}

#[cfg(target_os = "windows")]
pub fn monotonic_qpc() -> u64 {
    use windows::Win32::System::Performance::QueryPerformanceCounter;

    let mut value = 0i64;
    // SAFETY: QueryPerformanceCounter writes one i64 to a valid local pointer.
    if unsafe { QueryPerformanceCounter(&mut value) }.is_ok() {
        value.max(0) as u64
    } else {
        fallback_monotonic_ticks()
    }
}

#[cfg(target_os = "windows")]
fn qpc_frequency() -> u64 {
    use windows::Win32::System::Performance::QueryPerformanceFrequency;
    static FREQUENCY: OnceLock<u64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        let mut value = 0i64;
        // SAFETY: QueryPerformanceFrequency writes one i64 to a valid local pointer.
        if unsafe { QueryPerformanceFrequency(&mut value) }.is_ok() {
            value.max(1) as u64
        } else {
            1_000_000_000
        }
    })
}

#[cfg(not(target_os = "windows"))]
pub fn monotonic_qpc() -> u64 {
    fallback_monotonic_ticks()
}

#[cfg(not(target_os = "windows"))]
fn qpc_frequency() -> u64 {
    1_000_000_000
}

pub fn qpc_at(instant: Instant) -> u64 {
    let elapsed = instant.elapsed().as_nanos();
    monotonic_qpc().saturating_sub(
        ((elapsed * u128::from(qpc_frequency())) / 1_000_000_000).min(u64::MAX as u128) as u64,
    )
}

pub fn qpc_elapsed_us(earlier: u64, later: u64) -> u64 {
    let ticks = later.saturating_sub(earlier) as u128;
    ((ticks * 1_000_000) / u128::from(qpc_frequency())).min(u64::MAX as u128) as u64
}

fn fallback_monotonic_ticks() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_heartbeat_state_is_optional_for_older_workers() {
        let legacy = serde_json::to_value(WorkerMetrics::default()).unwrap();
        assert!(legacy.get("limiterEnabled").is_none());
        assert_eq!(
            serde_json::from_value::<WorkerMetrics>(legacy.clone())
                .unwrap()
                .limiter_enabled,
            None
        );
        let mut current = legacy;
        current["limiterEnabled"] = serde_json::Value::Bool(true);
        assert_eq!(
            serde_json::from_value::<WorkerMetrics>(current)
                .unwrap()
                .limiter_enabled,
            Some(true)
        );
    }

    #[test]
    fn midi_wire_frame_round_trip_is_fixed_and_bounded() {
        let frame = MidiWireFrame::new(42, 123_456, &[0x90, 60, 100]).expect("MIDI frame");
        assert_eq!(MidiWireFrame::decode(frame.encode()).unwrap(), frame);
        assert_eq!(frame.bytes(), &[0x90, 60, 100]);
        assert!(MidiWireFrame::new(1, 1, &[]).is_none());
        assert!(MidiWireFrame::new(1, 1, &[1, 2, 3, 4]).is_none());
        assert!(MidiWireFrame::new(1, 1, &[0x90, 60]).is_none());
        assert!(MidiWireFrame::new(1, 1, &[0x90, 128, 1]).is_none());
        assert!(MidiWireFrame::new(1, 1, &[0xF0, 1, 0xF7]).is_none());
        let zero_velocity = MidiWireFrame::new(1, 1, &[0x91, 60, 0]).unwrap();
        assert_eq!(zero_velocity.bytes(), &[0x81, 60, 0]);
        let mut truncated = frame.encode();
        truncated[24] = 2;
        assert!(MidiWireFrame::decode(truncated).is_err());
        let mut bad_data = frame.encode();
        bad_data[26] = 128;
        assert!(MidiWireFrame::decode(bad_data).is_err());
    }

    #[test]
    fn qpc_elapsed_conversion_is_monotonic_and_bounded() {
        let start = monotonic_qpc();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let elapsed = qpc_elapsed_us(start, monotonic_qpc());
        assert!(elapsed >= 1_000);
        assert_eq!(qpc_elapsed_us(u64::MAX, 0), 0);
    }

    #[tokio::test]
    async fn control_frame_round_trip_is_length_prefixed_json() {
        let message = ControlMessage::Ping { sequence: 99 };
        let (mut client, mut server) = tokio::io::duplex(4096);
        let write = write_control_frame(&mut client, &message);
        let read = read_control_frame(&mut server);
        let (write_result, read_result) = tokio::join!(write, read);
        write_result.unwrap();
        assert_eq!(read_result.unwrap(), message);
    }

    #[tokio::test]
    async fn eq_control_frame_round_trip_preserves_settings() {
        let settings = VstEqSettings {
            enabled: true,
            low_shelf: crate::config::EqShelfSettings {
                gain_db: -6.5,
                ..VstEqSettings::default().low_shelf
            },
            ..VstEqSettings::default()
        };
        let message = ControlMessage::SetEq {
            request_id: 7,
            settings,
        };
        let (mut client, mut server) = tokio::io::duplex(4096);
        let (write_result, read_result) = tokio::join!(
            write_control_frame(&mut client, &message),
            read_control_frame(&mut server)
        );
        write_result.unwrap();
        assert_eq!(read_result.unwrap(), message);
    }

    #[tokio::test]
    async fn complete_frames_survive_bytewise_delivery_and_unrelated_wakeups() {
        let (mut writer, reader) = tokio::io::duplex(1);
        let first = MidiWireFrame::new(1, 10, &[0x90, 60, 100]).unwrap();
        let second = MidiWireFrame::new(2, 20, &[0x80, 60, 0]).unwrap();
        let receiving = tokio::spawn(async move {
            let mut reader = reader;
            (
                read_midi_frame(&mut reader).await.unwrap(),
                read_midi_frame(&mut reader).await.unwrap(),
            )
        });
        for frame in [first, second] {
            for byte in frame.encode() {
                writer.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        }
        assert_eq!(receiving.await.unwrap(), (first, second));
    }

    #[tokio::test]
    async fn oversized_control_frame_is_rejected_before_allocation() {
        let (mut client, mut server) = tokio::io::duplex(16);
        client
            .write_u32_le((MAX_CONTROL_FRAME_BYTES + 1) as u32)
            .await
            .unwrap();
        let error = read_control_frame(&mut server).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
