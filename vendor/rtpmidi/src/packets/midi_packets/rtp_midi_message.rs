use bytes::BufMut;
use midi_types::MidiMessage;

use crate::packets::midi_packets::midi_message_ext::ReadWriteExt;

#[derive(Debug, Clone, PartialEq)]
pub enum RtpMidiMessage<'a> {
    MidiMessage(MidiMessage),
    SysEx(&'a [u8]),
    /// RFC 6295 SysEx sublist, including its segmentation delimiters.
    SysExSegment {
        head: u8,
        data: &'a [u8],
        tail: u8,
    },
}

impl From<MidiMessage> for RtpMidiMessage<'_> {
    fn from(msg: MidiMessage) -> Self {
        RtpMidiMessage::MidiMessage(msg)
    }
}

impl<'a> RtpMidiMessage<'a> {
    /// Parse one complete MIDI wire message; segmented RTP sublists are excluded.
    pub fn parse_complete(bytes: &'a [u8]) -> std::io::Result<Self> {
        let (message, remaining) = MidiMessage::from_be_bytes(bytes, None)?;
        if !remaining.is_empty() || matches!(message, Self::SysExSegment { .. }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Expected one complete MIDI message",
            ));
        }
        Ok(message)
    }

    pub fn len(&self) -> usize {
        match self {
            RtpMidiMessage::MidiMessage(msg) => msg.len(),
            RtpMidiMessage::SysEx(data) | RtpMidiMessage::SysExSegment { data, .. } => {
                data.len() + 2
            }
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn write(&self, bytes: &mut bytes::BytesMut, running_status: Option<u8>) {
        match self {
            RtpMidiMessage::MidiMessage(msg) => msg.write(bytes, running_status),
            RtpMidiMessage::SysExSegment { head, data, tail } => {
                bytes.put_u8(*head);
                bytes.extend_from_slice(data);
                bytes.put_u8(*tail);
            }
            RtpMidiMessage::SysEx(data) => {
                bytes.put_u8(0xF0); // SysEx start byte
                bytes.extend_from_slice(data);
                bytes.put_u8(0xF7); // SysEx end byte
            }
        }
    }

    pub(crate) fn status(&self) -> u8 {
        match self {
            RtpMidiMessage::MidiMessage(msg) => msg.status(),
            RtpMidiMessage::SysExSegment { head, .. } => *head,
            RtpMidiMessage::SysEx(_) => 0xF0, // SysEx messages have a special status byte
        }
    }
}
