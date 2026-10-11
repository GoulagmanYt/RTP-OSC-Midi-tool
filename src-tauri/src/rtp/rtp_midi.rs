use midi_types::{status, MidiMessage as RtMidiMessage};
use smallvec::SmallVec;
use std::net::SocketAddr;

pub fn midi_to_bytes(message: RtMidiMessage) -> SmallVec<[u8; 32]> {
    use RtMidiMessage::*;
    match message {
        NoteOn(channel, note, velocity) => SmallVec::from_slice(&[
            (if u8::from(velocity) == 0 {
                status::NOTE_OFF
            } else {
                status::NOTE_ON
            }) | u8::from(channel),
            u8::from(note),
            u8::from(velocity),
        ]),
        NoteOff(channel, note, velocity) => SmallVec::from_slice(&[
            status::NOTE_OFF | u8::from(channel),
            u8::from(note),
            u8::from(velocity),
        ]),
        KeyPressure(channel, note, pressure) => SmallVec::from_slice(&[
            status::KEY_PRESSURE | u8::from(channel),
            u8::from(note),
            u8::from(pressure),
        ]),
        ControlChange(channel, control, value) => SmallVec::from_slice(&[
            status::CONTROL_CHANGE | u8::from(channel),
            u8::from(control),
            u8::from(value),
        ]),
        ProgramChange(channel, program) => SmallVec::from_slice(&[
            status::PROGRAM_CHANGE | u8::from(channel),
            u8::from(program),
        ]),
        ChannelPressure(channel, pressure) => SmallVec::from_slice(&[
            status::CHANNEL_PRESSURE | u8::from(channel),
            u8::from(pressure),
        ]),
        PitchBendChange(channel, bend) => {
            let (b1, b2): (u8, u8) = bend.into();
            SmallVec::from_slice(&[status::PITCH_BEND_CHANGE | u8::from(channel), b2, b1])
        }
        QuarterFrame(frame) => SmallVec::from_slice(&[status::QUARTER_FRAME, frame.into()]),
        SongPositionPointer(pos) => {
            let (b1, b2): (u8, u8) = pos.into();
            SmallVec::from_slice(&[status::SONG_POSITION_POINTER, b2, b1])
        }
        SongSelect(song) => SmallVec::from_slice(&[status::SONG_SELECT, song.into()]),
        TuneRequest => SmallVec::from_slice(&[status::TUNE_REQUEST]),
        TimingClock => SmallVec::from_slice(&[status::TIMING_CLOCK]),
        Start => SmallVec::from_slice(&[status::START]),
        Continue => SmallVec::from_slice(&[status::CONTINUE]),
        Stop => SmallVec::from_slice(&[status::STOP]),
        ActiveSensing => SmallVec::from_slice(&[status::ACTIVE_SENSING]),
        Reset => SmallVec::from_slice(&[status::RESET]),
    }
}

fn normalize_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V4(v4) => std::net::IpAddr::V4(v4),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                std::net::IpAddr::V4(v4)
            } else {
                std::net::IpAddr::V6(v6)
            }
        }
    }
}

pub fn participant_matches_target(participant_addr: &str, target: &SocketAddr) -> bool {
    let Ok(participant) = participant_addr.parse::<SocketAddr>() else {
        return false;
    };
    if normalize_ip(participant.ip()) != normalize_ip(target.ip()) {
        return false;
    }
    let tp = target.port();
    let pp = participant.port();
    pp == tp || (tp.checked_add(1) == Some(pp)) || (tp.checked_sub(1) == Some(pp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_ipv4_and_mapped_ipv6_targets() {
        let target_v4: SocketAddr = "192.168.1.50:5004".parse().unwrap();
        assert!(participant_matches_target("192.168.1.50:5004", &target_v4));
        assert!(participant_matches_target("192.168.1.50:5005", &target_v4));
        assert!(participant_matches_target("192.168.1.50:5003", &target_v4));
        assert!(!participant_matches_target("192.168.1.50:5006", &target_v4));
        assert!(!participant_matches_target("192.168.1.51:5004", &target_v4));

        // Dual-stack IPv4-mapped IPv6 participant matching IPv4 target
        assert!(participant_matches_target(
            "[::ffff:192.168.1.50]:5004",
            &target_v4
        ));
        assert!(participant_matches_target(
            "[::ffff:192.168.1.50]:5005",
            &target_v4
        ));
        assert!(!participant_matches_target(
            "[::ffff:192.168.1.51]:5004",
            &target_v4
        ));

        // Invalid address parsing returns false
        assert!(!participant_matches_target("not-an-ip", &target_v4));
    }
}
