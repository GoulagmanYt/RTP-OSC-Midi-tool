use super::*;

fn channel(chapter: u8, data: &[u8], enhanced: bool) -> Vec<u8> {
    let size = data.len() + 3;
    let mut bytes = vec![
        if enhanced { 0x30 } else { 0x20 },
        0,
        0,
        (size >> 8) as u8 | if enhanced { 4 } else { 0 },
        size as u8,
        chapter,
    ];
    bytes.extend_from_slice(data);
    bytes
}

fn system(flags: u8, data: &[u8]) -> Vec<u8> {
    let size = data.len() + 2;
    let mut bytes = vec![0x40, 0, 0, flags | (size >> 8) as u8, size as u8];
    bytes.extend_from_slice(data);
    bytes
}

fn cc(number: u8, value: u8) -> MidiMessage {
    MidiMessage::ControlChange(Channel::C1, number.into(), value.into())
}

fn repair(bytes: &[u8], history: &mut ChannelHistory) -> Option<ChannelRepairs> {
    ChannelRepairs::build_with_state(Journal::parse(bytes).unwrap(), 1, &mut [0; 16], history)
}

#[test]
fn parameter_absolute_relative_and_transaction_counts_are_not_replayed() {
    let mut history = ChannelHistory::default();
    let full = channel(32, &[0x20, 8, 0, 0, 0xCE, 2, 50, 127], false);
    assert_eq!(
        repair(&full, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(101, 0), cc(100, 0), cc(6, 2), cc(38, 50)]
    );
    assert_eq!(repair(&full, &mut history).unwrap().messages().count(), 0);
    for message in [cc(101, 0), cc(100, 0), cc(96, 0), cc(96, 0)] {
        history.observe(message);
    }
    assert_eq!(history.0[0].parameters.get(0).count, Some(0));
    let relative = channel(32, &[0x20, 8, 0, 0, 0x2E, 0, 5, 0], false);
    assert_eq!(
        repair(&relative, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(101, 0), cc(100, 0), cc(96, 0), cc(96, 0), cc(96, 0)]
    );
    assert_eq!(history.0[0].parameters.get(0).buttons, Some(5));
    assert_eq!(
        repair(&relative, &mut history).unwrap().messages().count(),
        0
    );
    let absolute = channel(32, &[0x20, 8, 0, 0, 0xCE, 12, 99, 1], false);
    assert_eq!(
        repair(&absolute, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(101, 0), cc(100, 0), cc(6, 12), cc(38, 99)]
    );
    assert_eq!(history.0[0].parameters.get(0).buttons, Some(0));
}

#[test]
fn parameter_selection_variants_pending_null_and_reset_are_preserved() {
    let mut history = ChannelHistory::default();
    let nrpn = channel(32, &[0x2C, 6, 1, 0xC2, 3, 4], false);
    assert_eq!(
        repair(&nrpn, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(99, 0), cc(98, 1), cc(6, 3), cc(38, 4)]
    );
    let pending = channel(32, &[0x40, 3, 0x82], false);
    assert_eq!(
        repair(&pending, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(99, 2)]
    );
    history.observe(cc(6, 7)); // omitted LSB means parameter LSB zero
    assert_eq!(history.0[0].parameters.selected, Some(16384 + 256));
    assert_eq!(history.0[0].parameters.get(16384 + 256).msb, Some(7));
    history.observe(cc(98, 3)); // omitted MSB uses prior C-active MSB
    assert_eq!(history.0[0].parameters.selected, Some(16384 + 259));
    let closed = channel(32, &[0, 2], false);
    assert_eq!(
        repair(&closed, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(101, 127), cc(100, 127)]
    );
    assert_eq!(history.0[0].parameters.selected, None);
    history.observe(cc(121, 0));
    assert_eq!(
        history.0[0].parameters.get(16385).msb,
        Some(3),
        "RP015 retains parameter values"
    );
    history.observe(MidiMessage::Reset);
    assert_eq!(history.0[0].parameters.get(16385).msb, None);
}

#[test]
fn ambiguous_or_over_budget_parameter_recovery_rolls_back() {
    let mut history = ChannelHistory::default();
    let count_only = channel(32, &[0x20, 6, 0, 0, 0x0C, 1], false);
    assert!(repair(&count_only, &mut history).is_none());
    let initial = channel(32, &[0x20, 7, 0, 0, 0xC2, 2, 0], false);
    repair(&initial, &mut history).unwrap();
    let overflow = channel(32, &[0x20, 7, 0, 0, 0x22, 63, 255], false);
    assert!(repair(&overflow, &mut history).is_none());
    assert_eq!(history.0[0].parameters.get(0).buttons, Some(0));
    let allocations = crate::test_alloc::count_allocations(|| {
        for number in 0..1000u16 {
            history.observe(cc(101, (number >> 7) as u8));
            history.observe(cc(100, (number & 127) as u8));
            history.observe(cc(6, 5));
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        history.0[0].parameters.get(0).msb,
        None,
        "evicted entries must become unknown"
    );
}

#[test]
fn enhanced_relative_controls_replay_only_missing_commands_across_count_wrap() {
    let mut history = ChannelHistory::default();
    history.0[0].controls[16].count = 63;
    history.0[0].controls[16].count_known = true;
    history.0[0].controls[16].value = Some(3);
    let bytes = channel(64, &[4, 16, 0xFF, 16, 3, 16, 0xC0, 16, 3, 16, 4], true);
    assert_eq!(
        repair(&bytes, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(16, 3), cc(16, 4)]
    );
    assert_eq!(repair(&bytes, &mut history).unwrap().messages().count(), 0);
    let broken = channel(64, &[3, 16, 0xC2, 16, 3, 16, 0xC7, 16, 4], true);
    assert!(repair(&broken, &mut history).is_none());
    assert_eq!(history.0[0].controls[16].count, 1);
}

#[test]
fn system_reset_song_sense_and_sequencer_restore_without_duplicate_triggers() {
    let mut history = ChannelHistory::default();
    history.observe(MidiMessage::NoteOn(Channel::C1, 60.into(), 100.into()));
    let bytes = system(0x70, &[0x70, 1, 2, 4, 3, 0x70, 0, 13]);
    assert_eq!(
        repair(&bytes, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [
            MidiMessage::Reset,
            MidiMessage::TuneRequest,
            MidiMessage::SongSelect(4.into()),
            MidiMessage::ActiveSensing,
            MidiMessage::Stop,
            MidiMessage::SongPositionPointer(2u16.into()),
            MidiMessage::Continue,
            MidiMessage::TimingClock,
            MidiMessage::TimingClock,
        ]
    );
    assert_eq!(history.0[0].notes[60], 0);
    assert_eq!(repair(&bytes, &mut history).unwrap().messages().count(), 0);
    history.observe(MidiMessage::TimingClock);
    assert_eq!(history.1.sequencer.unwrap().clock, 14);
    let stop = system(16, &[0x30, 0, 14]);
    repair(&stop, &mut history).unwrap();
    assert!(!history.1.sequencer.unwrap().running);
    assert_eq!(history.1.sequencer.unwrap().clock, 14);
}

#[test]
fn timecode_complete_partial_and_sysex_count_recovery_preserve_order() {
    let mut history = ChannelHistory::default();
    let full = system(8, &[0x47, 0x21, 2, 3, 4]);
    let plan = repair(&full, &mut history).unwrap();
    let Some(RepairEvent::SysEx(payload)) = plan.events().next() else {
        panic!("missing full-frame repair")
    };
    assert_eq!(payload, [127, 127, 1, 1, 0x21, 2, 3, 4]);
    let quarter = system(8, &[0x57, 0x40, 0x30, 0x20, 0x12]);
    assert_eq!(repair(&quarter, &mut history).unwrap().events().count(), 0);
    let partial = system(8, &[0x22, 0x40, 0x30, 0, 0]);
    assert_eq!(
        repair(&partial, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [
            MidiMessage::QuarterFrame(4.into()),
            MidiMessage::QuarterFrame(0x10.into()),
            MidiMessage::QuarterFrame(0x23.into()),
        ]
    );
    history.1.sysex_count = Some(7);
    let reset = system(4, &[0x2B, 8, 0x7E, 0x7F, 9, 0x81]);
    assert_eq!(repair(&reset, &mut history).unwrap().events().count(), 1);
    assert_eq!(history.1.sysex_count, Some(8));
    assert_eq!(repair(&reset, &mut history).unwrap().events().count(), 0);
}

#[test]
fn journal_repairs_missing_sysex_suffix_and_retains_unfinished_assembly() {
    let mut history = ChannelHistory::default();
    history.1.sysex_count = Some(7);
    let bytes = system(4, &[0x38, 7, 2, 0x83]);
    let plan = ChannelRepairs::build_with_prefix(
        Journal::parse(&bytes).unwrap(),
        1,
        &mut [0; 16],
        &mut history,
        Some(&[1, 2]),
    )
    .unwrap();
    assert_eq!(plan.pending_sysex(), Some(&[1, 2, 3][..]));
    assert_eq!(plan.events().count(), 0);
    let mut assembly = crate::sessions::sysex::SysExAssembly::new();
    let now = std::time::Instant::now();
    assembly
        .segment(0xF0, plan.pending_sysex().unwrap(), 0xF0, now)
        .unwrap();
    assert_eq!(
        assembly.segment(0xF7, &[4], 0xF7, now),
        Ok(Some(&[1, 2, 3, 4][..]))
    );
    let complete = system(4, &[0x3B, 7, 2, 0x83]);
    let plan = ChannelRepairs::build_with_prefix(
        Journal::parse(&complete).unwrap(),
        1,
        &mut [0; 16],
        &mut history,
        Some(&[1, 2]),
    )
    .unwrap();
    let Some(RepairEvent::SysEx(payload)) = plan.events().next() else {
        panic!("missing completed SysEx")
    };
    assert_eq!(payload, [1, 2, 3]);
    assert!(
        repair(&bytes, &mut history).is_none(),
        "a missing prefix cannot be guessed"
    );
}

#[test]
fn advanced_recovery_reuses_workspace_without_packet_allocations() {
    let mut workspace = ChannelRepairs::new();
    let parameter = channel(32, &[0x20, 7, 0, 0, 0xC2, 2, 50], false);
    let sysex = system(4, &[0x2B, 8, 0x7E, 0x7F, 9, 0x81]);
    let parameter = Journal::parse(&parameter).unwrap();
    let sysex = Journal::parse(&sysex).unwrap();
    let allocations = crate::test_alloc::count_allocations(|| {
        for _ in 0..10_000 {
            let mut history = ChannelHistory::default();
            workspace
                .rebuild(parameter, 1, &mut [0; 16], &mut history, None)
                .unwrap();
            assert_eq!(workspace.messages().count(), 4);
            workspace
                .rebuild(sysex, 1, &mut [0; 16], &mut history, None)
                .unwrap();
            assert_eq!(workspace.events().count(), 1);
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn general_purpose_data_entry_cannot_modify_an_open_parameter() {
    let mut history = ChannelHistory::default();
    repair(
        &channel(32, &[0x20, 7, 0, 0, 0xC2, 2, 50], false),
        &mut history,
    )
    .unwrap();
    let general = channel(64, &[0, 6, 12], false);
    assert_eq!(
        repair(&general, &mut history)
            .unwrap()
            .messages()
            .collect::<Vec<_>>(),
        [cc(101, 127), cc(100, 127), cc(6, 12)]
    );
    assert_eq!(history.0[0].parameters.get(0).msb, Some(2));
}

#[test]
fn unrelated_packet_loss_does_not_restart_a_known_running_sequencer() {
    let mut history = ChannelHistory::default();
    history.observe(MidiMessage::Start);
    for _ in 0..14 {
        history.observe(MidiMessage::TimingClock);
    }
    assert_eq!(history.1.sequencer.unwrap().clock, 13);
    let bytes = system(16, &[0x70, 0, 13]);
    assert_eq!(repair(&bytes, &mut history).unwrap().messages().count(), 0);
}
