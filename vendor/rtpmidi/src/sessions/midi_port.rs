use super::rtp_midi_session::{RtpMidiSession, current_timestamp};
use super::rtp_port::RtpPort;
use crate::packets::control_packets::clock_sync_packet::ClockSyncPacket;
use crate::packets::control_packets::control_packet::ControlPacket;
use crate::packets::control_packets::session_initiation_packet::SessionInitiationPacketBody;
use crate::packets::midi_packets::midi_event::MidiEvent;
use crate::packets::midi_packets::midi_packet::MidiPacket;
use crate::packets::midi_packets::rtp_midi_message::RtpMidiMessage;
use crate::packets::packet::RtpMidiPacket;
use crate::participant::Participant;
use crate::sessions::events::event_handling::EventListeners;
use crate::sessions::rtp_midi_session::current_timestamp_u32;
use arc_swap::ArcSwap;
use bytes::BytesMut;
use std::ffi::{CStr, CString};
use std::iter;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{Level, event, instrument};
use zerocopy::network_endian::{U16, U32, U64};

pub const MAX_MIDI_PACKET_SIZE: usize = 32768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceDisposition {
    InOrder,
    Gap(u16),
    DuplicateOrOutOfOrder,
}

fn classify_sequence(expected: u16, received: u16) -> SequenceDisposition {
    let distance = received.wrapping_sub(expected);
    match distance {
        0 => SequenceDisposition::InOrder,
        1..=0x7fff => SequenceDisposition::Gap(distance),
        _ => SequenceDisposition::DuplicateOrOutOfOrder,
    }
}

impl RtpPort for MidiPort {
    fn session_name(&self) -> &CStr {
        &self.name
    }

    fn ssrc(&self) -> U32 {
        self.ssrc
    }

    fn socket(&self) -> &Arc<UdpSocket> {
        &self.socket
    }

    fn participant_addr(participant: &Participant) -> SocketAddr {
        participant.midi_port_addr()
    }
}

pub(super) struct MidiPort {
    name: CString,
    ssrc: U32,
    start_time: Instant,
    send_state: Mutex<SendState>,
    socket: Arc<UdpSocket>,
}

struct SendState {
    sequence: u16,
    packet: BytesMut,
}

impl MidiPort {
    pub async fn bind(port: u16, name: CString, ssrc: U32) -> std::io::Result<Self> {
        let socket = Arc::new(UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?);

        Ok(MidiPort {
            ssrc,
            start_time: Instant::now(),
            name,
            send_state: Mutex::new(SendState {
                sequence: 0,
                packet: BytesMut::with_capacity(MidiPacket::MAX_ENCODED_SIZE),
            }),
            socket,
        })
    }

    #[instrument(name = "MIDI", skip_all, fields(name = %ctx.name(), src, src_name))]
    pub async fn start(
        &self,
        ctx: &RtpMidiSession,
        listeners: Arc<ArcSwap<EventListeners>>,
        buf: &mut [u8; MAX_MIDI_PACKET_SIZE],
    ) {
        let recv = self.socket.recv_from(buf).await;
        if recv.is_err() {
            event!(
                Level::ERROR,
                "Failed to receive data on MIDI port: {recv:?}"
            );
            return;
        }

        let (amt, src) = recv.unwrap();
        tracing::Span::current().record("src", tracing::field::display(src));
        event!(Level::TRACE, "Received {amt} bytes");

        let packet = RtpMidiPacket::parse(&buf[..amt]);
        if packet.is_err() {
            event!(Level::ERROR, "Failed to parse RTP MIDI packet: {packet:?}");
            let ssrc = ctx
                .participants
                .lock()
                .await
                .values_mut()
                .find(|peer| peer.midi_port_addr() == src)
                .map(|peer| {
                    if let Some(sysex) = peer.sysex.as_mut() {
                        sysex.clear();
                    }
                    peer.ssrc().get()
                });
            if let Some(ssrc) = ssrc {
                listeners.load().notify_stream_fault(ssrc);
            }
            return;
        }

        let packet = packet.unwrap();
        match packet {
            RtpMidiPacket::Control(control_packet) => match control_packet {
                ControlPacket::Invitation { body, name } => {
                    event!(
                        Level::INFO,
                        name = name.to_str().unwrap_or("Unknown"),
                        "Received session invitation"
                    );
                    self.handle_invitation(body, name, src, ctx).await;
                }
                ControlPacket::Acceptance { body, name } => {
                    event!(
                        Level::INFO,
                        name = name.to_str().unwrap_or("Unknown"),
                        "Received session acceptance"
                    );
                    if let Ok(participant) = self.handle_acceptance(body, src, ctx).await {
                        event!(
                            Level::INFO,
                            "Accepted MIDI port invitation from {participant}"
                        );
                        listeners.load().notify_participant_joined(&participant);
                    }
                }
                ControlPacket::ClockSync(clock_sync_packet) => {
                    event!(Level::DEBUG, "Received clock sync from {}", src);
                    self.handle_clock_sync(clock_sync_packet, src, ctx).await;
                }
                ControlPacket::Termination(body) => {
                    event!(Level::INFO, "Received session termination from {}", src);
                    ctx.handle_termination(body.sender_ssrc, body.initiator_token, src, true)
                        .await;
                }
                _ => {
                    event!(Level::WARN, "Unhandled control packet {:?}", control_packet);
                }
            },
            RtpMidiPacket::Midi(midi_packet) => {
                event!(Level::DEBUG, "Parsed MIDI packet: {:#?}", midi_packet);
                let ssrc = midi_packet.ssrc().get();
                let arrived = Instant::now();
                let mut peers = ctx.participants.lock().await;
                let Some(peer) = peers
                    .get_mut(&midi_packet.ssrc())
                    .filter(|p| p.midi_port_addr() == src)
                else {
                    return;
                };
                let packet_deadline = peer
                    .deadline(u32::from(midi_packet.timestamp()))
                    .unwrap_or(arrived);
                let received = midi_packet.sequence_number().get();
                let mut loss = if let Some(expected) = peer.expected_sequence {
                    match classify_sequence(expected, received) {
                        SequenceDisposition::InOrder => None,
                        SequenceDisposition::Gap(distance) => {
                            Some(crate::sessions::events::event_handling::PacketLoss {
                                ssrc,
                                expected_sequence: expected,
                                received_sequence: received,
                                lost_packets: distance,
                                recovered: false,
                            })
                        }
                        SequenceDisposition::DuplicateOrOutOfOrder => {
                            event!(
                                Level::WARN,
                                ssrc,
                                expected,
                                received,
                                "Discarded duplicate or out-of-order MIDI packet"
                            );
                            return;
                        }
                    }
                } else {
                    None
                };
                let repairs = loss.as_mut().and_then(|loss| {
                    if peer.sysex.as_ref().is_some_and(|state| state.is_active()) {
                        return None;
                    }
                    let journal = crate::packets::midi_packets::recovery_journal::Journal::parse(
                        midi_packet.journal_bytes()?,
                    )
                    .ok()?;
                    let repairs = super::note_recovery::ChannelRepairs::build(
                        journal,
                        loss.expected_sequence,
                        &mut peer.active_notes,
                    )?;
                    loss.recovered = true;
                    Some(repairs)
                });
                for event in midi_packet.commands() {
                    if let RtpMidiMessage::MidiMessage(message) = event.command() {
                        super::note_recovery::observe(&mut peer.active_notes, *message);
                    }
                }
                let needs_assembly = peer.sysex.as_ref().is_some_and(|state| state.is_active())
                    || midi_packet.commands().any(|event| {
                        matches!(event.command(), RtpMidiMessage::SysExSegment { .. })
                    });
                let mut sysex_state = if needs_assembly {
                    peer.sysex.take()
                } else {
                    None
                };
                if loss.is_some()
                    && let Some(state) = sysex_state.as_mut()
                {
                    state.clear();
                }
                peer.expected_sequence = Some(received.wrapping_add(1));
                drop(peers);

                // Snapshot sequence state before invoking user callbacks. One
                // listener guard covers the whole packet, not each MIDI command.
                let listeners = listeners.load();
                if let Some(loss) = loss {
                    listeners.notify_packet_loss(loss);
                }
                if let Some(repairs) = repairs {
                    for message in repairs.messages() {
                        listeners.notify_midi_message(
                            message,
                            midi_packet.timestamp().get(),
                            ssrc,
                            packet_deadline,
                        );
                    }
                }
                let mut timestamp = u32::from(midi_packet.timestamp());
                for command in midi_packet.commands() {
                    timestamp = timestamp.wrapping_add(command.delta_time());
                    let deadline = packet_deadline
                        + std::time::Duration::from_micros(
                            u64::from(timestamp.wrapping_sub(midi_packet.timestamp().get())) * 100,
                        );
                    match command.command() {
                        RtpMidiMessage::MidiMessage(message) => {
                            if (command.command().status() < 0xF8
                                || matches!(message, midi_types::MidiMessage::Reset))
                                && let Some(state) = sysex_state.as_mut()
                            {
                                state.clear();
                            }
                            listeners.notify_midi_message(
                                *message,
                                timestamp,
                                ssrc,
                                packet_deadline
                                    + std::time::Duration::from_micros(
                                        u64::from(
                                            timestamp
                                                .wrapping_sub(u32::from(midi_packet.timestamp())),
                                        ) * 100,
                                    ),
                            );
                        }
                        RtpMidiMessage::SysExSegment { head, data, tail } => {
                            match sysex_state
                                .as_mut()
                                .expect("fragment storage reserved")
                                .segment(*head, data, *tail, arrived)
                            {
                                Ok(Some(payload)) => listeners
                                    .notify_sysex_packet(payload, timestamp, ssrc, deadline),
                                Ok(None) => {}
                                Err(()) => listeners.notify_stream_fault(ssrc),
                            }
                        }
                        RtpMidiMessage::SysEx(sysex) => {
                            if let Some(state) = sysex_state.as_mut() {
                                state.clear();
                            }
                            listeners.notify_sysex_packet(sysex, timestamp, ssrc, deadline);
                        }
                    }
                }
                drop(listeners);
                // A concurrently removed/replaced participant owns a fresh buffer.
                // Never restore old fragments into a reconnected session.
                if let Some(state) = sysex_state
                    && let Some(peer) = ctx.participants.lock().await.get_mut(&midi_packet.ssrc())
                    && peer.sysex.is_none()
                {
                    peer.sysex = Some(state);
                }
            }
        }
    }

    #[instrument(skip_all, fields(sender = %sender_name.to_str().unwrap_or("Unknown"), token = %body.initiator_token, src = %src))]
    async fn handle_invitation(
        &self,
        body: &SessionInitiationPacketBody,
        sender_name: &CStr,
        src: SocketAddr,
        ctx: &RtpMidiSession,
    ) {
        let Some(ctrl_port) = src.port().checked_sub(1) else {
            return;
        };
        let ctrl_addr = SocketAddr::new(src.ip(), ctrl_port);
        let existing = ctx
            .participants
            .lock()
            .await
            .get(&body.sender_ssrc)
            .cloned();
        if let Some(peer) = existing {
            if peer.addr() == ctrl_addr && peer.initiator_token() == Some(body.initiator_token) {
                self.send_invitation_acceptance(body.initiator_token, src)
                    .await;
            }
            return;
        }
        let mut pending = ctx.pending_invitations.lock().await;
        let valid = pending.get(&body.initiator_token).is_some_and(|inv| {
            inv.addr == ctrl_addr
                && inv.ssrc == body.sender_ssrc
                && inv.created.elapsed() < std::time::Duration::from_secs(30)
        });
        if !valid {
            return;
        }
        pending.remove(&body.initiator_token);
        drop(pending);
        let participant = Participant::new(
            ctrl_addr,
            false,
            Some(body.initiator_token),
            sender_name,
            body.sender_ssrc,
        );
        let mut peers = ctx.participants.lock().await;
        if peers.len() >= 128 {
            return;
        }
        peers.insert(body.sender_ssrc, participant.clone());
        drop(peers);
        self.send_invitation_acceptance(body.initiator_token, src)
            .await;
        ctx.notify_joined(&participant).await;
    }

    #[instrument(skip_all, fields(token = %ack_body.initiator_token))]
    async fn handle_acceptance(
        &self,
        ack_body: &SessionInitiationPacketBody,
        src: SocketAddr,
        ctx: &RtpMidiSession,
    ) -> Result<Participant, &str> {
        let mut locked_pending_invitations = ctx.pending_invitations.lock().await;

        let inv = locked_pending_invitations
            .get(&ack_body.initiator_token)
            .cloned();
        if inv.is_none() {
            event!(
                Level::WARN,
                ssrc = ack_body.sender_ssrc.get(),
                "Received Acceptance but no pending invitation found for this SSRC."
            );
            return Err("No pending invitation found");
        }

        let inv = inv.unwrap();
        if inv.token != ack_body.initiator_token
            || inv.addr != src
            || inv.ssrc != ack_body.sender_ssrc
            || inv.created.elapsed() >= std::time::Duration::from_secs(30)
        {
            event!(
                Level::WARN,
                expected = inv.token.get(),
                "Received Acceptance with mismatched token",
            );
            return Err("Token mismatch in acceptance");
        }

        locked_pending_invitations.remove(&ack_body.initiator_token);
        drop(locked_pending_invitations);
        event!(
            Level::DEBUG,
            "Matched Acceptance for MIDI port invitation. Sending Clock Sync."
        );
        let ctrl_addr = SocketAddr::new(
            inv.addr.ip(),
            inv.addr.port().checked_sub(1).ok_or("Invalid MIDI port")?,
        );
        let participant = Participant::new(
            ctrl_addr,
            true,
            Some(inv.token),
            &inv.name,
            ack_body.sender_ssrc,
        );
        let mut peers = ctx.participants.lock().await;
        if peers.len() >= 128 {
            return Err("Participant capacity reached");
        }
        peers.insert(ack_body.sender_ssrc, participant.clone());
        drop(peers);
        let timestamps = [U64::new(0); 3];
        self.send_clock_sync(std::iter::once(&participant), timestamps, 0)
            .await;
        Ok(participant)
    }

    #[instrument(skip_all, fields(count = count))]
    pub(super) async fn send_clock_sync<'a, I>(
        &self,
        participants: I,
        mut timestamps: [U64; 3],
        count: u8,
    ) where
        I: IntoIterator<Item = &'a Participant>,
    {
        if count > 2 {
            event!(Level::ERROR, "Invalid count for clock sync");
            return;
        }
        timestamps[count as usize] = current_timestamp(self.start_time);

        let packet = ControlPacket::new_clock_sync_as_bytes(count, timestamps, self.ssrc);
        for participant in participants {
            if let Err(e) = self
                .socket
                .send_to(&packet, participant.midi_port_addr())
                .await
            {
                event!(
                    Level::WARN,
                    name = participant.name().to_str().unwrap_or("Unknown"),
                    addr = %participant.midi_port_addr(),
                    "Failed to send clock sync: {e}"
                );
            } else {
                event!(
                    Level::DEBUG,
                    name = participant.name().to_str().unwrap_or("Unknown"),
                    "Sent clock sync"
                );
            }
        }
    }

    #[instrument(skip_all, fields(count = packet.count, ssrc = packet.sender_ssrc.get(), src_name))]
    async fn handle_clock_sync(
        &self,
        packet: &ClockSyncPacket,
        src: SocketAddr,
        ctx: &RtpMidiSession,
    ) {
        let mut part_lock = ctx.participants.lock().await;
        let maybe_participant = part_lock.get_mut(&packet.sender_ssrc);

        if maybe_participant.is_none() {
            event!(
                Level::WARN,
                "Received clock sync but no matching participant found"
            );
            return;
        }
        let participant = maybe_participant.unwrap();
        tracing::Span::current()
            .record("src_name", participant.name().to_str().unwrap_or("Unknown"));
        if participant.midi_port_addr() != src || packet.count > 2 {
            return;
        }
        let now = Instant::now();
        let local_ticks = current_timestamp(self.start_time).get();
        let [t0, t1, t2] = packet.timestamps.map(|value| value.get());
        match packet.count {
            1 if local_ticks >= t0 && local_ticks - t0 <= 100_000 => {
                let midpoint = now - std::time::Duration::from_micros((local_ticks - t0) * 50);
                participant.synchronize(t1, midpoint);
            }
            2 if t2 >= t0
                && t2 - t0 <= 100_000
                && local_ticks >= t1
                && local_ticks - t1 <= 100_000 =>
            {
                let local = now - std::time::Duration::from_micros((local_ticks - t1) * 100);
                participant.synchronize(t0 + (t2 - t0) / 2, local);
            }
            0 => {}
            _ => return,
        }
        participant.received_clock_sync();
        event!(Level::DEBUG, "Updated clock sync for existing participant");
        let participant = participant.clone();
        drop(part_lock);

        match packet.count {
            0 | 1 => {
                self.send_clock_sync(
                    iter::once(&participant),
                    packet.timestamps,
                    packet.count + 1,
                )
                .await;
            }
            2 => {
                let latency_estimate = packet.timestamps[2]
                    .get()
                    .saturating_sub(packet.timestamps[0].get())
                    as f64
                    / 10.0;
                event!(
                    Level::INFO,
                    latency_estimate = std::format!("{latency_estimate}ms"),
                    "Clock sync finalized"
                );
            }
            _ => {
                event!(Level::ERROR, "Unexpected clock sync count");
            }
        }
    }

    #[instrument(skip_all, fields(name = %ctx.name(), participants))]
    pub async fn send_midi_batch<'a>(
        &self,
        ctx: &RtpMidiSession,
        commands: &'a [MidiEvent<'a>],
    ) -> std::io::Result<()> {
        let lock = ctx.participants.lock().await;
        let mut destinations = [None; 128];
        for (slot, participant) in destinations.iter_mut().zip(lock.values()) {
            *slot = Some(participant.midi_port_addr());
        }
        drop(lock);
        let mut state = self.send_state.lock().await;
        let sequence = state.sequence;
        MidiPacket::write_into(
            &mut state.packet,
            U16::new(sequence),
            current_timestamp_u32(self.start_time),
            self.ssrc,
            commands,
            false,
        )?;
        state.sequence = sequence.wrapping_add(1);
        for destination in destinations.into_iter().flatten() {
            self.socket.send_to(&state.packet, destination).await?;
        }
        Ok(())
    }

    #[instrument(skip_all, fields(name = %ctx.name()))]
    pub async fn send_midi<'a>(
        &self,
        ctx: &RtpMidiSession,
        command: &'a RtpMidiMessage<'a>,
    ) -> std::io::Result<()> {
        let batch: [MidiEvent; 1] = [MidiEvent::new(None, command.to_owned())];
        self.send_midi_batch(ctx, &batch).await
    }

    #[instrument(skip_all, fields(addr = %addr))]
    pub(super) async fn send_invitation(&self, invitation: &[u8], addr: SocketAddr) {
        event!(Level::DEBUG, "Sending session invitation");
        let result = self.socket.send_to(invitation, addr).await;
        if let Err(e) = result {
            event!(Level::WARN, "Failed to send session invitation: {e}");
        } else {
            event!(Level::INFO, "Sent session invitation");
        }
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;

    #[test]
    fn sequence_classification_handles_gap_duplicate_and_wrap() {
        assert_eq!(classify_sequence(10, 10), SequenceDisposition::InOrder);
        assert_eq!(classify_sequence(10, 13), SequenceDisposition::Gap(3));
        assert_eq!(
            classify_sequence(10, 9),
            SequenceDisposition::DuplicateOrOutOfOrder
        );
        assert_eq!(classify_sequence(u16::MAX, 0), SequenceDisposition::Gap(1));
        assert_eq!(
            classify_sequence(0, u16::MAX),
            SequenceDisposition::DuplicateOrOutOfOrder
        );
    }
}
