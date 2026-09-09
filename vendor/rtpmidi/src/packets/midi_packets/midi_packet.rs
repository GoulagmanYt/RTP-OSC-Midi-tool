#[cfg(test)]
use bytes::Bytes;
use bytes::{BufMut, BytesMut};
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    network_endian::{U16, U32},
};

use super::midi_command_iterator::MidiCommandIterator;
use super::midi_command_list_body::MidiEventList;
use crate::packets::midi_packets::{
    midi_command_list_header::MidiCommandListHeader, midi_event::MidiEvent,
    midi_packet_header::MidiPacketHeader,
};

#[derive(FromBytes, KnownLayout, Immutable, Debug)]
#[repr(C)]
pub(crate) struct MidiPacket {
    header: MidiPacketHeader,
    body: [u8],
}

impl MidiPacket {
    pub(crate) const MAX_ENCODED_SIZE: usize = 12 + 2 + 1200;
    pub(crate) fn batch_fits(commands: &[MidiEvent<'_>]) -> bool {
        commands.size(false) <= 1200
            && commands
                .iter()
                .all(|event| event.delta_time() <= 0x0FFF_FFFF)
    }
    pub(crate) fn write_into<'a>(
        buffer: &mut BytesMut,
        sequence_number: U16,
        timestamp: U32,
        ssrc: U32,
        commands: &'a [MidiEvent<'a>],
        z_flag: bool,
    ) -> std::io::Result<()> {
        if !Self::batch_fits(commands) || commands.size(z_flag) > 1200 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "RTP MIDI batch exceeds packet or delta-time limit",
            ));
        }
        let packet_header = MidiPacketHeader::new(sequence_number, timestamp, ssrc);
        let command_list_header = MidiCommandListHeader::build_for(commands, z_flag);
        buffer.clear();
        buffer.put_slice(packet_header.as_bytes());
        command_list_header.write(buffer);
        commands.write(buffer, z_flag);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn new_as_bytes<'a>(
        sequence_number: U16,
        timestamp: U32,
        ssrc: U32,
        commands: &'a [MidiEvent<'a>],
        z_flag: bool,
    ) -> Bytes {
        let mut buffer = BytesMut::with_capacity(Self::MAX_ENCODED_SIZE);
        Self::write_into(
            &mut buffer,
            sequence_number,
            timestamp,
            ssrc,
            commands,
            z_flag,
        )
        .expect("valid test packet");
        buffer.freeze()
    }

    pub fn commands(&self) -> MidiCommandIterator<'_> {
        MidiCommandIterator::new(&self.body)
    }

    pub fn validate(&self) -> std::io::Result<()> {
        MidiCommandListHeader::validate(&self.body)?;
        if let Some(journal) = self.journal_bytes() {
            super::recovery_journal::Journal::parse(journal)?;
        }
        let mut commands = self.commands();
        while commands.try_next()?.is_some() {}
        Ok(())
    }

    pub(crate) fn journal_bytes(&self) -> Option<&[u8]> {
        if self.body[0] & 0x40 == 0 {
            return None;
        }
        let header = MidiCommandListHeader::from_slice(&self.body);
        Some(&self.body[header.size() + header.length()..])
    }

    pub fn sequence_number(&self) -> U16 {
        self.header.sequence_number
    }

    #[allow(dead_code)]
    pub fn timestamp(&self) -> U32 {
        self.header.timestamp
    }

    #[allow(dead_code)]
    pub fn ssrc(&self) -> U32 {
        self.header.ssrc
    }
}

#[cfg(test)]
mod tests {
    use midi_types::{Channel, MidiMessage, Note, Value7};

    use crate::packets::midi_packets::rtp_midi_message::RtpMidiMessage;

    use super::*;

    #[test]
    fn reusable_encoder_keeps_storage_and_rejects_oversized_batches() {
        let mut buffer = BytesMut::with_capacity(MidiPacket::MAX_ENCODED_SIZE);
        let pointer = buffer.as_ptr();
        let capacity = buffer.capacity();
        let payload = [0x01; 1198];
        let commands = [MidiEvent::new(None, RtpMidiMessage::SysEx(&payload))];
        for sequence in 0..10_000 {
            MidiPacket::write_into(
                &mut buffer,
                U16::new(sequence),
                U32::new(0),
                U32::new(1),
                &commands,
                false,
            )
            .unwrap();
            assert_eq!(buffer.len(), MidiPacket::MAX_ENCODED_SIZE);
            assert_eq!(buffer.as_ptr(), pointer);
            assert_eq!(buffer.capacity(), capacity);
            MidiPacket::ref_from_bytes(&buffer)
                .unwrap()
                .validate()
                .unwrap();
        }
        let saved = buffer.clone();
        // The first delta byte would exceed the body limit with Z set.
        assert!(
            MidiPacket::write_into(
                &mut buffer,
                U16::new(0),
                U32::new(0),
                U32::new(1),
                &commands,
                true
            )
            .is_err()
        );
        assert_eq!(buffer, saved);
        let too_large = [MidiEvent::new(None, RtpMidiMessage::SysEx(&[1; 1199]))];
        assert!(
            MidiPacket::write_into(
                &mut buffer,
                U16::new(0),
                U32::new(0),
                U32::new(1),
                &too_large,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn test_midi_packet_creation() {
        let sequence_number = U16::from(1);
        let timestamp = U32::from(2);
        let ssrc = U32::from(3);
        let commands = vec![
            MidiEvent::new(
                None,
                RtpMidiMessage::MidiMessage(MidiMessage::NoteOn(
                    Channel::C1,
                    Note::C4,
                    Value7::from(127),
                )),
            ),
            MidiEvent::new(
                None,
                RtpMidiMessage::MidiMessage(MidiMessage::NoteOff(
                    Channel::C1,
                    Note::C4,
                    Value7::from(0),
                )),
            ),
        ];
        let z_flag = false;

        let packet = MidiPacket::new_as_bytes(sequence_number, timestamp, ssrc, &commands, z_flag);

        let expected = [
            0x80, 0x61, // flags
            0x00, 0x01, // sequence number
            0x00, 0x00, 0x00, 0x02, // timestamp
            0x00, 0x00, 0x00, 0x03, // ssrc
            0x07, // command list flags and length
            0x90, 0x48, 0x7F, // command list header and commands would follow here
            0x00, // delta time
            0x80, 0x48, 0x00, // Note On command for C4
        ];

        assert_eq!(packet.len(), expected.len());
        assert_eq!(&packet[..], &expected);
    }
}
