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

pub fn record_rtp(bytes: &[u8], ingress_qpc: u64, planned_wait_us: u64) {
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
        at_qpc: ingress_qpc,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_audio_health_does_not_consume_diagnostic_records() {
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
}
