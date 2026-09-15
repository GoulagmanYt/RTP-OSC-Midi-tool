use super::{ChannelHistory, ChannelRepairs, MidiMessage, Value14};
use crate::sessions::system_state::SequencerState;

fn take<'a>(bytes: &mut &'a [u8], size: usize) -> Option<&'a [u8]> {
    let part = bytes.get(..size)?;
    *bytes = &bytes[size..];
    Some(part)
}

impl ChannelRepairs {
    fn system_message(
        &mut self,
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
        message: MidiMessage,
    ) {
        self.push(message);
        super::observe_received(active, history, message);
    }

    pub(super) fn repair_system(
        &mut self,
        bytes: &[u8],
        active: &mut [u128; 16],
        history: &mut ChannelHistory,
        prefix: Option<&[u8]>,
    ) -> Option<()> {
        let flags = bytes[0];
        let mut data = &bytes[2..];
        let mut prefix_covered = prefix.is_none();
        if flags & 64 != 0 {
            let toc = take(&mut data, 1)?[0];
            // Reserved/undefined system commands cannot be represented by the
            // MIDI 1.0 destination API. Never interpret their future LEGAL data.
            if toc & 15 != 0 {
                return None;
            }
            if toc & 64 != 0 {
                let count = take(&mut data, 1)?[0] & 127;
                if history.1.resets != Some(count) {
                    self.system_message(active, history, MidiMessage::Reset);
                }
                history.1.resets = Some(count);
            }
            if toc & 32 != 0 {
                let count = take(&mut data, 1)?[0] & 127;
                if history.1.tunes != Some(count) {
                    self.system_message(active, history, MidiMessage::TuneRequest);
                }
                history.1.tunes = Some(count);
            }
            if toc & 16 != 0 {
                let song = take(&mut data, 1)?[0] & 127;
                if history.1.song != Some(song) {
                    self.system_message(active, history, MidiMessage::SongSelect(song.into()));
                }
            }
        }
        if flags & 32 != 0 {
            let count = take(&mut data, 1)?[0] & 127;
            if history.1.senses != Some(count) {
                self.system_message(active, history, MidiMessage::ActiveSensing);
            }
            history.1.senses = Some(count);
        }
        if flags & 16 != 0 {
            let toc = take(&mut data, 1)?[0];
            let mut clock = 0u32;
            if toc & 16 != 0 {
                let field = take(&mut data, 2)?;
                clock = u32::from(toc & 7) * 65536
                    + u32::from(u16::from_be_bytes([field[0], field[1]]));
            }
            // TIMETOOLS requires a negotiated nonstandard sequencer and tempo.
            // There is no valid conversion to MIDI Clock without that context.
            if toc & 8 != 0 {
                return None;
            }
            let desired = SequencerState {
                running: toc & 64 != 0,
                clock,
                downbeat: toc & 32 != 0,
                continued: toc & 16 != 0,
            };
            if !desired.downbeat && !clock.is_multiple_of(6) {
                return None;
            }
            if history.1.sequencer != Some(desired) {
                let position = (clock / 6).min(16383);
                let ticks = clock - position * 6 + u32::from(desired.downbeat);
                if ticks as usize + 4 > super::MAX_REPAIRS.saturating_sub(self.len) {
                    return None;
                }
                self.system_message(active, history, MidiMessage::Stop);
                if desired.clock == 0 && !desired.downbeat && !desired.continued {
                    self.system_message(active, history, MidiMessage::Start);
                } else {
                    self.system_message(
                        active,
                        history,
                        MidiMessage::SongPositionPointer(Value14::from(position as u16)),
                    );
                    // A stopped receiver must temporarily run to consume the
                    // residual clocks; finish by restoring the journal's N bit.
                    self.system_message(active, history, MidiMessage::Continue);
                    for _ in 0..ticks {
                        self.system_message(active, history, MidiMessage::TimingClock);
                    }
                }
                if !desired.running {
                    self.system_message(active, history, MidiMessage::Stop);
                }
                history.1.sequencer = Some(desired);
            }
        }
        if flags & 8 != 0 {
            let toc = take(&mut data, 1)?[0];
            if toc & 0x60 == 0 {
                return None;
            }
            if toc & 64 != 0 {
                let field = take(&mut data, 4)?;
                let frame = if toc & 16 != 0 {
                    // Chapter F already compensates forward quarter-frame
                    // latency. Translate to Full Frame without adding it again.
                    let nibble = |index: usize| (field[index / 2] >> (4 * (1 - index % 2))) & 15;
                    [
                        nibble(6) | (nibble(7) << 4),
                        nibble(4) | (nibble(5) << 4),
                        nibble(2) | (nibble(3) << 4),
                        nibble(0) | (nibble(1) << 4),
                    ]
                } else {
                    [field[0], field[1], field[2], field[3]]
                };
                let max_frames = [24, 25, 30, 30][usize::from((frame[0] >> 5) & 3)];
                if frame[0] & 128 != 0
                    || frame[0] & 31 >= 24
                    || frame[1] >= 60
                    || frame[2] >= 60
                    || frame[3] >= max_frames
                {
                    return None;
                }
                if history.1.full_frame != Some(frame) {
                    let payload = [127, 127, 1, 1, frame[0], frame[1], frame[2], frame[3]];
                    self.push_sysex(&payload)?;
                    history.observe_sysex(&payload);
                }
            }
            if toc & 32 != 0 {
                let field = take(&mut data, 4)?;
                let point = usize::from(toc & 7);
                for offset in 0..8 {
                    let index = if toc & 8 != 0 { 7 - offset } else { offset };
                    let nibble = (field[index / 2] >> (4 * (1 - index % 2))) & 15;
                    self.system_message(
                        active,
                        history,
                        MidiMessage::QuarterFrame(((index as u8) * 16 + nibble).into()),
                    );
                    if index == point {
                        break;
                    }
                }
            }
        }
        if flags & 4 != 0 {
            let baseline = history.1.sysex_count;
            while !data.is_empty() {
                let toc = take(&mut data, 1)?[0];
                let type_count = if toc & 64 != 0 {
                    Some(take(&mut data, 1)?[0])
                } else {
                    None
                };
                let count = if toc & 32 != 0 {
                    Some(take(&mut data, 1)?[0])
                } else {
                    None
                };
                let first = if toc & 16 != 0 {
                    let mut first = 0usize;
                    loop {
                        let byte = take(&mut data, 1)?[0];
                        first = first
                            .checked_mul(128)?
                            .checked_add(usize::from(byte & 127))?;
                        if byte & 128 == 0 {
                            break;
                        }
                    }
                    first
                } else {
                    0
                };
                let mut payload = [0u8; 1024];
                let mut len = 0;
                if toc & 8 != 0 {
                    loop {
                        let byte = take(&mut data, 1)?[0];
                        *payload.get_mut(len)? = byte & 127;
                        len += 1;
                        if byte & 128 != 0 {
                            break;
                        }
                    }
                }
                let missing = match (count, baseline) {
                    (Some(next), Some(previous)) => {
                        let distance = next.wrapping_sub(previous);
                        if distance == 128 {
                            return None;
                        }
                        distance != 0 && distance < 128
                    }
                    _ => true,
                };
                let unfinished = toc & 3 == 0;
                let completing_prefix = prefix.is_some() && count.is_some() && count == baseline;
                prefix_covered |= completing_prefix;
                if (missing || unfinished || completing_prefix) && toc & 3 != 1 {
                    if (type_count.is_some() && count.is_none()) || (len == 0 && !unfinished) {
                        return None;
                    }
                    let retained = if first == 0 {
                        &[][..]
                    } else {
                        if !completing_prefix {
                            return None;
                        }
                        prefix?.get(..first)?
                    };
                    if unfinished {
                        if !data.is_empty() {
                            return None;
                        }
                        prefix_covered = true;
                        self.store_pending_sysex(retained, &payload[..len])?;
                    } else if retained.is_empty() {
                        self.push_sysex(&payload[..len])?;
                        if history.observe_sysex(&payload[..len]) {
                            active.fill(0);
                        }
                    } else {
                        let start = self.sysex_len;
                        let end = start.checked_add(retained.len())?.checked_add(len)?;
                        if end > self.sysex.len() || self.len >= super::MAX_REPAIRS {
                            return None;
                        }
                        self.sysex[start..start + retained.len()].copy_from_slice(retained);
                        self.sysex[start + retained.len()..end].copy_from_slice(&payload[..len]);
                        self.messages[self.len] = Some(super::Repair::SysEx {
                            start: start as u32,
                            len: (end - start) as u32,
                        });
                        self.len += 1;
                        self.sysex_len = end;
                        if history.observe_sysex(&self.sysex[start..end]) {
                            active.fill(0);
                        }
                    }
                }
                if let Some(count) = count {
                    history.1.sysex_count = Some(count);
                }
            }
        }
        if data.is_empty() && prefix_covered {
            Some(())
        } else {
            None
        }
    }
}
