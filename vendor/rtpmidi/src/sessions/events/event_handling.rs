use midi_types::MidiMessage;

use crate::participant::Participant;

pub(super) type MidiMessageListener = dyn Fn((MidiMessage, u32)) + Send + 'static;
pub(super) type TimestampedMidiMessageListener = dyn Fn(TimestampedMidiMessage) + Send + 'static;
pub(super) type SysExPacketListener = dyn for<'a> Fn(&'a [u8]) + Send + 'static;
pub(super) type ParticipantListener = dyn for<'a> Fn(&'a Participant) + Send + 'static;
pub(super) type PacketLossListener = dyn Fn(PacketLoss) + Send + 'static;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketLoss {
    pub ssrc: u32,
    pub expected_sequence: u16,
    pub received_sequence: u16,
    pub lost_packets: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampedMidiMessage {
    pub message: MidiMessage,
    /// RTP timestamp in 100-microsecond units, including command delta time.
    pub timestamp: u32,
    pub ssrc: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RtpMidiEventType {
    MidiMessage,
    SysExPacket,
    ParticipantJoined,
    ParticipantLeft,
    PacketLoss,
    TimestampedMidiMessage,
}

pub struct EventListeners {
    midi_message: Vec<Box<MidiMessageListener>>,
    timestamped_midi_message: Vec<Box<TimestampedMidiMessageListener>>,
    sysex_packet: Vec<Box<SysExPacketListener>>,
    participant_joined: Vec<Box<ParticipantListener>>,
    participant_left: Vec<Box<ParticipantListener>>,
    packet_loss: Vec<Box<PacketLossListener>>,
}

pub struct MidiMessageEvent;
pub struct TimestampedMidiMessageEvent;
pub struct SysExPacketEvent;
pub struct ParticipantJoinedEvent;
pub struct ParticipantLeftEvent;
pub struct PacketLossEvent;

pub trait EventType {
    type Data<'a>;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static;
}

impl EventType for MidiMessageEvent {
    type Data<'a> = (MidiMessage, u32);

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static,
    {
        listeners.midi_message.push(Box::new(callback));
    }
}

impl EventType for TimestampedMidiMessageEvent {
    type Data<'a> = TimestampedMidiMessage;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static,
    {
        listeners.timestamped_midi_message.push(Box::new(callback));
    }
}

impl EventType for SysExPacketEvent {
    type Data<'a> = &'a [u8];

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static,
    {
        listeners.sysex_packet.push(Box::new(callback));
    }
}

impl EventType for ParticipantJoinedEvent {
    type Data<'a> = &'a Participant;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static,
    {
        listeners.participant_joined.push(Box::new(callback));
    }
}

impl EventType for ParticipantLeftEvent {
    type Data<'a> = &'a Participant;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static,
    {
        listeners.participant_left.push(Box::new(callback));
    }
}

impl EventType for PacketLossEvent {
    type Data<'a> = PacketLoss;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + 'static,
    {
        listeners.packet_loss.push(Box::new(callback));
    }
}

impl Default for EventListeners {
    fn default() -> Self {
        Self::new()
    }
}

impl EventListeners {
    pub fn new() -> Self {
        Self {
            midi_message: Vec::new(),
            timestamped_midi_message: Vec::new(),
            sysex_packet: Vec::new(),
            participant_joined: Vec::new(),
            participant_left: Vec::new(),
            packet_loss: Vec::new(),
        }
    }

    pub fn notify_midi_message(&self, message: MidiMessage, timestamp: u32, ssrc: u32) {
        for listener in &self.midi_message {
            listener((message, timestamp));
        }
        let event = TimestampedMidiMessage {
            message,
            timestamp,
            ssrc,
        };
        for listener in &self.timestamped_midi_message {
            listener(event);
        }
    }

    pub fn notify_sysex_packet(&self, bytes: &[u8]) {
        for listener in &self.sysex_packet {
            listener(bytes);
        }
    }

    pub fn notify_participant_joined(&self, participant: &Participant) {
        for listener in &self.participant_joined {
            listener(participant);
        }
    }

    pub fn notify_participant_left(&self, participant: &Participant) {
        for listener in &self.participant_left {
            listener(participant);
        }
    }

    pub fn notify_packet_loss(&self, loss: PacketLoss) {
        for listener in &self.packet_loss {
            listener(loss);
        }
    }
}
