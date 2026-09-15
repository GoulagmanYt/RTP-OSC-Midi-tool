use super::channel_state::{ChannelHistory, ProgramState};
use crate::packets::midi_packets::recovery_journal::{Journal, note_layout};
use midi_types::{Channel, Control, MidiMessage, Note, Value7, Value14};

// Fixed work budget covers ordinary complete channel journals. Large note
// reference-count repairs must also fit; otherwise the plan is rolled back.
const MAX_REPAIRS: usize = 16 * 519;

#[derive(Clone, Copy)]
enum Repair {
    Midi(MidiMessage),
    SysEx { start: u32, len: u32 },
}

pub(crate) enum RepairEvent<'a> {
    Midi(MidiMessage),
    SysEx(&'a [u8]),
}

#[path = "system_recovery.rs"]
mod system_recovery;

pub(crate) struct ChannelRepairs {
    messages: Box<[Option<Repair>]>,
    sysex: Box<[u8]>,
    pending_sysex: Option<(usize, usize)>,
    sysex_len: usize,
    len: usize,
    overflow: bool,
}
impl ChannelRepairs {
    pub(super) fn new() -> Self {
        Self {
            messages: vec![None; MAX_REPAIRS].into_boxed_slice(),
            sysex: vec![0; super::sysex::MAX_SYSEX_BYTES].into_boxed_slice(),
            pending_sysex: None,
            sysex_len: 0,
            len: 0,
            overflow: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn build(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
    ) -> Option<Self> {
        Self::build_with_state(journal, expected, active, &mut ChannelHistory::default())
    }

    #[cfg(test)]
    pub(super) fn build_with_state(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
    ) -> Option<Self> {
        Self::build_with_prefix(journal, expected, active, history, None)
    }

    #[cfg(test)]
    pub(super) fn build_with_prefix(
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
        prefix: Option<&[u8]>,
    ) -> Option<Self> {
        let mut repairs = Self::new();
        repairs.rebuild(journal, expected, active, history, prefix)?;
        Some(repairs)
    }

    pub(super) fn rebuild(
        &mut self,
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
        prefix: Option<&[u8]>,
    ) -> Option<()> {
        if prefix.is_some() && journal.system_bytes().is_none_or(|bytes| bytes[0] & 4 == 0) {
            return None;
        }
        let saved_active = *active;
        let saved_history = *history;
        self.len = 0;
        self.sysex_len = 0;
        self.pending_sysex = None;
        self.overflow = false;
        let plan = self.build_plan(journal, expected, active, history, prefix);
        if plan.is_none() {
            *active = saved_active;
            *history = saved_history;
        }
        plan
    }

    fn build_plan(
        &mut self,
        journal: Journal<'_>,
        expected: u16,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
        prefix: Option<&[u8]>,
    ) -> Option<()> {
        if expected.wrapping_sub(journal.checkpoint()) >= 0x8000 {
            return None;
        }
        let channels = journal.state_channels()?;
        let repairs = self;
        if let Some(system) = journal.system_bytes() {
            repairs.repair_system(system, active, history, prefix)?;
        }
        for (channel, toc, enhanced, mut data) in channels {
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
                if enhanced {
                    repairs.repair_enhanced_controllers(history, channel, &data[1..size])?;
                } else {
                    repairs.repair_controllers(history, channel, &data[1..size])?;
                }
                data = &data[size..];
            }
            if toc & 32 != 0 {
                let size = (usize::from(data[0] & 3) << 8) | usize::from(data[1]);
                repairs.repair_parameters(history, channel, &data[..size])?;
                data = &data[size..];
            }
            for message in repairs.messages[channel_start..repairs.len]
                .iter()
                .flatten()
            {
                if let Repair::Midi(message) = message {
                    observe(active, *message);
                }
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
        if repairs.overflow { None } else { Some(()) }
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

    fn select_parameter(&mut self, history: &mut ChannelHistory, channel: u8, key: u16) {
        let msb = if key & 16384 != 0 { 99 } else { 101 };
        self.controller(history, channel, msb, ((key >> 7) & 127) as u8);
        self.controller(history, channel, msb - 1, (key & 127) as u8);
    }

    fn repair_parameters(
        &mut self,
        history: &mut ChannelHistory,
        channel: u8,
        bytes: &[u8],
    ) -> Option<()> {
        let index = usize::from(channel);
        let mut last = None;
        for log in crate::packets::midi_packets::parameter_journal::logs(bytes) {
            last = Some(log.key);
            let previous = history.0[index].parameters.get(log.key);
            let msb = log.msb.or(previous.msb);
            let lsb = log.lsb.or(if log.msb.is_some() {
                Some(0)
            } else {
                previous.lsb
            });
            let buttons = log.buttons.or(if log.msb.is_some() || log.lsb.is_some() {
                Some(0)
            } else {
                previous.buttons
            });
            let missing_transactions = match (log.count, previous.count) {
                (Some(next), Some(old)) => usize::from(next.wrapping_sub(old) & 127),
                (Some(_), None) if !log.value_tool => return None,
                _ => 0,
            };
            let rebase = log.value_tool
                && (msb != previous.msb
                    || lsb != previous.lsb
                    || ((log.msb.is_some() || log.lsb.is_some()) && buttons != previous.buttons));
            let relative = if rebase {
                i32::from(buttons.unwrap_or(0))
            } else {
                match (buttons, previous.buttons) {
                    (Some(next), Some(old)) => i32::from(next) - i32::from(old),
                    (Some(0), None) => 0,
                    (Some(_), None) => return None,
                    _ => 0,
                }
            };
            if rebase && msb.is_none() {
                return None;
            }
            let changes_value = rebase || relative != 0;
            let selections = missing_transactions.max(usize::from(changes_value));
            let required =
                selections * 2 + if rebase { 2 } else { 0 } + relative.unsigned_abs() as usize;
            if required + 4 > MAX_REPAIRS.saturating_sub(self.len) {
                return None;
            }
            for _ in 0..selections {
                self.select_parameter(history, channel, log.key);
            }
            if rebase {
                self.controller(history, channel, 6, msb?);
                if let Some(lsb) = lsb {
                    self.controller(history, channel, 38, lsb);
                }
            }
            for _ in 0..relative.unsigned_abs() {
                self.controller(history, channel, if relative >= 0 { 96 } else { 97 }, 0);
            }
            let mut state = history.0[index].parameters.get(log.key);
            state.msb = msb;
            state.lsb = lsb;
            state.buttons = buttons;
            // Generated selector commands must not become source transaction counts.
            state.count = log.count.or(if selections == 0 {
                previous.count
            } else {
                None
            });
            history.0[index].parameters.set(log.key, state);
        }
        if bytes[0] & 64 != 0 {
            let pending = (bytes[2] & 128 != 0, bytes[2] & 127);
            if history.0[index].parameters.pending != Some(pending) {
                self.controller(
                    history,
                    channel,
                    if pending.0 { 99 } else { 101 },
                    pending.1,
                );
            }
        } else if bytes[0] & 32 != 0 {
            let key = last?;
            if history.0[index].parameters.selected != Some(key) {
                let saved = history.0[index].parameters.get(key);
                self.select_parameter(history, channel, key);
                history.0[index].parameters.set(key, saved);
            }
        } else if history.0[index].parameters.selected.is_some()
            || history.0[index].parameters.pending.is_some()
        {
            self.select_parameter(history, channel, 16383);
        }
        Some(())
    }

    fn repair_enhanced_controllers(
        &mut self,
        history: &mut ChannelHistory,
        channel: u8,
        logs: &[u8],
    ) -> Option<()> {
        #[derive(Clone, Copy, Default)]
        struct Command {
            number: u8,
            count: Option<u8>,
            value: Option<u8>,
            toggle: Option<u8>,
        }
        let mut commands = [Command::default(); 128];
        let mut count = 0usize;
        let mut last_tool = 3;
        for log in logs.chunks_exact(2) {
            let number = log[0] & 127;
            let tool = match log[1] & 0xC0 {
                0xC0 => 0,
                0x80 => 2,
                _ => 1,
            };
            if count == 0 || commands[count - 1].number != number || tool <= last_tool {
                commands[count] = Command {
                    number,
                    ..Command::default()
                };
                count += 1;
            }
            let command = &mut commands[count - 1];
            match tool {
                0 => command.count = Some(log[1] & 63),
                1 => command.value = Some(log[1] & 127),
                _ => command.toggle = Some(log[1] & 63),
            }
            last_tool = tool;
        }
        let mut inferred = [None; 128];
        let mut masks = [None; 128];
        let baseline = history.0[usize::from(channel)].controls;
        for command in &commands[..count] {
            let number = usize::from(command.number);
            let repeated = commands[..count]
                .iter()
                .filter(|c| c.number == command.number)
                .count()
                > 1;
            if !repeated {
                let mut raw = [0u8; 6];
                let mut len = 0;
                for value in [
                    command.count.map(|v| v | 0xC0),
                    command.value,
                    command.toggle.map(|v| v | 0x80),
                ]
                .into_iter()
                .flatten()
                {
                    raw[len] = command.number;
                    raw[len + 1] = value;
                    len += 2;
                }
                self.repair_controllers(history, channel, &raw[..len])?;
                continue;
            }
            let next = match inferred[number] {
                Some(previous) => {
                    let expected = (previous + 1) & 63;
                    if command.count.is_some_and(|count| count != expected) {
                        return None;
                    }
                    expected
                }
                None => command.count?,
            };
            inferred[number] = Some(next);
            let mask = (command.value.is_some(), command.toggle.is_some());
            if masks[number].is_some_and(|old| old != mask) {
                return None;
            }
            masks[number] = Some(mask);
            if command
                .value
                .zip(command.toggle)
                .is_some_and(|(v, t)| (v >= 64) != (t & 1 != 0))
            {
                return None;
            }
            if !baseline[number].count_known {
                return None;
            }
            let distance = next.wrapping_sub(baseline[number].count) & 63;
            if distance == 32 {
                return None;
            }
            if distance == 0 || distance > 32 {
                continue;
            }
            let value = command
                .value
                .or(command.toggle.map(|v| if v & 1 != 0 { 127 } else { 0 }))
                .or_else(|| matches!(command.number, 120 | 121 | 123..=125 | 127).then_some(0))?;
            self.controller(history, channel, command.number, value);
            let current = &mut history.0[usize::from(channel)].controls[number];
            current.count = next;
            current.count_known = true;
            if let Some(toggle) = command.toggle {
                current.toggles = toggle;
                current.toggles_known = true;
            }
        }
        Some(())
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
            if should_emit
                && matches!(number, 6 | 38 | 96 | 97)
                && (history.0[usize::from(channel)]
                    .parameters
                    .selected
                    .is_some()
                    || history.0[usize::from(channel)].parameters.pending.is_some())
            {
                // Chapter C codes these as general-purpose commands. Close the
                // parameter transaction first; Chapter M restores its final state.
                self.select_parameter(history, channel, 16383);
            }
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
            *slot = Some(Repair::Midi(message));
            self.len += 1;
        } else {
            self.overflow = true;
        }
    }
    fn push_sysex(&mut self, payload: &[u8]) -> Option<()> {
        let end = self.sysex_len.checked_add(payload.len())?;
        if end > self.sysex.len() || self.len >= MAX_REPAIRS {
            return None;
        }
        self.sysex[self.sysex_len..end].copy_from_slice(payload);
        self.messages[self.len] = Some(Repair::SysEx {
            start: self.sysex_len as u32,
            len: payload.len() as u32,
        });
        self.len += 1;
        self.sysex_len = end;
        Some(())
    }
    fn store_pending_sysex(&mut self, prefix: &[u8], suffix: &[u8]) -> Option<()> {
        let start = self.sysex_len;
        let end = start.checked_add(prefix.len())?.checked_add(suffix.len())?;
        if end > self.sysex.len() || self.pending_sysex.is_some() {
            return None;
        }
        self.sysex[start..start + prefix.len()].copy_from_slice(prefix);
        self.sysex[start + prefix.len()..end].copy_from_slice(suffix);
        self.sysex_len = end;
        self.pending_sysex = Some((start, end));
        Some(())
    }
    pub(crate) fn pending_sysex(&self) -> Option<&[u8]> {
        self.pending_sysex
            .map(|(start, end)| &self.sysex[start..end])
    }
    pub(crate) fn events(&self) -> impl Iterator<Item = RepairEvent<'_>> {
        self.messages[..self.len]
            .iter()
            .flatten()
            .map(|message| match message {
                Repair::Midi(message) => RepairEvent::Midi(*message),
                Repair::SysEx { start, len } => RepairEvent::SysEx(
                    &self.sysex[*start as usize..*start as usize + *len as usize],
                ),
            })
    }
    #[cfg(test)]
    pub(crate) fn messages(&self) -> impl Iterator<Item = MidiMessage> + '_ {
        self.messages[..self.len]
            .iter()
            .copied()
            .flatten()
            .filter_map(|message| match message {
                Repair::Midi(message) => Some(message),
                _ => None,
            })
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
        let mut repairs = ChannelRepairs::new();
        let allocations = crate::test_alloc::count_allocations(|| {
            for _ in 0..10_000 {
                let mut history = ChannelHistory::default();
                let mut active = [0; 16];
                repairs
                    .rebuild(parsed, 1, &mut active, &mut history, None)
                    .unwrap();
                assert_eq!(repairs.messages().count(), 6);
            }
        });
        assert_eq!(allocations, 0);
    }

    #[test]
    fn unsupported_parameter_context_does_not_partially_apply_a_program() {
        let bytes = journal(0xC0, &[127, 0x82, 3, 0, 98, 64]);
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
        let closed_parameter = Journal::parse(&[0x20, 0, 0, 0, 5, 32, 0, 2]).unwrap();
        assert!(ChannelRepairs::build(closed_parameter, 1, &mut active).is_some());
    }
}

#[cfg(test)]
#[path = "advanced_recovery_tests.rs"]
mod advanced_recovery_tests;
