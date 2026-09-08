use crate::packets::midi_packets::recovery_journal::{Journal, note_layout};
use midi_types::{Channel, Control, MidiMessage, Note, Value7};

pub(crate) struct NoteRepairs {
    messages: [Option<MidiMessage>; 2064],
    len: usize,
}
impl NoteRepairs {
    pub(crate) fn build(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
    ) -> Option<Self> {
        if expected.wrapping_sub(journal.checkpoint()) >= 0x8000 {
            return None;
        }
        let channels = journal.note_channels()?;
        let mut repairs = Self {
            messages: [None; 2064],
            len: 0,
        };
        for (channel, data) in channels {
            let (notes, low, off) = note_layout(data).ok()?;
            let note_offs = &data[2 + notes * 2..2 + notes * 2 + off];
            let channel_index = usize::from(channel);
            let channel = Channel::from(channel);
            if note_offs.iter().any(|bits| *bits != 0) {
                // The journal cannot always order sustain changes against lost
                // NoteOffs. RFC 6295 A.6.2 recommends silencing ambiguous pedals.
                repairs.push(MidiMessage::ControlChange(
                    channel,
                    Control::from(64),
                    Value7::from(0),
                ));
            }
            for (index, bits) in note_offs.iter().enumerate() {
                for bit in 0..8 {
                    if bits & (0x80 >> bit) != 0 {
                        let note = ((low + index) * 8 + bit) as u8;
                        repairs.push(MidiMessage::NoteOff(
                            channel,
                            Note::from(note),
                            Value7::from(64),
                        ));
                        active[channel_index] &= !(1u128 << note);
                    }
                }
            }
            for log in data[2..2 + notes * 2].chunks_exact(2) {
                let note = log[0] & 127;
                if active[channel_index] & (1u128 << note) == 0 && log[1] & 128 != 0 {
                    repairs.push(MidiMessage::NoteOn(
                        channel,
                        Note::from(note),
                        Value7::from(log[1] & 127),
                    ));
                    active[channel_index] |= 1u128 << note;
                }
            }
        }
        Some(repairs)
    }
    fn push(&mut self, message: MidiMessage) {
        // A validated journal has <=16 channels and <=128 distinct notes each.
        self.messages[self.len] = Some(message);
        self.len += 1;
    }
    pub(crate) fn messages(&self) -> impl Iterator<Item = MidiMessage> + '_ {
        self.messages[..self.len].iter().copied().flatten()
    }
}

pub(crate) fn observe(active: &mut [u128; 16], message: MidiMessage) {
    match message {
        MidiMessage::NoteOn(channel, note, velocity) if u8::from(velocity) != 0 => {
            active[usize::from(u8::from(channel))] |= 1u128 << u8::from(note)
        }
        MidiMessage::NoteOff(channel, note, _) | MidiMessage::NoteOn(channel, note, _) => {
            active[usize::from(u8::from(channel))] &= !(1u128 << u8::from(note))
        }
        MidiMessage::ControlChange(channel, control, _)
            if matches!(u8::from(control), 120 | 123..=127) =>
        {
            active[usize::from(u8::from(channel))] = 0
        }
        MidiMessage::Reset => *active = [0; 16],
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_notes_across_rollover_without_retriggering_known_notes() {
        let bytes = [0x20, 0xFF, 0xFE, 0, 8, 8, 1, 0x77, 61, 0xE4, 8];
        let journal = Journal::parse(&bytes).unwrap();
        let mut active = [0; 16];
        active[0] = 1 << 60;
        let plan = NoteRepairs::build(journal, 0, &mut active).unwrap();
        assert_eq!(plan.messages().count(), 3);
        assert_eq!(active[0], 1 << 61);
        assert_eq!(
            NoteRepairs::build(journal, 0, &mut active)
                .unwrap()
                .messages()
                .count(),
            2
        );
        assert!(NoteRepairs::build(journal, 0xFFFD, &mut active).is_none());
        let unsupported = Journal::parse(&[0x20, 0, 0, 0, 5, 16, 0, 64]).unwrap();
        assert!(NoteRepairs::build(unsupported, 1, &mut active).is_none());
    }
}
