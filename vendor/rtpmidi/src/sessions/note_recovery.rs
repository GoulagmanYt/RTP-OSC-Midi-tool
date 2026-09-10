use super::channel_state::{ChannelHistory, ProgramState};
use crate::packets::midi_packets::recovery_journal::{Journal, note_layout};
use midi_types::{Channel, Control, MidiMessage, Note, Value7, Value14};

// Fixed work budget covers ordinary complete channel journals. Large note
// reference-count repairs must also fit; otherwise the plan is rolled back.
const MAX_REPAIRS: usize = 16 * 519;

pub(crate) struct ChannelRepairs {
    messages: [Option<MidiMessage>; MAX_REPAIRS],
    len: usize,
    overflow: bool,
}
impl ChannelRepairs {
    #[cfg(test)]
    pub(crate) fn build(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
    ) -> Option<Self> {
        Self::build_with_state(journal, expected, active, &mut ChannelHistory::default())
    }

    pub(super) fn build_with_state(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
    ) -> Option<Self> {
        let saved_active = *active;
        let saved_history = *history;
        let plan = Self::build_plan(journal, expected, active, history);
        if plan.is_none() {
            *active = saved_active;
            *history = saved_history;
        }
        plan
    }

    fn build_plan(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
    ) -> Option<Self> {
        if expected.wrapping_sub(journal.checkpoint()) >= 0x8000 {
            return None;
        }
        let channels = journal.state_channels()?;
        let mut repairs = Self {
            messages: [None; MAX_REPAIRS],
            len: 0,
            overflow: false,
        };
        for (channel, toc, mut data) in channels {
            let channel_start = repairs.len;
            let midi_channel = Channel::from(channel);
            if toc & 128 != 0 {
                let desired = ProgramState {
                    program: data[0] & 127,
                    bank: (data[1] & 128 != 0).then_some((data[1] & 127, data[2] & 127)),
                };
                if history.0[usize::from(channel)].program != Some(desired) {
                    if let Some((msb, lsb)) = desired.bank {
                        repairs.controller(history, channel, 0, msb);
                        repairs.controller(history, channel, 32, lsb);
                    }
                    let program = MidiMessage::ProgramChange(midi_channel, desired.program.into());
                    repairs.push(program);
                    history.observe(program);
                    history.0[usize::from(channel)].program = Some(desired);
                }
                data = &data[3..];
            }
            if toc & 64 != 0 {
                let size = 1 + 2 * (usize::from(data[0] & 127) + 1);
                repairs.repair_controllers(history, channel, &data[1..size])?;
                data = &data[size..];
            }
            for message in repairs.messages[channel_start..repairs.len]
                .iter()
                .flatten()
            {
                observe(active, *message);
            }
            if toc & 16 != 0 {
                repairs.push(MidiMessage::PitchBendChange(
                    midi_channel,
                    Value14::from(u16::from(data[0] & 127) | (u16::from(data[1] & 127) << 7)),
                ));
                data = &data[2..];
            }
            let note_data = if toc & 8 != 0 {
                let (notes, _, off) = note_layout(data).ok()?;
                let size = 2 + notes * 2 + off;
                let section = &data[..size];
                data = &data[size..];
                Some(section)
            } else {
                None
            };
            let extras = if toc & 4 != 0 {
                let size = 1 + 2 * (usize::from(data[0] & 127) + 1);
                let section = &data[1..size];
                data = &data[size..];
                section
            } else {
                &[]
            };
            if note_data.is_some() || !extras.is_empty() {
                repairs.repair_notes(active, history, channel, note_data, extras)?;
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
        if repairs.overflow {
            None
        } else {
            Some(repairs)
        }
    }

    fn controller(&mut self, history: &mut ChannelHistory, channel: u8, number: u8, value: u8) {
        let message = MidiMessage::ControlChange(
            Channel::from(channel),
            Control::from(number),
            Value7::from(value),
        );
        self.push(message);
        history.observe(message);
    }

    fn repair_controllers(
        &mut self,
        history: &mut ChannelHistory,
        channel: u8,
        mut logs: &[u8],
    ) -> Option<()> {
        while !logs.is_empty() {
            let number = logs[0] & 127;
            let (mut count, mut value, mut toggles) = (None, None, None);
            while !logs.is_empty() && logs[0] & 127 == number {
                match logs[1] & 0xC0 {
                    0xC0 => count = Some(logs[1] & 63),
                    0x80 => toggles = Some(logs[1] & 63),
                    _ => value = Some(logs[1] & 127),
                }
                logs = &logs[2..];
            }
            let previous = history.0[usize::from(channel)].controls[usize::from(number)];
            let missing_count = count.is_some_and(|c| !previous.count_known || c != previous.count);
            let missing_toggle =
                toggles.is_some_and(|t| !previous.toggles_known || t != previous.toggles);
            // A count identifies a missing command, not its data value. Only
            // channel-mode commands whose value is ignored can be reconstructed
            // from the count alone. Guessing a volume or mono-channel count would
            // introduce an indefinite artifact of our own.
            if missing_count
                && value.is_none()
                && toggles.is_none()
                && !matches!(number, 120 | 121 | 123..=125 | 127)
            {
                return None;
            }
            let final_value =
                value.unwrap_or_else(|| toggles.map_or(0, |t| if t & 1 != 0 { 127 } else { 0 }));
            let reset_pair = value.is_some()
                && number < 32
                && history.0[usize::from(channel)].controls[usize::from(number) + 32].value
                    != Some(0)
                && !logs.chunks_exact(2).any(|log| log[0] & 127 == number + 32);
            let changed_value = value.is_some() && previous.value != value;
            let changed_toggle_value =
                toggles.is_some() && previous.value.map(|v| v >= 64) != Some(final_value >= 64);
            let should_emit = missing_count
                || missing_toggle
                || changed_value
                || changed_toggle_value
                || reset_pair;
            if should_emit {
                // A lost off->on transition must damp old sustained voices
                // before restoring the current on state.
                if missing_toggle && final_value >= 64 {
                    self.controller(history, channel, number, 0);
                }
                self.controller(history, channel, number, final_value);
            }
            let current = &mut history.0[usize::from(channel)].controls[usize::from(number)];
            if let Some(c) = count {
                current.count = c;
                current.count_known = true;
            } else if should_emit {
                current.count_known = false;
            }
            if let Some(t) = toggles {
                current.toggles = t;
                current.toggles_known = true;
            } else if should_emit {
                current.toggles_known = false;
            }
        }
        Some(())
    }

    fn repair_notes(
        &mut self,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
        channel: u8,
        data: Option<&[u8]>,
        extras: &[u8],
    ) -> Option<()> {
        let index = usize::from(channel);
        let midi_channel = Channel::from(channel);
        let mut targets = [None; 128];
        let mut recommended = [false; 128];
        let mut force_release = [false; 128];
        let mut velocities = history.0[index].velocities;
        let mut releases = [64u8; 128];
        if let Some(data) = data {
            let (notes, low, off) = note_layout(data).ok()?;
            for log in data[2..2 + notes * 2].chunks_exact(2) {
                let note = usize::from(log[0] & 127);
                targets[note] = Some(1u16);
                recommended[note] = log[1] & 128 != 0;
                velocities[note] = log[1] & 127;
            }
            for (offset, bits) in data[2 + notes * 2..2 + notes * 2 + off].iter().enumerate() {
                for bit in 0..8 {
                    if bits & (0x80 >> bit) != 0 {
                        let note = (low + offset) * 8 + bit;
                        targets[note] = Some(0);
                        force_release[note] = true;
                    }
                }
            }
        }
        for log in extras.chunks_exact(2) {
            let note = usize::from(log[0] & 127);
            if log[1] & 128 != 0 {
                releases[note] = log[1] & 127;
            } else {
                // 127 represents an unbounded reference count, not exactly 127.
                // Use the caller's reset fallback when exact repair is ambiguous.
                if log[1] == 127 {
                    return None;
                }
                targets[note] = Some(u16::from(log[1]));
            }
        }
        let mut offs = [0u16; 128];
        let mut ons = [0u16; 128];
        let mut required = 0usize;
        for note in 0..128 {
            if let Some(target) = targets[note] {
                let current = history.0[index].notes[note]
                    .max(u16::from(active[index] & (1u128 << note) != 0));
                history.0[index].notes[note] = current;
                offs[note] = current
                    .saturating_sub(target)
                    .max(u16::from(force_release[note] && target == 0));
                if recommended[note] {
                    ons[note] = target.saturating_sub(current);
                }
                required += usize::from(offs[note]) + usize::from(ons[note]);
            }
        }
        let release_pedal = offs.iter().any(|count| *count != 0);
        let sustain = history.0[index].controls[64]
            .value
            .filter(|value| *value >= 64);
        required += usize::from(release_pedal) * (1 + usize::from(sustain.is_some()));
        if required > MAX_REPAIRS.saturating_sub(self.len) {
            return None;
        }
        if release_pedal {
            self.push(MidiMessage::ControlChange(
                midi_channel,
                Control::from(64),
                Value7::from(0),
            ));
        }
        for (note, count) in offs.into_iter().enumerate() {
            for _ in 0..count {
                let message = MidiMessage::NoteOff(
                    midi_channel,
                    Note::from(note as u8),
                    Value7::from(releases[note]),
                );
                self.push(message);
                observe_received(active, history, message);
            }
        }
        if release_pedal && let Some(sustain) = sustain {
            self.push(MidiMessage::ControlChange(
                midi_channel,
                Control::from(64),
                Value7::from(sustain),
            ));
        }
        for (note, count) in ons.into_iter().enumerate() {
            for _ in 0..count {
                let message = MidiMessage::NoteOn(
                    midi_channel,
                    Note::from(note as u8),
                    Value7::from(velocities[note]),
                );
                self.push(message);
                observe_received(active, history, message);
            }
        }
        Some(())
    }
    fn push(&mut self, message: MidiMessage) {
        // No journal may grow the storage or exceed the bounded repair work budget.
        if let Some(slot) = self.messages.get_mut(self.len) {
            *slot = Some(message);
            self.len += 1;
        } else {
            self.overflow = true;
        }
    }
    pub(crate) fn messages(&self) -> impl Iterator<Item = MidiMessage> + '_ {
        self.messages[..self.len].iter().copied().flatten()
    }
}

pub(super) fn observe_received(
    active: &mut [u128; 16],
    history: &mut ChannelHistory,
    message: MidiMessage,
) {
    history.observe(message);
    observe(active, message);
    match message {
        MidiMessage::NoteOn(channel, note, _) | MidiMessage::NoteOff(channel, note, _) => {
            let channel = usize::from(u8::from(channel));
            let note = u8::from(note);
            if history.0[channel].notes[usize::from(note)] > 0 {
                active[channel] |= 1u128 << note;
            }
        }
        _ => {}
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

    fn journal(toc: u8, chapters: &[u8]) -> Vec<u8> {
        let size = chapters.len() + 3;
        let mut bytes = vec![0x20, 0, 0, ((size >> 8) & 3) as u8, size as u8, toc];
        bytes.extend_from_slice(chapters);
        bytes
    }

    fn cc(number: u8, value: u8) -> MidiMessage {
        MidiMessage::ControlChange(Channel::C1, number.into(), value.into())
    }

    #[test]
    fn program_banks_volume_and_pedal_recover_without_replaying_known_commands() {
        let bytes = journal(0xC0, &[127, 0x82, 3, 1, 7, 100, 64, 0x83]);
        let mut history = ChannelHistory::default();
        let mut active = [0; 16];
        let repairs = ChannelRepairs::build_with_state(
            Journal::parse(&bytes).unwrap(),
            1,
            &mut active,
            &mut history,
        )
        .unwrap();
        assert_eq!(
            repairs.messages().collect::<Vec<_>>(),
            [
                cc(0, 2),
                cc(32, 3),
                MidiMessage::ProgramChange(Channel::C1, 127.into()),
                cc(7, 100),
                cc(64, 0),
                cc(64, 127),
            ]
        );
        assert_eq!(
            ChannelRepairs::build_with_state(
                Journal::parse(&bytes).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .unwrap()
            .messages()
            .count(),
            0
        );
        // A missed off->on cycle must still damp old voices, despite equal final values.
        let toggled = journal(64, &[0, 64, 0x85]);
        assert_eq!(
            ChannelRepairs::build_with_state(
                Journal::parse(&toggled).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
            [cc(64, 0), cc(64, 127)]
        );
    }

    #[test]
    fn controller_count_wrap_and_reset_do_not_suppress_recovered_notes() {
        let mut history = ChannelHistory::default();
        let mut active = [0; 16];
        let initial = journal(64, &[0, 123, 0xFF]);
        ChannelRepairs::build_with_state(
            Journal::parse(&initial).unwrap(),
            1,
            &mut active,
            &mut history,
        )
        .unwrap();
        history.observe(cc(123, 0)); // count 63 -> 0
        let same = journal(64, &[0, 123, 0xC0]);
        assert_eq!(
            ChannelRepairs::build_with_state(
                Journal::parse(&same).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .unwrap()
            .messages()
            .count(),
            0
        );
        active[0] = 1 << 61;
        let next = journal(0x48, &[0, 123, 0xC1, 1, 0xF0, 61, 0xE4]);
        assert_eq!(
            ChannelRepairs::build_with_state(
                Journal::parse(&next).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
            [
                cc(123, 0),
                MidiMessage::NoteOn(Channel::C1, 61.into(), 100.into())
            ]
        );
        assert_eq!(active[0], 1 << 61);
    }

    #[test]
    fn note_repair_temporarily_releases_and_restores_known_sustain() {
        let mut history = ChannelHistory::default();
        history.observe(cc(64, 100));
        let mut active = [0; 16];
        active[0] = 1 << 60;
        let bytes = journal(8, &[0, 0x77, 8]);
        let repairs = ChannelRepairs::build_with_state(
            Journal::parse(&bytes).unwrap(),
            1,
            &mut active,
            &mut history,
        )
        .unwrap();
        assert_eq!(
            repairs.messages().collect::<Vec<_>>(),
            [
                cc(64, 0),
                MidiMessage::NoteOff(Channel::C1, 60.into(), 64.into()),
                cc(64, 100)
            ]
        );
    }

    #[test]
    fn controller_msb_lsb_order_and_implicit_lsb_reset_are_preserved() {
        let mut history = ChannelHistory::default();
        let mut active = [0; 16];
        let paired = journal(64, &[1, 7, 100, 39, 127]);
        let plan = ChannelRepairs::build_with_state(
            Journal::parse(&paired).unwrap(),
            1,
            &mut active,
            &mut history,
        )
        .unwrap();
        assert_eq!(
            plan.messages().collect::<Vec<_>>(),
            [cc(7, 100), cc(39, 127)]
        );
        assert_eq!(
            ChannelRepairs::build_with_state(
                Journal::parse(&paired).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .unwrap()
            .messages()
            .count(),
            0
        );
        let msb_only = journal(64, &[0, 7, 100]);
        assert_eq!(
            ChannelRepairs::build_with_state(
                Journal::parse(&msb_only).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
            [cc(7, 100)]
        );
        assert_eq!(history.0[0].controls[39].value, Some(0));
    }

    #[test]
    fn extras_recover_stacked_releases_and_release_velocity() {
        let mut history = ChannelHistory::default();
        let mut active = [0; 16];
        let on = MidiMessage::NoteOn(Channel::C1, 60.into(), 100.into());
        for _ in 0..3 {
            observe_received(&mut active, &mut history, on);
        }
        observe_received(
            &mut active,
            &mut history,
            MidiMessage::NoteOff(Channel::C1, 60.into(), 17.into()),
        );
        assert_eq!(active[0], 1 << 60);
        assert_eq!(history.0[0].notes[60], 2);
        // Last command is NoteOff, but one stacked voice remains in the sender.
        let bytes = journal(0x0C, &[0, 0x77, 8, 1, 60, 1, 60, 0x91]);
        let plan = ChannelRepairs::build_with_state(
            Journal::parse(&bytes).unwrap(),
            1,
            &mut active,
            &mut history,
        )
        .unwrap();
        assert_eq!(
            plan.messages().collect::<Vec<_>>(),
            [
                cc(64, 0),
                MidiMessage::NoteOff(Channel::C1, 60.into(), 17.into())
            ]
        );
        assert_eq!(history.0[0].notes[60], 1);
        assert_eq!(active[0], 1 << 60);
        let final_off = journal(0x0C, &[0, 0x77, 8, 0, 60, 0x91]);
        ChannelRepairs::build_with_state(
            Journal::parse(&final_off).unwrap(),
            1,
            &mut active,
            &mut history,
        )
        .unwrap();
        assert_eq!(history.0[0].notes[60], 0);
        assert_eq!(active[0], 0);
    }

    #[test]
    fn unbounded_or_over_budget_reference_repairs_leave_history_unchanged() {
        for target in [0, 127] {
            let mut history = ChannelHistory::default();
            let mut active = [0; 16];
            history.0[0].notes[60] = 50_000;
            active[0] = 1 << 60;
            let bytes = journal(0x84, &[7, 0x82, 3, 0, 60, target]);
            assert!(
                ChannelRepairs::build_with_state(
                    Journal::parse(&bytes).unwrap(),
                    1,
                    &mut active,
                    &mut history
                )
                .is_none()
            );
            assert!(history.0[0].program.is_none());
            assert_eq!(history.0[0].controls[0].value, None);
            assert_eq!(history.0[0].notes[60], 50_000);
            assert_eq!(active[0], 1 << 60);
        }
    }

    #[test]
    fn general_midi_and_dls_reset_state_commands_invalidate_history() {
        for payload in [
            [0x7E, 0x7F, 9, 0],
            [0x7E, 0x7F, 9, 1],
            [0x7E, 0x7F, 9, 3],
            [0x7E, 0, 10, 1],
            [0x7E, 0, 10, 2],
        ] {
            let mut history = ChannelHistory::default();
            history.observe(MidiMessage::NoteOn(Channel::C1, 60.into(), 100.into()));
            history.observe(cc(7, 100));
            assert!(history.observe_sysex(&payload));
            assert_eq!(history.0[0].notes[60], 0);
            assert_eq!(history.0[0].controls[7].value, None);
        }
    }

    #[test]
    fn count_without_controller_value_rolls_back_instead_of_guessing() {
        for number in [7, 10, 11, 126] {
            let bytes = journal(0xC0, &[9, 0, 0, 0, number, 0xC1]);
            let mut history = ChannelHistory::default();
            history.observe(cc(number, 55));
            let mut active = [0; 16];
            assert!(
                ChannelRepairs::build_with_state(
                    Journal::parse(&bytes).unwrap(),
                    1,
                    &mut active,
                    &mut history,
                )
                .is_none()
            );
            assert!(history.0[0].program.is_none());
            assert_eq!(history.0[0].controls[usize::from(number)].value, Some(55));
        }
    }

    #[test]
    fn repeated_state_recovery_allocates_nothing() {
        let bytes = journal(0xC0, &[127, 0x82, 3, 1, 7, 100, 64, 0x83]);
        let parsed = Journal::parse(&bytes).unwrap();
        let allocations = crate::test_alloc::count_allocations(|| {
            for _ in 0..10_000 {
                let mut history = ChannelHistory::default();
                let mut active = [0; 16];
                let repairs =
                    ChannelRepairs::build_with_state(parsed, 1, &mut active, &mut history).unwrap();
                assert_eq!(repairs.messages().count(), 6);
            }
        });
        assert_eq!(allocations, 0);
    }

    #[test]
    fn unsupported_parameter_context_does_not_partially_apply_a_program() {
        let bytes = journal(0xC0, &[127, 0x82, 3, 0, 6, 64]);
        let mut history = ChannelHistory::default();
        let mut active = [0; 16];
        assert!(
            ChannelRepairs::build_with_state(
                Journal::parse(&bytes).unwrap(),
                1,
                &mut active,
                &mut history
            )
            .is_none()
        );
        assert!(history.0[0].program.is_none());
        assert_eq!(history.0[0].controls[0].value, None);
    }

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
        let unsupported = Journal::parse(&[0x20, 0, 0, 0, 5, 32, 0, 2]).unwrap();
        assert!(ChannelRepairs::build(unsupported, 1, &mut active).is_none());
    }
}
