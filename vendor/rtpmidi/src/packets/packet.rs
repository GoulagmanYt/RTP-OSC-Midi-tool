use zerocopy::FromBytes;

use super::{
    control_packets::control_packet::ControlPacket, midi_packets::midi_packet::MidiPacket,
};

#[derive(Debug)]
pub(crate) enum RtpMidiPacket<'a> {
    Midi(&'a MidiPacket),
    Control(ControlPacket<'a>),
}

impl<'a> RtpMidiPacket<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, std::io::Error> {
        if ControlPacket::is_control_packet(bytes) {
            let packet = ControlPacket::try_from_bytes(bytes).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Failed to parse Control packet",
                )
            })?;
            Ok(RtpMidiPacket::Control(packet))
        } else {
            if bytes.len() < 13 || bytes[0] != 0x80 || bytes[1] & 0x7F != 0x61 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unsupported RTP header",
                ));
            }
            let (packet, _remaining) = MidiPacket::ref_from_prefix(bytes).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Failed to parse MIDI packet",
                )
            })?;
            packet.validate()?;
            Ok(RtpMidiPacket::Midi(packet))
        }
    }
}

#[cfg(test)]
mod tests {
    use midi_types::{Channel, MidiMessage, Note, Value7};
    use zerocopy::U16;
    use zerocopy::network_endian::U32;

    use super::*;
    use crate::packets::midi_packets::midi_event::MidiEvent;
    use crate::packets::midi_packets::rtp_midi_message::RtpMidiMessage;

    fn raw(body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x80, 0x61, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn rejects_truncated_and_malformed_packets_without_panicking() {
        let valid = raw(&[3, 0x90, 60, 100]);
        for size in 0..valid.len() {
            assert!(RtpMidiPacket::parse(&valid[..size]).is_err());
        }
        for body in [
            &[0x80][..],
            &[3, 0x90, 60],
            &[1, 0x90],
            &[3, 0x90, 255, 255],
            &[0x26, 0x80, 0x80, 0x80, 0x80, 0x80, 0],
            &[0x40],
        ] {
            assert!(RtpMidiPacket::parse(&raw(body)).is_err(), "{body:?}");
        }
        // Deterministic malformed-input campaign, including all header bytes.
        let mut seed = 0x12345678u32;
        for length in 0..128 {
            for _ in 0..100 {
                let mut bytes = vec![0; length];
                for byte in &mut bytes {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    *byte = seed as u8;
                }
                let _ = RtpMidiPacket::parse(&bytes);
                if length >= 12 {
                    bytes[..12].copy_from_slice(&valid[..12]);
                    let _ = RtpMidiPacket::parse(&bytes);
                }
            }
        }
    }

    #[test]
    fn running_status_survives_realtime_and_zero_velocity_normalization() {
        let packet = raw(&[12, 0x90, 60, 100, 0, 0xF8, 0, 60, 0, 0, 61, 100, 0]);
        let RtpMidiPacket::Midi(packet) = RtpMidiPacket::parse(&packet).unwrap() else {
            panic!()
        };
        let events: Vec<_> = packet.commands().collect();
        assert_eq!(events.len(), 4);
        assert!(matches!(
            events[1].command(),
            RtpMidiMessage::MidiMessage(MidiMessage::TimingClock)
        ));
        assert!(matches!(
            events[2].command(),
            RtpMidiMessage::MidiMessage(MidiMessage::NoteOff(..))
        ));
        assert!(matches!(
            events[3].command(),
            RtpMidiMessage::MidiMessage(MidiMessage::NoteOn(..))
        ));
    }

    #[test]
    fn z_flag_and_pitch_bend_use_independent_golden_vectors() {
        let packet = raw(&[0x25, 0x81, 0, 0xE0, 0, 0x40]);
        let RtpMidiPacket::Midi(packet) = RtpMidiPacket::parse(&packet).unwrap() else {
            panic!()
        };
        let event = packet.commands().next().unwrap();
        assert_eq!(event.delta_time(), 128);
        let RtpMidiMessage::MidiMessage(MidiMessage::PitchBendChange(_, value)) = event.command()
        else {
            panic!()
        };
        assert_eq!(u16::from(*value), 8192);
        let encoded =
            MidiPacket::new_as_bytes(U16::new(1), U32::new(0), U32::new(1), &[event], true);
        assert_eq!(&encoded[12..], &[0x25, 0x81, 0, 0xE0, 0, 0x40]);
    }

    #[test]
    fn sysex_preserves_payload_and_following_command() {
        let packet = raw(&[9, 0xF0, 0x7D, 1, 2, 0xF7, 0, 0x90, 60, 100]);
        let RtpMidiPacket::Midi(packet) = RtpMidiPacket::parse(&packet).unwrap() else {
            panic!()
        };
        let events: Vec<_> = packet.commands().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].command(), &RtpMidiMessage::SysEx(&[0x7D, 1, 2]));
    }

    #[test]
    fn realtime_bytes_inside_channel_messages_do_not_consume_data_or_running_status() {
        let bytes = raw(&[11, 0x90, 60, 0xF8, 0xFA, 100, 0, 61, 0xFC, 0xFE, 0, 0]);
        let RtpMidiPacket::Midi(packet) = RtpMidiPacket::parse(&bytes).unwrap() else {
            panic!()
        };
        let events: Vec<_> = packet.commands().collect();
        assert_eq!(events.len(), 6);
        assert!(
            matches!(events[2].command(), RtpMidiMessage::MidiMessage(MidiMessage::NoteOn(_, note, _)) if u8::from(*note) == 60)
        );
        assert!(
            matches!(events[5].command(), RtpMidiMessage::MidiMessage(MidiMessage::NoteOff(_, note, _)) if u8::from(*note) == 61)
        );
    }

    #[test]
    fn test_parse_midi_packet() {
        let commands = vec![MidiEvent::new(
            None,
            RtpMidiMessage::MidiMessage(MidiMessage::NoteOn(
                Channel::C1,
                Note::C4,
                Value7::from(127),
            )),
        )];
        let packet =
            MidiPacket::new_as_bytes(U16::new(1), U32::new(2), U32::new(3), &commands, false);

        let parsed_packet = RtpMidiPacket::parse(&packet).unwrap();
        if let RtpMidiPacket::Midi(parsed_midi_packet) = parsed_packet {
            assert_eq!(parsed_midi_packet.sequence_number(), 1);
            assert_eq!(parsed_midi_packet.timestamp(), 2);
            assert_eq!(parsed_midi_packet.ssrc(), 3);
            let values = parsed_midi_packet.commands().collect::<Vec<_>>();
            assert_eq!(values.len(), 1);
            assert_eq!(
                values[0].command().to_owned(),
                RtpMidiMessage::MidiMessage(MidiMessage::NoteOn(
                    Channel::C1,
                    Note::C4,
                    Value7::from(127)
                ))
            );
        } else {
            panic!("Expected MidiPacket");
        }
    }

    // #[test]
    // fn test_parse_control_packet() {
    //     let packet = ControlPacket::new_acceptance(U32::new(1), U32::new(1), c"Test Name");
    //     let parsed = RtpMidiPacket::parse(&packet).unwrap();

    //     match parsed {
    //         RtpMidiPacket::Control(ControlPacket::Acceptance { body: _, name: _ }) => {
    //             // all good
    //         }
    //         _ => panic!("Expected ControlPacket"),
    //     }
    // }
}
