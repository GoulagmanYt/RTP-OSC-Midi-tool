use midi_types::MidiMessage;
use std::sync::Arc;

use crate::participant::Participant;

pub(super) type MidiMessageListener = dyn Fn((MidiMessage, u32)) + Send + Sync + 'static;
pub(super) type TimestampedMidiMessageListener =
    dyn Fn(TimestampedMidiMessage) + Send + Sync + 'static;
pub(super) type TimestampedSysExListener =
    dyn for<'a> Fn(TimestampedSysEx<'a>) + Send + Sync + 'static;
pub(super) type SysExPacketListener = dyn for<'a> Fn(&'a [u8]) + Send + Sync + 'static;
pub(super) type ParticipantListener = dyn for<'a> Fn(&'a Participant) + Send + Sync + 'static;
pub(super) type PacketLossListener = dyn Fn(PacketLoss) + Send + Sync + 'static;
pub(super) type StreamFaultListener = dyn Fn(u32) + Send + Sync + 'static;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketLoss {
    pub ssrc: u32,
    pub expected_sequence: u16,
    pub received_sequence: u16,
    pub lost_packets: u16,
    /// True only when the entire covering journal uses supported repair forms.
    pub recovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampedMidiMessage {
    pub message: MidiMessage,
    /// RTP timestamp in 100-microsecond units, including command delta time.
    pub timestamp: u32,
    pub ssrc: u32,
    /// Monotonic local rendering deadline derived from the latest CK exchange.
    pub deadline: std::time::Instant,
}

/// Completed SysEx payload. For segmented transfers the deadline is that of
/// the final segment; this API does not reproduce inter-byte transmission timing.
#[derive(Debug, Clone, Copy)]
pub struct TimestampedSysEx<'a> {
    pub payload: &'a [u8],
    pub timestamp: u32,
    pub ssrc: u32,
    pub deadline: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RtpMidiEventType {
    MidiMessage,
    SysExPacket,
    TimestampedSysEx,
    ParticipantJoined,
    ParticipantLeft,
    PacketLoss,
    TimestampedMidiMessage,
    StreamFault,
}

#[derive(Clone)]
pub struct EventListeners {
    midi_message: Vec<Arc<MidiMessageListener>>,
    timestamped_midi_message: Vec<Arc<TimestampedMidiMessageListener>>,
    sysex_packet: Vec<Arc<SysExPacketListener>>,
    timestamped_sysex: Vec<Arc<TimestampedSysExListener>>,
    participant_joined: Vec<Arc<ParticipantListener>>,
    participant_left: Vec<Arc<ParticipantListener>>,
    packet_loss: Vec<Arc<PacketLossListener>>,
    stream_fault: Vec<Arc<StreamFaultListener>>,
}

pub struct MidiMessageEvent;
pub struct TimestampedMidiMessageEvent;
pub struct SysExPacketEvent;
pub struct TimestampedSysExEvent;
pub struct ParticipantJoinedEvent;
pub struct ParticipantLeftEvent;
pub struct PacketLossEvent;
/// An established endpoint sent an unusable packet. Its last release may be lost.
pub struct StreamFaultEvent;

impl EventType for StreamFaultEvent {
    type Data<'a> = u32;
    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.stream_fault.push(Arc::new(callback));
    }
}

pub trait EventType {
    type Data<'a>;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static;
}

impl EventType for MidiMessageEvent {
    type Data<'a> = (MidiMessage, u32);

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.midi_message.push(Arc::new(callback));
    }
}

impl EventType for TimestampedMidiMessageEvent {
    type Data<'a> = TimestampedMidiMessage;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.timestamped_midi_message.push(Arc::new(callback));
    }
}

impl EventType for TimestampedSysExEvent {
    type Data<'a> = TimestampedSysEx<'a>;
    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.timestamped_sysex.push(Arc::new(callback));
    }
}

impl EventType for SysExPacketEvent {
    type Data<'a> = &'a [u8];

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.sysex_packet.push(Arc::new(callback));
    }
}

impl EventType for ParticipantJoinedEvent {
    type Data<'a> = &'a Participant;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.participant_joined.push(Arc::new(callback));
    }
}

impl EventType for ParticipantLeftEvent {
    type Data<'a> = &'a Participant;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.participant_left.push(Arc::new(callback));
    }
}

impl EventType for PacketLossEvent {
    type Data<'a> = PacketLoss;

    fn add_listener_to_storage<F>(listeners: &mut EventListeners, callback: F)
    where
        F: for<'a> Fn(Self::Data<'a>) + Send + Sync + 'static,
    {
        listeners.packet_loss.push(Arc::new(callback));
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
            timestamped_sysex: Vec::new(),
            participant_joined: Vec::new(),
            participant_left: Vec::new(),
            packet_loss: Vec::new(),
            stream_fault: Vec::new(),
        }
    }

    pub fn notify_midi_message(
        &self,
        message: MidiMessage,
        timestamp: u32,
        ssrc: u32,
        deadline: std::time::Instant,
    ) {
        for listener in &self.midi_message {
            listener((message, timestamp));
        }
        let event = TimestampedMidiMessage {
            message,
            timestamp,
            ssrc,
            deadline,
        };
        for listener in &self.timestamped_midi_message {
            listener(event);
        }
    }

    pub fn notify_sysex_packet(
        &self,
        bytes: &[u8],
        timestamp: u32,
        ssrc: u32,
        deadline: std::time::Instant,
    ) {
        let event = TimestampedSysEx {
            payload: bytes,
            timestamp,
            ssrc,
            deadline,
        };
        for listener in &self.timestamped_sysex {
            listener(event);
        }
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

    pub fn notify_stream_fault(&self, ssrc: u32) {
        for listener in &self.stream_fault {
            listener(ssrc);
        }
    }
}
