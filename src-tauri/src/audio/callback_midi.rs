#![allow(deprecated)]

use std::sync::Arc;

use parking_lot::Mutex;
use rack::prelude::{MidiEvent as RackMidiEvent, MidiEventKind as RackMidiEventKind};
use rack::PluginInstance as _;
use vst::{
    api::{self, MidiEventFlags},
    host::PluginInstance,
    plugin::Plugin,
};

use super::runtime_state::{monotonic_us, AudioCallbackState, PluginBackend};

#[cfg(test)]
use super::midi_playback::MidiBlockTiming;

pub(super) fn midi_to_rack_event(data: [u8; 3]) -> Option<RackMidiEvent> {
    let status = data[0];
    let channel = status & 0x0F;
    let kind = match status & 0xF0 {
        0x80 => RackMidiEventKind::NoteOff {
            note: data[1],
            velocity: data[2],
            channel,
        },
        0x90 => {
            if data[2] == 0 {
                RackMidiEventKind::NoteOff {
                    note: data[1],
                    velocity: 0,
                    channel,
                }
            } else {
                RackMidiEventKind::NoteOn {
                    note: data[1],
                    velocity: data[2],
                    channel,
                }
            }
        }
        0xA0 => RackMidiEventKind::PolyphonicAftertouch {
            note: data[1],
            pressure: data[2],
            channel,
        },
        0xB0 => RackMidiEventKind::ControlChange {
            controller: data[1],
            value: data[2],
            channel,
        },
        0xC0 => RackMidiEventKind::ProgramChange {
            program: data[1],
            channel,
        },
        0xD0 => RackMidiEventKind::ChannelAftertouch {
            pressure: data[1],
            channel,
        },
        0xE0 => {
            let value = ((data[2] as u16) << 7) | data[1] as u16;
            RackMidiEventKind::PitchBend { value, channel }
        }
        _ => return None,
    };

    Some(RackMidiEvent {
        sample_offset: 0,
        kind,
    })
}

pub(super) fn process_pending_vst2_midi(
    state: &mut AudioCallbackState,
    instance: &mut PluginInstance,
) {
    prepare_pending_vst2_midi(
        state,
        state.render_source_end_us.unwrap_or_else(monotonic_us),
    );
    if state.vst2_event_batch.num_events > 0 {
        // VST2 Events declares two pointers; the preallocated batch has room
        // for every event, as required by the C flexible-array ABI.
        let events = unsafe {
            &*(&state.vst2_event_batch as *const super::runtime_state::Vst2EventBatch
                as *const api::Events)
        };
        instance.process_events(events);
    }
}

pub(super) fn prepare_pending_vst2_midi(state: &mut AudioCallbackState, now_us: u64) {
    state.vst2_midi_events.clear();
    let mut timing = state.midi_playback.begin_block(
        state.last_frames,
        state.sample_rate,
        now_us,
        state.pending_midi.front().map(|packet| packet.timestamp_us),
    );
    while state.vst2_midi_events.len() < state.vst2_midi_events.capacity() {
        let Some(msg) = state.pending_midi.front().copied() else {
            break;
        };
        if !timing.contains(msg.timestamp_us) {
            break;
        }
        state.pending_midi.pop_front();
        state.telemetry.record_midi_delivery(msg);
        let delta_frames = timing.offset(msg.timestamp_us) as i32;
        state.vst2_midi_events.push(api::MidiEvent {
            event_type: api::EventType::Midi,
            byte_size: std::mem::size_of::<api::MidiEvent>() as i32,
            delta_frames,
            flags: MidiEventFlags::REALTIME_EVENT.bits(),
            note_length: 0,
            note_offset: 0,
            midi_data: msg.data,
            _midi_reserved: 0,
            detune: 0,
            note_off_velocity: 0,
            _reserved1: 0,
            _reserved2: 0,
        });
    }
    state
        .midi_playback
        .commit(&timing, state.next_midi_timestamp_us());
    for (index, event) in state.vst2_midi_events.iter_mut().enumerate() {
        state.vst2_event_batch.events[index] = event as *mut api::MidiEvent as *mut api::Event;
    }
    state.vst2_event_batch.num_events = state.vst2_midi_events.len() as i32;
}

pub(super) fn process_pending_vst3_midi(
    state: &mut AudioCallbackState,
    instance: &mut rack::vst3::Vst3Plugin,
) -> Result<(), rack::Error> {
    let available = instance.remaining_midi_capacity();
    submit_pending_vst3_midi(state, available, |events| instance.send_midi(events))
}

pub(super) fn submit_pending_vst3_midi(
    state: &mut AudioCallbackState,
    available: usize,
    send: impl FnOnce(&[RackMidiEvent]) -> Result<(), rack::Error>,
) -> Result<(), rack::Error> {
    submit_pending_vst3_midi_at(
        state,
        available,
        state.render_source_end_us.unwrap_or_else(monotonic_us),
        send,
    )
}

pub(super) fn submit_pending_vst3_midi_at(
    state: &mut AudioCallbackState,
    available: usize,
    now_us: u64,
    send: impl FnOnce(&[RackMidiEvent]) -> Result<(), rack::Error>,
) -> Result<(), rack::Error> {
    state.midi_events.clear();
    let mut timing = state.midi_playback.begin_block(
        state.last_frames,
        state.sample_rate,
        now_us,
        state.pending_midi.front().map(|packet| packet.timestamp_us),
    );
    let mut consumed = 0usize;
    let event_limit = available.min(state.midi_events.capacity());
    // A reset may already occupy part of the backend batch. Keep unadmitted
    // messages in FIFO order, including when the backend rejects submission.
    for msg in state.pending_midi.iter().take(state.midi_events.capacity()) {
        if !timing.contains(msg.timestamp_us) {
            break;
        }
        if let Some(mut event) = midi_to_rack_event(msg.data) {
            if state.midi_events.len() == event_limit {
                break;
            }
            event.sample_offset = timing.offset(msg.timestamp_us);
            state.midi_events.push(event);
        }
        consumed += 1;
    }
    if !state.midi_events.is_empty() {
        let result = send(&state.midi_events);
        state.midi_events.clear();
        result?;
    }
    for msg in state.pending_midi.drain(..consumed) {
        state.telemetry.record_midi_delivery(msg);
    }
    state
        .midi_playback
        .commit(&timing, state.next_midi_timestamp_us());
    Ok(())
}

pub(super) fn reset_all_notes(plugin: Arc<Mutex<PluginBackend>>) {
    if let Some(mut plugin) = plugin.try_lock() {
        send_reset_messages(&mut plugin);
    }
}

pub(super) fn send_reset_messages(plugin: &mut PluginBackend) {
    match plugin {
        PluginBackend::Vst2 { instance, .. } => {
            let mut midi_events: [api::MidiEvent; 64] = core::array::from_fn(|_| api::MidiEvent {
                event_type: api::EventType::Midi,
                byte_size: std::mem::size_of::<api::MidiEvent>() as i32,
                delta_frames: 0,
                flags: MidiEventFlags::REALTIME_EVENT.bits(),
                note_length: 0,
                note_offset: 0,
                midi_data: [0; 3],
                _midi_reserved: 0,
                detune: 0,
                note_off_velocity: 0,
                _reserved1: 0,
                _reserved2: 0,
            });
            let mut event_ptrs: [*mut api::Event; 64] = [std::ptr::null_mut(); 64];
            let mut idx = 0;
            for ch in 0..16u8 {
                for message in reset_messages_for_channel(ch) {
                    midi_events[idx].midi_data = message;
                    event_ptrs[idx] =
                        &mut midi_events[idx] as *mut api::MidiEvent as *mut api::Event;
                    idx += 1;
                }
            }
            #[repr(C)]
            struct ResetBatch {
                num_events: i32,
                _reserved: isize,
                events: [*mut api::Event; 64],
            }
            let batch = ResetBatch {
                num_events: 64,
                _reserved: 0,
                events: event_ptrs,
            };
            let evt = unsafe { &*(&batch as *const ResetBatch as *const api::Events) };
            instance.process_events(evt);
        }
        PluginBackend::Vst3 { instance, .. } => {
            // Dedicated admission cannot lose the panic to a full MIDI batch.
            // Completion is checked only after a successful process() call.
            let _ = instance.request_midi_reset();
        }
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        PluginBackend::Remote { instance } => instance.panic_all_notes(),
    }
}

pub(super) fn reset_messages_for_channel(ch: u8) -> [[u8; 3]; 4] {
    [
        [0xB0 | ch, 64, 0],
        [0xB0 | ch, 120, 0],
        [0xB0 | ch, 121, 0],
        [0xB0 | ch, 123, 0],
    ]
}

#[cfg(test)]
mod timing_tests {
    use super::MidiBlockTiming;

    #[test]
    fn block_offsets_preserve_fifo_order_for_regressing_ingress_timestamps() {
        let mut timing = MidiBlockTiming::new(1000, 100_000, 1_010_000);
        // A release followed by a retrigger must retain that order even when
        // independently measured ingress ages place the retrigger slightly earlier.
        let timestamps = [1_001_000, 1_001_000, 1_000_990, 1_002_000];
        assert_eq!(
            timestamps.map(|stamp| timing.offset(stamp)),
            [100, 100, 100, 200]
        );
    }

    #[test]
    fn block_offsets_bound_late_and_future_events_without_resampling_the_clock() {
        let mut timing = MidiBlockTiming::new(480, 48_000, 1_010_000);
        assert_eq!(timing.offset(0), 0);
        assert_eq!(timing.offset(1_005_000), 240);
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_eq!(timing.offset(1_005_000), 240);
        assert_eq!(timing.offset(u64::MAX), 479);
        assert_eq!(MidiBlockTiming::new(0, 0, 0).offset(u64::MAX), 0);
    }
}
