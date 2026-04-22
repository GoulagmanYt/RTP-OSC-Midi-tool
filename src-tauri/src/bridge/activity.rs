use crate::{
    midi::{parse_note, MidiFrame},
    types::MidiActivityInfo,
};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
struct MidiActivityState {
    messages: u32,
    last_note: Option<u8>,
    last_channel: Option<u8>,
    last_seen_ms: Option<u64>,
}

#[derive(Default)]
pub(super) struct MidiActivityTracker {
    stats: HashMap<String, MidiActivityState>,
}

impl MidiActivityTracker {
    pub(super) fn record(&mut self, source: &str, data: &[u8]) {
        let entry = self.stats.entry(source.to_string()).or_default();
        entry.messages = entry.messages.saturating_add(1);
        entry.last_seen_ms = Some(now_ms());
        if let Some(note) = parse_note(data) {
            entry.last_note = Some(note.note);
            entry.last_channel = Some(note.channel);
        }
    }

    pub(super) fn snapshot_and_reset(&mut self) -> Vec<MidiActivityInfo> {
        let now = now_ms();
        let mut snapshot = Vec::with_capacity(self.stats.len());
        self.stats.retain(|source, state| {
            snapshot.push(MidiActivityInfo {
                source: source.to_string(),
                messages_per_sec: state.messages,
                last_note: state.last_note,
                last_channel: state.last_channel,
                last_seen_ms: state.last_seen_ms,
            });
            state.messages = 0;
            if let Some(last) = state.last_seen_ms {
                now.saturating_sub(last) < 300_000
            } else {
                true
            }
        });
        snapshot
    }
}

pub(super) fn record_activity(activity: &Arc<Mutex<MidiActivityTracker>>, frame: &MidiFrame) {
    let mut guard = activity.lock();
    guard.record(&*frame.source, frame.data.as_slice());
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
