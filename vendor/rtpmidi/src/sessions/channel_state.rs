use midi_types::MidiMessage;

#[derive(Clone, Copy, Default)]
pub(super) struct ControllerState {
    pub value: Option<u8>,
    pub count: u8,
    pub count_known: bool,
    pub toggles: u8,
    pub toggles_known: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ProgramState {
    pub program: u8,
    pub bank: Option<(u8, u8)>,
}

#[derive(Clone, Copy)]
pub(super) struct ChannelState {
    pub controls: [ControllerState; 128],
    pub parameters: super::parameter_state::ParameterState,
    pub program: Option<ProgramState>,
    pub notes: [u16; 128],
    pub velocities: [u8; 128],
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            controls: [ControllerState::default(); 128],
            parameters: super::parameter_state::ParameterState::default(),
            program: None,
            notes: [0; 128],
            velocities: [64; 128],
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct ChannelHistory(pub [ChannelState; 16], pub super::system_state::SystemState);

impl ChannelHistory {
    pub fn observe(&mut self, message: MidiMessage) {
        self.1.observe(message);
        match message {
            MidiMessage::ControlChange(channel, number, value) => {
                let state = &mut self.0[usize::from(u8::from(channel))];
                let number = usize::from(u8::from(number));
                let value = u8::from(value);
                state.parameters.observe(number as u8, value);
                let control = &mut state.controls[number];
                control.count = control.count.wrapping_add(1) & 63;
                if let Some(previous) = control.value {
                    if (previous >= 64) != (value >= 64) {
                        control.toggles = control.toggles.wrapping_add(1) & 63;
                    }
                } else {
                    control.toggles_known = false;
                }
                control.value = Some(value);
                if number < 32 {
                    // A new MSB resets the paired LSB unless a later LSB updates it.
                    state.controls[number + 32].value = Some(0);
                    state.controls[number + 32].toggles_known = false;
                }
                if number == 121 {
                    // Do not guess instrument-specific Reset All Controllers defaults.
                    // Bank select is retained by the standard reset semantics.
                    for (index, control) in state.controls.iter_mut().enumerate() {
                        if !matches!(index, 0 | 32 | 121) {
                            control.value = None;
                            control.toggles_known = false;
                        }
                    }
                }
                if matches!(number, 120 | 123..=127) {
                    state.notes.fill(0);
                }
            }
            MidiMessage::ProgramChange(channel, program) => {
                let state = &mut self.0[usize::from(u8::from(channel))];
                state.program = Some(ProgramState {
                    program: u8::from(program),
                    bank: state.controls[0]
                        .value
                        .map(|msb| (msb, state.controls[32].value.unwrap_or(0))),
                });
            }
            MidiMessage::Reset => self.0 = Self::default().0,
            MidiMessage::NoteOn(channel, note, velocity) if u8::from(velocity) != 0 => {
                let state = &mut self.0[usize::from(u8::from(channel))];
                let index = usize::from(u8::from(note));
                state.notes[index] = state.notes[index].saturating_add(1);
                state.velocities[index] = u8::from(velocity);
            }
            MidiMessage::NoteOff(channel, note, _) | MidiMessage::NoteOn(channel, note, _) => {
                let count =
                    &mut self.0[usize::from(u8::from(channel))].notes[usize::from(u8::from(note))];
                *count = count.saturating_sub(1);
            }
            _ => {}
        }
    }

    pub fn observe_sysex(&mut self, payload: &[u8]) -> bool {
        self.1.observe_sysex(payload);
        let reset = matches!(payload, [0x7E, _, 9, 0 | 1 | 3] | [0x7E, _, 10, 1 | 2]);
        if reset {
            self.0 = Self::default().0;
            self.1.reset_render_state();
        }
        reset
    }
}
