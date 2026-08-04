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

use super::runtime_state::{AudioCallbackState, PluginBackend, RESET_CONTROLLERS};

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
    state.vst2_midi_events.clear();
    while state.vst2_midi_events.len() < state.vst2_midi_events.capacity() {
        let Some(msg) = state.pending_midi.pop_front() else {
            break;
        };
        let _ = msg.timestamp_us;
        state.vst2_midi_events.push(api::MidiEvent {
            event_type: api::EventType::Midi,
            byte_size: std::mem::size_of::<api::MidiEvent>() as i32,
            delta_frames: 0,
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
    for (index, event) in state.vst2_midi_events.iter_mut().enumerate() {
        state.vst2_event_batch.events[index] = event as *mut api::MidiEvent as *mut api::Event;
    }
    state.vst2_event_batch.num_events = state.vst2_midi_events.len() as i32;
    if state.vst2_event_batch.num_events > 0 {
        // VST2 models Events as a C flexible array whose Rust declaration has
        // only two pointer slots. Vst2EventBatch provides the full preallocated
        // 512-pointer layout expected by the ABI.
        let events = unsafe {
            &*(&state.vst2_event_batch as *const super::runtime_state::Vst2EventBatch
                as *const api::Events)
        };
        instance.process_events(events);
    }
}

pub(super) fn process_pending_vst3_midi(
    state: &mut AudioCallbackState,
    instance: &mut rack::vst3::Vst3Plugin,
) -> Result<(), rack::Error> {
    if state.pending_midi.is_empty() {
        return Ok(());
    }

    state.midi_events.clear();
    let mut consumed = 0usize;
    while consumed < state.midi_events.capacity() {
        let Some(msg) = state.pending_midi.pop_front() else {
            break;
        };
        consumed += 1;
        let _ = msg.timestamp_us;
        if let Some(event) = midi_to_rack_event(msg.data) {
            state.midi_events.push(event);
        }
    }
    if !state.midi_events.is_empty() {
        instance.send_midi(&state.midi_events)?;
        state.midi_events.clear();
    }
    Ok(())
}

pub(super) fn reset_all_notes(plugin: Arc<Mutex<PluginBackend>>) {
    if let Some(mut plugin) = plugin.try_lock() {
        send_reset_messages(&mut plugin);
    }
}

pub(super) fn send_reset_messages(plugin: &mut PluginBackend) {
    match plugin {
        PluginBackend::Vst2 { instance } => {
            for ch in 0..16u8 {
                for message in reset_messages_for_channel(ch) {
                    let mut midi_event = api::MidiEvent {
                        event_type: api::EventType::Midi,
                        byte_size: std::mem::size_of::<api::MidiEvent>() as i32,
                        delta_frames: 0,
                        flags: MidiEventFlags::REALTIME_EVENT.bits(),
                        note_length: 0,
                        note_offset: 0,
                        midi_data: message,
                        _midi_reserved: 0,
                        detune: 0,
                        note_off_velocity: 0,
                        _reserved1: 0,
                        _reserved2: 0,
                    };
                    let evt = api::Events {
                        num_events: 1,
                        _reserved: 0,
                        events: [
                            &mut midi_event as *mut api::MidiEvent as *mut api::Event,
                            std::ptr::null_mut(),
                        ],
                    };
                    instance.process_events(&evt);
                }
            }
        }
        PluginBackend::Vst3 { instance, .. } => {
            for ch in 0..16u8 {
                for controller in RESET_CONTROLLERS {
                    let event = RackMidiEvent {
                        sample_offset: 0,
                        kind: RackMidiEventKind::ControlChange {
                            controller,
                            value: 0,
                            channel: ch,
                        },
                    };
                    let _ = instance.send_midi(std::slice::from_ref(&event));
                }
            }
        }
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
