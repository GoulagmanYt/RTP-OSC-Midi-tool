use super::sysex::SysExAssembly;
use crate::participant::Participant;

pub(super) struct ReceiveState {
    identity: Option<u64>,
    pub expected_sequence: Option<u16>,
    pub active_notes: [u128; 16],
    pub channels: super::channel_state::ChannelHistory,
    generation: u64,
    pub sysex: SysExAssembly,
}

impl ReceiveState {
    fn new() -> Self {
        Self {
            identity: None,
            expected_sequence: None,
            active_notes: [0; 16],
            channels: super::channel_state::ChannelHistory::default(),
            generation: 0,
            sysex: SysExAssembly::new(),
        }
    }

    fn reset(&mut self, identity: u64, generation: u64) {
        self.identity = Some(identity);
        self.expected_sequence = None;
        self.active_notes.fill(0);
        self.channels = super::channel_state::ChannelHistory::default();
        self.generation = generation;
        self.sysex.clear();
    }
}

/// Owned exclusively by the MIDI receive task. Reserve all 128 fragment buffers
/// before receiving, so reconnecting peers never allocate on the packet path.
pub(super) struct ReceiveStates {
    slots: Vec<ReceiveState>,
}

impl ReceiveStates {
    pub fn new() -> Self {
        Self {
            slots: (0..128).map(|_| ReceiveState::new()).collect(),
        }
    }

    pub fn get(
        &mut self,
        peer: &Participant,
        peers: &[Participant],
        generation: u64,
    ) -> &mut ReceiveState {
        let identity = peer.identity();
        let index = self
            .slots
            .iter()
            .position(|s| s.identity == Some(identity))
            .unwrap_or_else(|| {
                self.slots
                    .iter()
                    .position(|s| !peers.iter().any(|p| Some(p.identity()) == s.identity))
                    .expect("registered peers are capped at 128")
            });
        let state = &mut self.slots[index];
        if state.identity != Some(identity) {
            state.reset(identity, generation);
        } else if state.generation != generation {
            state.active_notes.fill(0);
            state.channels = super::channel_state::ChannelHistory::default();
            state.sysex.clear();
            state.generation = generation;
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::network_endian::U32;

    #[test]
    fn reconnects_reuse_slots_without_inheriting_notes_sequences_or_fragments() {
        let mut states = ReceiveStates::new();
        let storage = states.slots.as_ptr();
        for _ in 0..10_000 {
            let peers = [Participant::new(
                "127.0.0.1:5004".parse().unwrap(),
                true,
                None,
                c"peer",
                U32::new(7),
            )];
            let state = states.get(&peers[0], &peers, 0);
            assert_eq!(state.expected_sequence, None);
            assert_eq!(state.active_notes, [0; 16]);
            assert!(!state.sysex.is_active());
            state.expected_sequence = Some(123);
            state.active_notes[0] = 1 << 60;
            state
                .sysex
                .segment(0xF0, &[1, 2], 0xF0, std::time::Instant::now())
                .unwrap();
        }
        assert_eq!(states.slots.as_ptr(), storage);
        assert_eq!(states.slots.len(), 128);
    }
}
