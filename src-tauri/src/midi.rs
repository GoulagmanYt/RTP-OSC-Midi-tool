use smallvec::SmallVec;
use std::sync::Arc;

pub const NOTE_MIN: u8 = 21;
pub const NOTE_MAX: u8 = 108;

#[derive(Debug, Clone)]
pub struct MidiFrame {
    pub data: SmallVec<[u8; 32]>,
    pub source: Arc<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiKind {
    NoteOn,
    NoteOff,
}

#[derive(Debug, Clone, Copy)]
pub struct MidiNote {
    pub note: u8,
    pub channel: u8,
    pub kind: MidiKind,
    pub index: Option<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct MidiSustain {
    pub channel: u8,
    pub pressed: bool,
}

pub fn parse_note(data: &[u8]) -> Option<MidiNote> {
    if data.len() < 2 {
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
    use crossbeam_channel::bounded;
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
        let (tx, rx) = bounded::<MidiFrame>(8192);

        let producer = thread::spawn(move || {
            for i in 0..TOTAL {
                let note = NOTE_MIN + (i % usize::from(NOTE_MAX - NOTE_MIN + 1)) as u8;
                let status = if i % 2 == 0 { 0x90 } else { 0x80 };
                let velocity = if status == 0x90 { 100 } else { 0 };
                tx.send(MidiFrame {
                    data: SmallVec::from_slice(&[status, note, velocity]),
                    source: Arc::from("stress"),
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
