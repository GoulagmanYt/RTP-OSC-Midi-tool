use midi_types::{status, MidiMessage as RtMidiMessage};
use smallvec::SmallVec;
use std::net::SocketAddr;

pub fn midi_to_bytes(message: RtMidiMessage) -> SmallVec<[u8; 32]> {
    use RtMidiMessage::*;
    match message {
        NoteOn(channel, note, velocity) => SmallVec::from_slice(&[
            status::NOTE_ON | u8::from(channel),
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
            SmallVec::from_slice(&[status::PITCH_BEND_CHANGE | u8::from(channel), b1, b2])
        }
        QuarterFrame(frame) => SmallVec::from_slice(&[status::QUARTER_FRAME, frame.into()]),
        SongPositionPointer(pos) => {
            let (b1, b2): (u8, u8) = pos.into();
            SmallVec::from_slice(&[status::SONG_POSITION_POINTER, b1, b2])
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

pub fn participant_matches_target(participant_addr: &str, target: &SocketAddr) -> bool {
    if participant_addr == target.to_string() {
        return true;
    }
    let Ok(participant) = participant_addr.parse::<SocketAddr>() else {
        return false;
    };
    if participant.ip() != target.ip() {
        return false;
    }
    let tp = target.port();
    let pp = participant.port();
    pp == tp || (tp.checked_add(1) == Some(pp)) || (tp.checked_sub(1) == Some(pp))
}
