use super::activity::MidiActivityTracker;
use crate::{audio::AudioEngine, types::BridgeMetrics};
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::{Emitter, Window};

pub(super) const MIDI_ACTIVITY_EVENT: &str = "midi:activity";
pub(super) const BRIDGE_METRICS_EVENT: &str = "runtime:metrics";

pub(super) fn spawn_activity_emitter(
    stop: Arc<AtomicBool>,
    window: Window,
    activity_state: Arc<Mutex<MidiActivityTracker>>,
    osc_counter: Arc<AtomicU32>,
    audio: AudioEngine,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
            let snapshot = activity_state.lock().snapshot_and_reset();
            let midi_messages = snapshot.iter().fold(0u32, |acc, entry| {
                acc.saturating_add(entry.messages_per_sec)
            });
            let _ = window.emit(MIDI_ACTIVITY_EVENT, snapshot);
            let osc_messages = osc_counter.swap(0, Ordering::Relaxed);
            let peaks = audio.peak_levels();
            let (audio_peak_l, audio_peak_r) = match peaks {
                Some((left, right)) => (Some(left), Some(right)),
                None => (None, None),
            };
            let metrics = BridgeMetrics {
                audio_peak_l,
                audio_peak_r,
                audio_latency_ms: audio.current_latency_ms(),
                audio_xruns: audio.xrun_count(),
                audio_midi_drops: audio.midi_drop_count(),
                audio_lock_misses: audio.audio_lock_miss_count(),
                audio_emergency_resets: audio.emergency_reset_count(),
                midi_messages_per_sec: midi_messages,
                osc_messages_per_sec: osc_messages,
            };
            let _ = window.emit(BRIDGE_METRICS_EVENT, metrics);
        }
    })
}
