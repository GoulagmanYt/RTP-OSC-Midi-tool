use serde::{Deserialize, Serialize};

pub const NOTE_MIN: u8 = 21;
pub const NOTE_MAX: u8 = 108;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MidiFrame {
    pub data: smallvec::SmallVec<[u8; 32]>,
    pub source: std::sync::Arc<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiKind {
    NoteOn,
    NoteOff,
}

#[derive(Debug, Clone)]
pub struct MidiNote {
    pub note: u8,
    pub channel: u8,
    /// Index de la note (pour le tri)
    pub index: Option<u8>,
    pub kind: MidiKind,
}

#[derive(Debug, Clone, Copy)]
pub struct MidiSustain {
    pub channel: u8,
    pub pressed: bool,
}

pub fn parse_note(data: &[u8]) -> Option<MidiNote> {
    if data.len() != 3 || data[1..].iter().any(|byte| *byte >= 0x80) {
        return None;
    }
    let status = data[0];
    let message_type = status & 0xF0;
    let channel = (status & 0x0F) + 1; // human readable
    let note = data[1];
    let velocity = if data.len() > 2 { data[2] } else { 0 };

    match message_type {
        0x90 => Some(MidiNote {
            note,
            channel,
            kind: if velocity == 0 {
                MidiKind::NoteOff
            } else {
                MidiKind::NoteOn
            },
            index: adjust_note(note),
        }),
        0x80 => Some(MidiNote {
            note,
            channel,
            kind: MidiKind::NoteOff,
            index: adjust_note(note),
        }),
        _ => None,
    }
}

/// Parse un message MIDI de sustain
pub fn parse_sustain(data: &[u8]) -> Option<MidiSustain> {
    if data.len() < 3 {
        return None;
    }
    let status = data[0];
    if (status & 0xF0) != 0xB0 || data[1] != 64 {
        return None;
    }
    let channel = (status & 0x0F) + 1; // human readable
    let pressed = data[2] >= 64;
    Some(MidiSustain { channel, pressed })
}

pub fn is_critical_release_message(data: &[u8]) -> bool {
    if data == [0xFF] {
        return true;
    }
    if data.len() < 3 {
        return false;
    }
    let status = data[0] & 0xF0;
    matches!(status, 0x80)
        || (status == 0x90 && data[2] == 0)
        || (status == 0xB0
            && ((data[1] == 64 && data[2] < 64) || matches!(data[1], 120 | 121 | 123)))
}

/// Validate complete driver/bridge messages before indexing or dispatching them.
pub fn normalize_message(data: &mut [u8]) -> bool {
    let Some(&status) = data.first() else {
        return false;
    };
    let len = match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 3,
        0xC0..=0xDF | 0xF1 | 0xF3 => 2,
        0xF6 | 0xF8 | 0xFA..=0xFC | 0xFE..=0xFF => 1,
        0xF0 => {
            return data.len() >= 2
                && data.last() == Some(&0xF7)
                && data[1..data.len() - 1].iter().all(|b| *b < 0x80)
        }
        _ => return false,
    };
    if data.len() != len || data[1..].iter().any(|byte| *byte >= 0x80) {
        return false;
    }
    if status & 0xF0 == 0x90 && data[2] == 0 {
        data[0] = 0x80 | (status & 0x0F);
    }
    true
}

/// Owned by the MIDI destination thread; no shared mutation or heap allocation.
#[derive(Default)]
pub(crate) struct ActiveNotes {
    notes: [u128; 16],
}

impl ActiveNotes {
    pub(crate) fn observe(&mut self, data: &[u8]) {
        if let Some(note) = parse_note(data) {
            let channel = usize::from(note.channel - 1);
            let mask = 1u128 << note.note;
            if note.kind == MidiKind::NoteOn {
                self.notes[channel] |= mask;
            } else {
                self.notes[channel] &= !mask;
            }
        } else if data == [0xFF] {
            self.notes.fill(0);
        } else if data.len() == 3 && data[0] & 0xF0 == 0xB0 && matches!(data[1], 120 | 123) {
            self.notes[usize::from(data[0] & 0x0F)] = 0;
        }
    }

    pub(crate) fn release_all(&mut self, mut send: impl FnMut([u8; 3])) {
        for (channel, notes) in self.notes.iter_mut().enumerate() {
            while *notes != 0 {
                let note = notes.trailing_zeros() as u8;
                send([0x80 | channel as u8, note, 0]);
                *notes &= !(1u128 << note);
            }
        }
    }
}

pub fn adjust_note(note: u8) -> Option<u8> {
    if (NOTE_MIN..=NOTE_MAX).contains(&note) {
        Some(note - NOTE_MIN + 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::{thread, time::Instant};

    #[test]
    fn adjust_note_maps_correctly() {
        assert_eq!(adjust_note(21), Some(1));
        assert_eq!(adjust_note(60), Some(40));
        assert_eq!(adjust_note(108), Some(88));
        assert_eq!(adjust_note(20), None);
        assert_eq!(adjust_note(109), None);
    }

    #[test]
    fn parse_sustain_maps_threshold() {
        let on = parse_sustain(&[0xB0, 64, 127]).expect("sustain on");
        assert_eq!(on.channel, 1);
        assert!(on.pressed);

        let off = parse_sustain(&[0xB3, 64, 63]).expect("sustain off");
        assert_eq!(off.channel, 4);
        assert!(!off.pressed);
    }

    #[test]
    fn parse_sustain_ignores_non_sustain_messages() {
        assert!(parse_sustain(&[0x90, 60, 100]).is_none());
        assert!(parse_sustain(&[0xB0, 1, 127]).is_none());
        assert!(parse_sustain(&[0xB0, 64]).is_none());
    }

    #[test]
    fn stress_midi_pipeline_no_loss() {
        const TOTAL: usize = 400_000;
        let (tx, rx) = unbounded::<MidiFrame>();

        let producer = thread::spawn(move || {
            for i in 0..TOTAL {
                let note = NOTE_MIN + (i % usize::from(NOTE_MAX - NOTE_MIN + 1)) as u8;
                let status = if i % 2 == 0 { 0x90 } else { 0x80 };
                let velocity = if status == 0x90 { 100 } else { 0 };
                tx.send(MidiFrame {
                    data: smallvec::SmallVec::from_slice(&[status, note, velocity]),
                    source: std::sync::Arc::from("stress"),
                })
                .expect("producer send");
            }
        });

        let start = Instant::now();
        let mut received = 0usize;
        while received < TOTAL {
            let frame = rx.recv().expect("consumer recv");
            let parsed = parse_note(frame.data.as_slice());
            assert!(parsed.is_some(), "frame not parsed at index {received}");
            received += 1;
        }
        let elapsed = start.elapsed();
        producer.join().expect("producer join");

        assert_eq!(received, TOTAL, "MIDI stress lost messages");
        eprintln!(
            "MIDI stress: {received} messages in {:?} ({:.0} msg/s)",
            elapsed,
            received as f64 / elapsed.as_secs_f64()
        );
    }
}
