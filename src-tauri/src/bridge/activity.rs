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
        let timestamp_ms = now_ms();
        if let Some(entry) = self.stats.get_mut(source) {
            entry.record(data, timestamp_ms);
            return;
        }

        let mut entry = MidiActivityState::default();
        entry.record(data, timestamp_ms);
        self.stats.insert(source.to_owned(), entry);
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

impl MidiActivityState {
    fn record(&mut self, data: &[u8], timestamp_ms: u64) {
        self.messages = self.messages.saturating_add(1);
        self.last_seen_ms = Some(timestamp_ms);
        if let Some(note) = parse_note(data) {
            self.last_note = Some(note.note);
            self.last_channel = Some(note.channel);
        }
    }
}

pub(super) fn record_activity(activity: &Arc<Mutex<MidiActivityTracker>>, frame: &MidiFrame) {
    let mut guard = activity.lock();
    guard.record(&frame.source, frame.data.as_slice());
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
