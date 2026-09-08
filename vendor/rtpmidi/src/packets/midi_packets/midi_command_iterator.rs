use crate::packets::midi_packets::midi_event::MidiEvent;

use super::midi_command_list_header::MidiCommandListHeader;

#[derive(Debug)]
pub(crate) struct MidiCommandIterator<'a> {
    data: &'a [u8],
    running_status: Option<u8>,
    read_delta_time: bool,
    partial: Option<PartialCommand>,
}

#[derive(Debug)]
struct PartialCommand {
    status: u8,
    data: [u8; 2],
    used: usize,
    required: usize,
    delta: u32,
}

impl<'a> MidiCommandIterator<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let command_list_header = MidiCommandListHeader::from_slice(data);
        let read_delta_time = command_list_header.flags().z_flag();
        let offset = command_list_header.size();
        let length = command_list_header.length();
        let slice = &data[offset..length + offset];
        MidiCommandIterator {
            data: slice,
            running_status: None,
            read_delta_time,
            partial: None,
        }
    }

    pub fn try_next(&mut self) -> std::io::Result<Option<MidiEvent<'a>>> {
        use super::{midi_message_ext::ReadWriteExt, rtp_midi_message::RtpMidiMessage};
        use midi_types::MidiMessage;
        let invalid = || {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Truncated or invalid MIDI command",
            )
        };
        if self.partial.is_none() {
            if self.data.is_empty() {
                return Ok(None);
            }
            let (delta, bytes) = if self.read_delta_time {
                super::delta_time::read_delta_time(self.data)?
            } else {
                (0, self.data)
            };
            self.data = bytes;
            // A trailing delta without a command is valid RTP-MIDI.
            if bytes.is_empty() {
                return Ok(None);
            }
            let explicit = bytes[0] >= 0x80;
            let status = if explicit {
                bytes[0]
            } else {
                self.running_status.ok_or_else(invalid)?
            };
            match status {
                0x80..=0xEF => self.running_status = Some(status),
                0xF0..=0xF7 => self.running_status = None,
                _ => {}
            }
            self.read_delta_time = true;
            if status == 0xF0 || status >= 0xF6 {
                let (event, remaining) =
                    MidiEvent::from_be_bytes(bytes, false, self.running_status)?;
                self.data = remaining;
                return Ok(Some(MidiEvent::new(Some(delta), event.command().clone())));
            }
            let required = match status {
                0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 2,
                0xC0..=0xDF | 0xF1 | 0xF3 => 1,
                _ => return Err(invalid()),
            };
            self.data = &bytes[usize::from(explicit)..];
            self.partial = Some(PartialCommand {
                status,
                data: [0; 2],
                used: 0,
                required,
                delta,
            });
        }
        let partial = self.partial.as_mut().expect("partial command initialized");
        while partial.used < partial.required {
            let byte = *self.data.first().ok_or_else(invalid)?;
            self.data = &self.data[1..];
            if byte >= 0xF8 {
                let message = match byte {
                    0xF8 => MidiMessage::TimingClock,
                    0xFA => MidiMessage::Start,
                    0xFB => MidiMessage::Continue,
                    0xFC => MidiMessage::Stop,
                    0xFE => MidiMessage::ActiveSensing,
                    0xFF => MidiMessage::Reset,
                    _ => return Err(invalid()),
                };
                let delta = std::mem::take(&mut partial.delta);
                return Ok(Some(MidiEvent::new(
                    Some(delta),
                    RtpMidiMessage::MidiMessage(message),
                )));
            }
            if byte >= 0x80 {
                return Err(invalid());
            }
            partial.data[partial.used] = byte;
            partial.used += 1;
        }
        let partial = self.partial.take().expect("completed MIDI command");
        let (command, _) = MidiMessage::from_status_byte(
            partial.status,
            partial.status & 0x0F,
            &partial.data[..partial.required],
        )?;
        let RtpMidiMessage::MidiMessage(message) = command else {
            return Err(invalid());
        };
        Ok(Some(MidiEvent::new(
            Some(partial.delta),
            RtpMidiMessage::MidiMessage(message),
        )))
    }
}

impl<'a> Iterator for MidiCommandIterator<'a> {
    type Item = MidiEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.try_next() {
            Ok(event) => event,
            Err(_) => {
                self.data = &[];
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use midi_types::{Channel, MidiMessage, Note};

    use crate::packets::midi_packets::rtp_midi_message::RtpMidiMessage;

    use super::*;

    #[test]
    fn test_midi_command_iterator() {
        let data = &[
            70, 145, 65, 0, 11, 62, 0, 32, 126, 37, 8, 12, 8, 131, 136, 62, 83, 193, 93, 197, 83,
            144,
        ];
        let iterator = MidiCommandIterator::new(data);
        let events = iterator.collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].delta_time(), 0);
        assert_eq!(events[1].delta_time(), 11);

        let RtpMidiMessage::MidiMessage(MidiMessage::NoteOff(channel, key, velocity)) =
            events[0].command()
        else {
            panic!("Unexpected MIDI command")
        };
        assert_eq!(*channel, Channel::from(1));
        assert_eq!(*key, Note::from(65));
        assert_eq!(*velocity, Into::into(0));

        let RtpMidiMessage::MidiMessage(MidiMessage::NoteOff(channel, key, velocity)) =
            events[1].command()
        else {
            panic!("Unexpected MIDI command")
        };
        assert_eq!(*channel, Channel::from(1));
        assert_eq!(*key, Note::from(62));
        assert_eq!(*velocity, Into::into(0));
    }
}
