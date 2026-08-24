pub mod rtp_advertisement;
pub mod rtp_discovery;
pub mod rtp_midi;
pub mod rtp_server;

pub use rtp_discovery::RtpDiscoveryManager;
#[cfg(test)]
use rtp_midi::{midi_to_bytes, participant_matches_target};
pub use rtp_server::{rtp_dropped_count, RtpRemoteTarget, RtpServer};

pub fn ports_available(port: u16) -> Result<bool, String> {
    rtp_discovery::ports_available(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use midi_types::{
        status, Channel, Control, MidiMessage as RtMidiMessage, Note, Program, Value14, Value7,
    };
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

    #[test]
    fn converts_channel_messages_without_filtering() {
        let note = RtMidiMessage::NoteOn(Channel::C1, Note::new(60), Value7::new(100));
        assert_eq!(
            midi_to_bytes(note).as_slice(),
            [status::NOTE_ON | u8::from(Channel::C1), 60, 100]
        );

        let cc = RtMidiMessage::ControlChange(Channel::C10, Control::new(74), Value7::new(42));
        assert_eq!(
            midi_to_bytes(cc).as_slice(),
            [status::CONTROL_CHANGE | u8::from(Channel::C10), 74, 42]
        );

        let prog = RtMidiMessage::ProgramChange(Channel::C5, Program::new(10));
        assert_eq!(
            midi_to_bytes(prog).as_slice(),
            [status::PROGRAM_CHANGE | u8::from(Channel::C5), 10]
        );
    }

    #[test]
    fn keeps_raw_order_for_14bit_and_system_messages() {
        let bend = RtMidiMessage::PitchBendChange(Channel::C2, Value14::from((2, 1)));
        assert_eq!(
            midi_to_bytes(bend).as_slice(),
            [status::PITCH_BEND_CHANGE | u8::from(Channel::C2), 2, 1]
        );

        let spp = RtMidiMessage::SongPositionPointer(Value14::from((4, 3)));
        assert_eq!(
            midi_to_bytes(spp).as_slice(),
            [status::SONG_POSITION_POINTER, 4, 3]
        );

        assert_eq!(
            midi_to_bytes(RtMidiMessage::Start).as_slice(),
            [status::START]
        );
        assert_eq!(
            midi_to_bytes(RtMidiMessage::Stop).as_slice(),
            [status::STOP]
        );
    }

    #[test]
    fn detects_busy_rtp_ports() {
        let mut base = 52000u16;
        let (s1, s2) = loop {
            if let Ok(sock1) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, base)) {
                if let Ok(sock2) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, base + 1)) {
                    break (sock1, sock2);
                }
            }
            base = base.saturating_add(2);
            assert!(base < 65000, "Could not find free port pair for test");
        };

        assert!(!ports_available(base).unwrap());
        drop(s1);
        drop(s2);
        assert!(ports_available(base).unwrap());
    }

    #[test]
    fn participant_match_accepts_control_and_data_ports_for_same_host() {
        let target: SocketAddr = "192.168.1.50:5004".parse().expect("target parse");
        assert!(participant_matches_target("192.168.1.50:5004", &target));
        assert!(participant_matches_target("192.168.1.50:5005", &target));
        assert!(!participant_matches_target("192.168.1.50:5006", &target));
        assert!(!participant_matches_target("192.168.1.51:5004", &target));
    }

    #[test]
    fn participant_match_rejects_invalid_socket_addr_text() {
        let target: SocketAddr = "127.0.0.1:5004".parse().expect("target parse");
        assert!(!participant_matches_target("not-an-addr", &target));
    }
}
