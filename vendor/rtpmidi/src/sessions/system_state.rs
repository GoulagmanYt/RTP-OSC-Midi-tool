use midi_types::MidiMessage;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SequencerState {
    pub running: bool,
    pub clock: u32,
    pub downbeat: bool,
    pub continued: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct SystemState {
    pub resets: Option<u8>,
    pub tunes: Option<u8>,
    pub senses: Option<u8>,
    pub song: Option<u8>,
    pub sequencer: Option<SequencerState>,
    pub sysex_count: Option<u8>,
    pub full_frame: Option<[u8; 4]>,
}

impl SystemState {
    pub fn reset_render_state(&mut self) {
        self.song = None;
        self.sequencer = None;
        self.full_frame = None;
    }

    pub fn observe(&mut self, message: MidiMessage) {
        match message {
            MidiMessage::Reset => {
                self.resets = self.resets.map(|c| c.wrapping_add(1) & 127);
                self.reset_render_state();
            }
            MidiMessage::TuneRequest => self.tunes = self.tunes.map(|c| c.wrapping_add(1) & 127),
            MidiMessage::ActiveSensing => {
                self.senses = self.senses.map(|c| c.wrapping_add(1) & 127)
            }
            MidiMessage::SongSelect(song) => self.song = Some(song.into()),
            MidiMessage::Start => {
                self.sequencer = Some(SequencerState {
                    running: true,
                    ..SequencerState::default()
                })
            }
            MidiMessage::Continue => {
                if let Some(state) = self.sequencer.as_mut() {
                    state.running = true;
                    state.continued = true;
                }
            }
            MidiMessage::Stop => {
                if let Some(state) = self.sequencer.as_mut() {
                    state.running = false;
                }
            }
            MidiMessage::SongPositionPointer(position) => {
                let state = self.sequencer.get_or_insert_with(SequencerState::default);
                state.clock = u32::from(u16::from(position)) * 6;
                state.downbeat = false;
                state.continued = true;
            }
            MidiMessage::TimingClock => {
                if let Some(state) = self.sequencer.as_mut()
                    && state.running
                {
                    if state.downbeat {
                        state.clock = (state.clock + 1) & 0x7FFFF;
                    }
                    state.downbeat = true;
                }
            }
            MidiMessage::QuarterFrame(_) => self.full_frame = None,
            _ => {}
        }
    }

    pub fn observe_sysex(&mut self, payload: &[u8]) {
        if let [0x7F, _, 1, 1, hour, minute, second, frame] = payload {
            self.full_frame = Some([*hour, *minute, *second, *frame]);
        }
    }
}
