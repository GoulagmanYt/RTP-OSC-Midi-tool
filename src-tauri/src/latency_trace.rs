//! Opt-in bounded diagnostic records. No file I/O, locks or allocation on
//! MIDI/audio threads. This module is absent from normal application builds.
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    LazyLock,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stage {
    RtpReceived,
    RtpDispatched,
    Routing,
    OscSent,
    PipeWriteStarted,
    WorkerReceived,
    DspDequeued,
    MidiScheduled,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MidiTrace {
    pub stage: Stage,
    pub data: [u8; 3],
    pub ingress_qpc: u64,
    pub at_qpc: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_wait_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_offset_frames: Option<u32>,
}

struct TraceBuffer {
    sender: Sender<MidiTrace>,
    receiver: Receiver<MidiTrace>,
    dropped: AtomicU64,
}

impl TraceBuffer {
    fn new(capacity: usize) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(capacity);
        Self {
            sender,
            receiver,
            dropped: AtomicU64::new(0),
        }
    }

    fn push(&self, record: MidiTrace) {
        if self.sender.try_send(record).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static BUFFER: LazyLock<TraceBuffer> = LazyLock::new(|| TraceBuffer::new(65_536));

/// Call before starting MIDI/audio threads. A new session discards old records.
pub fn start() {
    ENABLED.store(false, Ordering::Release);
    while BUFFER.receiver.try_recv().is_ok() {}
    BUFFER.dropped.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Release);
}

pub fn record(stage: Stage, bytes: &[u8], ingress_qpc: u64, at_qpc: u64) {
    if !ENABLED.load(Ordering::Relaxed)
        || bytes.len() != 3
        || !matches!(bytes[0] & 0xf0, 0x80 | 0x90)
    {
        return;
    }
    BUFFER.push(MidiTrace {
        stage,
        data: [bytes[0], bytes[1], bytes[2]],
        ingress_qpc,
        at_qpc,
        planned_wait_us: None,
        sample_offset_frames: None,
    });
}

pub fn record_rtp(bytes: &[u8], ingress_qpc: u64, received_qpc: u64, planned_wait_us: u64) {
    if !ENABLED.load(Ordering::Relaxed)
        || bytes.len() != 3
        || !matches!(bytes[0] & 0xf0, 0x80 | 0x90)
    {
        return;
    }
    BUFFER.push(MidiTrace {
        stage: Stage::RtpReceived,
        data: [bytes[0], bytes[1], bytes[2]],
        ingress_qpc,
        at_qpc: received_qpc,
        planned_wait_us: Some(planned_wait_us),
        sample_offset_frames: None,
    });
}

/// Import worker records on the ordinary control reader, never on the audio thread.
pub fn import(records: &[MidiTrace]) {
    if ENABLED.load(Ordering::Relaxed) {
        for record in records {
            BUFFER.push(*record);
        }
    }
}

pub fn drain() -> Vec<MidiTrace> {
    BUFFER.receiver.try_iter().collect()
}
pub fn dropped() -> u64 {
    BUFFER.dropped.load(Ordering::Relaxed)
}

/// Observed successful VST submission and its sample offset; not DAC onset.
pub fn record_sample_offset(bytes: &[u8], ingress_qpc: u64, sample_offset_frames: u32) {
    if !ENABLED.load(Ordering::Relaxed)
        || bytes.len() != 3
        || !matches!(bytes[0] & 0xf0, 0x80 | 0x90)
    {
        return;
    }
    BUFFER.push(MidiTrace {
        stage: Stage::MidiScheduled,
        data: [bytes[0], bytes[1], bytes[2]],
        ingress_qpc,
        at_qpc: crate::vst_worker::protocol::monotonic_qpc(),
        planned_wait_us: None,
        sample_offset_frames: Some(sample_offset_frames),
    });
}

pub struct LatencyTraceGuard {
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for LatencyTraceGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        ENABLED.store(false, Ordering::Release);
    }
}

pub fn default_trace_log_path() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("OSCMIDI_LATENCY_LOG_FILE") {
        return std::path::PathBuf::from(path);
    }
    let base_dir = if let Ok(dir) = std::env::var("OSCMIDI_LATENCY_LOG_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        let local_artifacts = std::path::PathBuf::from("artifacts").join("logs");
        if local_artifacts.is_dir() {
            local_artifacts
        } else {
            crate::app_paths::config_dir()
                .map(|dir| dir.join("logs"))
                .unwrap_or_else(|| {
                    std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                })
        }
    };
    base_dir.join("latency-trace.jsonl")
}

#[must_use = "keep the latency trace guard alive until shutdown"]
pub fn start_file_recorder() -> LatencyTraceGuard {
    start();
    let log_path = default_trace_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => {
            crate::logger::background_log(
                "warn",
                format!(
                    "Failed to open latency trace file {}: {}",
                    log_path.display(),
                    e
                ),
            );
            return LatencyTraceGuard {
                stop: None,
                thread: None,
            };
        }
    };

    crate::logger::background_log(
        "info",
        format!("Latency trace recording active: {}", log_path.display()),
    );

    let (stop, rx) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("latency-trace-flusher".into())
        .spawn(move || {
            use std::io::Write;
            let mut file = std::io::BufWriter::new(file);

            let qpc_freq = crate::vst_worker::protocol::qpc_frequency();
            let header = serde_json::json!({
                "header": {
                    "qpc_frequency": qpc_freq,
                    "version": env!("CARGO_PKG_VERSION"),
                    "started_at": chrono::Utc::now().to_rfc3339(),
                }
            });
            let _ = writeln!(file, "{}", header);
            let _ = file.flush();

            loop {
                let res = rx.recv_timeout(std::time::Duration::from_millis(500));
                flush_records(&mut file);
                if !matches!(res, Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
                    break;
                }
            }
            flush_records(&mut file);
            let _ = file.flush();
        })
        .ok();

    LatencyTraceGuard {
        stop: Some(stop),
        thread,
    }
}

fn flush_records<W: std::io::Write>(writer: &mut W) {
    for record in drain() {
        if let Ok(json) = serde_json::to_string(&record) {
            let _ = writeln!(writer, "{}", json);
        }
    }
    let _ = writer.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_audio_health_does_not_consume_diagnostic_records() {
        let _lock = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        start();
        record(Stage::Routing, &[0x90, 47, 99], u64::MAX - 1, u64::MAX);
        let engine = crate::audio::AudioEngine::new();
        let _ = engine.realtime_metrics();
        let _ = engine.realtime_metrics();
        let matching = drain()
            .into_iter()
            .filter(|r| r.ingress_qpc == u64::MAX - 1)
            .count();
        ENABLED.store(false, Ordering::Release);
        assert_eq!(
            matching, 1,
            "ordinary health reads must not steal the heartbeat's trace payload"
        );
    }

    #[test]
    fn bounded_trace_preserves_identity_and_counts_overflow_without_blocking() {
        let buffer = TraceBuffer::new(2);
        for at_qpc in [110, 120, 130] {
            buffer.push(MidiTrace {
                stage: Stage::Routing,
                data: [0x90, 60, 100],
                ingress_qpc: 100,
                at_qpc,
                planned_wait_us: None,
                sample_offset_frames: None,
            });
        }
        let records: Vec<_> = buffer.receiver.try_iter().collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].ingress_qpc, 100);
        assert_eq!(records[0].data, [0x90, 60, 100]);
        assert_eq!(records[0].at_qpc, 110);
        assert_eq!(records[1].at_qpc, 120);
        assert_eq!(buffer.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn file_recorder_writes_header_and_flushes_records_on_drop() {
        let _lock = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let temp_dir =
            std::env::temp_dir().join(format!("oscmidi-trace-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("test-trace.jsonl");
        std::env::set_var("OSCMIDI_LATENCY_LOG_FILE", &log_file);

        {
            let _guard = start_file_recorder();
            record(Stage::Routing, &[0x90, 60, 127], 1000, 1050);
            record(Stage::OscSent, &[0x90, 60, 127], 1000, 1100);
        }

        std::env::remove_var("OSCMIDI_LATENCY_LOG_FILE");
        assert!(log_file.is_file());
        let content = std::fs::read_to_string(&log_file).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.len() >= 3);
        assert!(lines[0].contains("qpc_frequency"));
        assert!(lines[1].contains("Routing"));
        assert!(lines[2].contains("OscSent"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
