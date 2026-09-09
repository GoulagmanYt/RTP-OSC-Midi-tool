use crate::packets::midi_packets::recovery_journal::{Journal, note_layout};
use midi_types::{Channel, Control, MidiMessage, Note, Value7, Value14};

pub(crate) struct ChannelRepairs {
    messages: [Option<MidiMessage>; 4144],
    len: usize,
}
impl ChannelRepairs {
    pub(crate) fn build(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
    ) -> Option<Self> {
        if expected.wrapping_sub(journal.checkpoint()) >= 0x8000 {
            return None;
        }
        let channels = journal.state_channels()?;
        let mut repairs = Self {
            messages: [None; 4144],
            len: 0,
        };
        for (channel, toc, mut data) in channels {
            let midi_channel = Channel::from(channel);
            if toc & 16 != 0 {
                repairs.push(MidiMessage::PitchBendChange(
                    midi_channel,
                    Value14::from(u16::from(data[0] & 127) | (u16::from(data[1] & 127) << 7)),
                ));
                data = &data[2..];
            }
            if toc & 8 != 0 {
                let (notes, _, off) = note_layout(data).ok()?;
                let size = 2 + notes * 2 + off;
                repairs.repair_notes(active, channel, &data[..size])?;
                data = &data[size..];
            }
            if toc & 2 != 0 {
                repairs.push(MidiMessage::ChannelPressure(
                    midi_channel,
                    Value7::from(data[0] & 127),
                ));
                data = &data[1..];
            }
            if toc & 1 != 0 {
                for log in data[1..].chunks_exact(2) {
                    // X marks pressure preceding All Notes/Sound Off. Do not
                    // restore that stale pressure to a newly sounding voice.
                    if log[1] & 128 == 0 {
                        repairs.push(MidiMessage::KeyPressure(
                            midi_channel,
                            Note::from(log[0] & 127),
                            Value7::from(log[1] & 127),
                        ));
                    }
                }
            }
        }
        Some(repairs)
    }

    fn repair_notes(&mut self, active: &mut [u128; 16], channel: u8, data: &[u8]) -> Option<()> {
        let (notes, low, off) = note_layout(data).ok()?;
        let note_offs = &data[2 + notes * 2..2 + notes * 2 + off];
        let channel_index = usize::from(channel);
        let channel = Channel::from(channel);
        if note_offs.iter().any(|bits| *bits != 0) {
            // The journal cannot always order sustain changes against lost
            // NoteOffs. RFC 6295 A.6.2 recommends silencing ambiguous pedals.
            self.push(MidiMessage::ControlChange(
                channel,
                Control::from(64),
                Value7::from(0),
            ));
        }
        for (index, bits) in note_offs.iter().enumerate() {
            for bit in 0..8 {
                if bits & (0x80 >> bit) != 0 {
                    let note = ((low + index) * 8 + bit) as u8;
                    self.push(MidiMessage::NoteOff(
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
                self.push(MidiMessage::NoteOn(
                    channel,
                    Note::from(note),
                    Value7::from(log[1] & 127),
                ));
                active[channel_index] |= 1u128 << note;
            }
        }
        Some(())
    }
    fn push(&mut self, message: MidiMessage) {
        // <=16 channels: 128 note repairs, 128 pressure logs, pedal, wheel, pressure.
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
    fn restores_pitch_and_pressure_with_notes_and_skips_stale_poly_pressure() {
        let bytes = [
            0x20, 0, 0, 0, 16, 0x1B, 127, 127, 1, 0x77, 61, 0xE4, 8, 101, 1, 61, 32, 62, 0xA0,
        ];
        let mut active = [0; 16];
        active[0] = 1 << 60;
        let repairs =
            ChannelRepairs::build(Journal::parse(&bytes).unwrap(), 1, &mut active).unwrap();
        assert_eq!(
            repairs.messages().collect::<Vec<_>>(),
            [
                MidiMessage::PitchBendChange(Channel::C1, Value14::from(16383u16)),
                MidiMessage::ControlChange(Channel::C1, Control::from(64), Value7::from(0)),
                MidiMessage::NoteOff(Channel::C1, Note::from(60), Value7::from(64)),
                MidiMessage::NoteOn(Channel::C1, Note::from(61), Value7::from(100)),
                MidiMessage::ChannelPressure(Channel::C1, Value7::from(101)),
                MidiMessage::KeyPressure(Channel::C1, Note::from(61), Value7::from(32)),
            ]
        );
        assert_eq!(active[0], 1 << 61);
        let mut duplicate = bytes;
        duplicate[17] = 61;
        assert!(Journal::parse(&duplicate).is_err());
    }

    #[test]
    fn maximum_channel_state_fits_preallocated_repair_plan() {
        let mut bytes = vec![0x2F, 0, 0];
        for channel in 0..16 {
            let mut body = vec![127, 127, 127, 0xF0];
            for note in 0..128 {
                body.extend_from_slice(&[note, 0xE4]);
            }
            body.extend_from_slice(&[127, 127]);
            for note in 0..128 {
                body.extend_from_slice(&[note, 100]);
            }
            let size = body.len() + 3;
            bytes.extend_from_slice(&[(channel << 3) | ((size >> 8) as u8 & 3), size as u8, 0x1B]);
            bytes.extend_from_slice(&body);
        }
        let mut active = [0; 16];
        let repairs =
            ChannelRepairs::build(Journal::parse(&bytes).unwrap(), 1, &mut active).unwrap();
        assert_eq!(repairs.messages().count(), 16 * 258);
        assert_eq!(active, [u128::MAX; 16]);
    }
    #[test]
    fn recovers_notes_across_rollover_without_retriggering_known_notes() {
        let bytes = [0x20, 0xFF, 0xFE, 0, 8, 8, 1, 0x77, 61, 0xE4, 8];
        let journal = Journal::parse(&bytes).unwrap();
        let mut active = [0; 16];
        active[0] = 1 << 60;
        let plan = ChannelRepairs::build(journal, 0, &mut active).unwrap();
        assert_eq!(plan.messages().count(), 3);
        assert_eq!(active[0], 1 << 61);
        assert_eq!(
            ChannelRepairs::build(journal, 0, &mut active)
                .unwrap()
                .messages()
                .count(),
            2
        );
        assert!(ChannelRepairs::build(journal, 0xFFFD, &mut active).is_none());
        let unsupported = Journal::parse(&[0x20, 0, 0, 0, 6, 128, 0, 0, 0]).unwrap();
        assert!(ChannelRepairs::build(unsupported, 1, &mut active).is_none());
    }
}
