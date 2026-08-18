use std::{io, sync::OnceLock, time::Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{audio::AudioSettings, types::VstParameter};

pub const CONTROL_PROTOCOL_VERSION: u32 = 6;
pub const HOST_ABI_VERSION: u32 = crate::types::VST_HOST_ABI_VERSION;
pub const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_MIDI_BYTES: usize = 3;
const MIDI_FRAME_BYTES: usize = 8 + 8 + 1 + MAX_MIDI_BYTES;

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
        Some(Self {
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
        encoded[16] = self.len;
        encoded[17..20].copy_from_slice(&self.data);
        encoded
    }

    pub fn decode(encoded: [u8; MIDI_FRAME_BYTES]) -> io::Result<Self> {
        let len = encoded[16];
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
        data.copy_from_slice(&encoded[17..20]);
        Ok(Self {
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
    fn midi_wire_frame_round_trip_is_fixed_and_bounded() {
        let frame = MidiWireFrame::new(42, 123_456, &[0x90, 60, 100]).expect("MIDI frame");
        assert_eq!(MidiWireFrame::decode(frame.encode()).unwrap(), frame);
        assert_eq!(frame.bytes(), &[0x90, 60, 100]);
        assert!(MidiWireFrame::new(1, 1, &[]).is_none());
        assert!(MidiWireFrame::new(1, 1, &[1, 2, 3, 4]).is_none());
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
