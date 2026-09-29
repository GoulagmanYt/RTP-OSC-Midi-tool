use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::ffi::CString;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{Level, event, instrument};
use zerocopy::network_endian::{U32, U64};

use super::host_syncer::HostSyncer;
use super::invite_responder::InviteResponder;
#[cfg(feature = "mdns")]
use super::mdns::advertise_mdns;
use super::rtp_port::RtpPort;
use crate::packets::midi_packets::midi_event::MidiEvent;
use crate::packets::midi_packets::rtp_midi_message::RtpMidiMessage;
use crate::participant::Participant;
use crate::sessions::control_port::{ControlPort, MAX_CONTROL_PACKET_SIZE};
use crate::sessions::events::event_handling::{EventListeners, EventType};
use crate::sessions::midi_port::{MAX_MIDI_PACKET_SIZE, MidiPort};

#[derive(Clone)]
pub struct RtpMidiSession {
    pub(super) participants: Arc<super::peer_registry::PeerRegistry>, // key by ssrc
    pub(super) pending_invitations: Arc<Mutex<HashMap<U32, PendingInvitation>>>, // key by token
    pub(super) midi_port: Arc<MidiPort>,
    pub(super) recovery_generation: Arc<AtomicU64>,

    listeners: Arc<ArcSwap<EventListeners>>,
    control_port: Arc<ControlPort>,
    host_syncer: Arc<HostSyncer>,
    cancel_token: Arc<CancellationToken>,
    task_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
    name: CString,
    lifetime: Option<Arc<SessionLifetime>>,
    stop_lock: Arc<Mutex<()>>,
    #[cfg(feature = "mdns")]
    mdns: mdns_sd::ServiceDaemon,
}

// Only public session handles keep this guard alive. Background task contexts
// must not own it, otherwise they keep their own cancellation alive forever.
struct SessionLifetime {
    cancel_token: Arc<CancellationToken>,
    #[cfg(feature = "mdns")]
    mdns: mdns_sd::ServiceDaemon,
}

impl Drop for SessionLifetime {
    fn drop(&mut self) {
        self.cancel_token.cancel();
        #[cfg(feature = "mdns")]
        let _ = self.mdns.shutdown();
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingInvitation {
    pub addr: SocketAddr,
    pub token: U32,
    pub name: CString,
    pub created: Instant,
    pub ssrc: U32,
}

impl RtpMidiSession {
    async fn bind(port: u16, name: &str, ssrc: u32) -> std::io::Result<Self> {
        if port == u16::MAX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid RTP port pair",
            ));
        }
        let cstr_name = CString::new(name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // Keep the control socket bound while reserving its adjacent MIDI port.
        // Port zero requests an ephemeral pair, not an unrelated MIDI port 1.
        let mut attempts = 0;
        let (control_port, midi_port, bound_port) = loop {
            attempts += 1;
            let control = ControlPort::bind(port, cstr_name.to_owned(), U32::new(ssrc)).await?;
            let bound = control.socket().local_addr()?.port();
            let midi = if let Some(midi_port) = bound.checked_add(1) {
                MidiPort::bind(midi_port, cstr_name.to_owned(), U32::new(ssrc)).await
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "No adjacent MIDI port",
                ))
            };
            match midi {
                Ok(midi) => break (Arc::new(control), Arc::new(midi), bound),
                Err(error)
                    if port == 0
                        && attempts < 32
                        && error.kind() == std::io::ErrorKind::AddrInUse =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        };
        #[cfg(not(feature = "mdns"))]
        let _ = bound_port;

        let mut context = RtpMidiSession {
            participants: Arc::new(super::peer_registry::PeerRegistry::new()),
            pending_invitations: Arc::new(Mutex::new(HashMap::new())),
            control_port,
            midi_port,
            recovery_generation: Arc::new(AtomicU64::new(0)),
            host_syncer: Arc::new(HostSyncer::new()),
            listeners: Arc::new(ArcSwap::from_pointee(EventListeners::new())),
            cancel_token: Arc::new(CancellationToken::new()),
            task_handles: Arc::new(std::sync::Mutex::new(Vec::new())),
            name: cstr_name,
            lifetime: None,
            stop_lock: Arc::new(Mutex::new(())),
            #[cfg(feature = "mdns")]
            mdns: advertise_mdns(name, bound_port)
                .map_err(|e| std::io::Error::other(e.to_string()))?,
        };
        context.lifetime = Some(Arc::new(SessionLifetime {
            cancel_token: context.cancel_token.clone(),
            #[cfg(feature = "mdns")]
            mdns: context.mdns.clone(),
        }));
        Ok(context)
    }

    #[instrument(skip(port),fields(requested_control_port = %port))]
    pub async fn start(
        port: u16,
        name: &str,
        ssrc: u32,
        invite_handler: InviteResponder,
    ) -> std::io::Result<Arc<Self>> {
        Self::start_with_recovery_generation(
            port,
            name,
            ssrc,
            invite_handler,
            Arc::new(AtomicU64::new(0)),
        )
        .await
    }

    /// Share the destination's reset epoch so journal recovery does not assume
    /// that notes received before a destination panic are still sounding.
    pub async fn start_with_recovery_generation(
        port: u16,
        name: &str,
        ssrc: u32,
        invite_handler: InviteResponder,
        recovery_generation: Arc<AtomicU64>,
    ) -> std::io::Result<Arc<Self>> {
        event!(tracing::Level::INFO, "Starting RTP-MIDI session");
        let mut session = Self::bind(port, name, ssrc).await?;
        session.recovery_generation = recovery_generation;
        let ctx = Arc::new(session);
        ctx.start_threads(invite_handler);
        Ok(ctx)
    }

    fn task_context(&self) -> Self {
        let mut context = self.clone();
        context.lifetime = None;
        context
    }

    fn start_threads(&self, invite_handler: InviteResponder) {
        let mut handles = Vec::new();

        // Control port listener
        let control_port = Arc::clone(&self.control_port);
        let ctx_control = self.task_context();
        let control_cancel_token = Arc::clone(&self.cancel_token);

        let handle = tokio::spawn(async move {
            let mut buf = [0u8; MAX_CONTROL_PACKET_SIZE];
            loop {
                tokio::select! {
                    biased;
                    _ = control_cancel_token.cancelled() => {
                        event!(Level::DEBUG, "listen_for_control: cancellation requested");
                        break;
                    },
                    _ = control_port.start(&ctx_control, &invite_handler, &mut buf) => {}
                }
            }
        });
        handles.push(handle);

        // MIDI port listener
        let ctx_midi = self.task_context();
        let midi_port_listener = Arc::clone(&self.midi_port);
        let listeners_midi = Arc::clone(&self.listeners);
        let midi_cancel_token = Arc::clone(&self.cancel_token);

        let handle = tokio::spawn(async move {
            let mut buf = [0u8; MAX_MIDI_PACKET_SIZE];
            let mut receive_states = crate::sessions::receive_state::ReceiveStates::new();
            loop {
                tokio::select! {
                    biased;
                    _ = midi_cancel_token.cancelled() => {
                        event!(Level::DEBUG, "listen_for_midi: cancellation requested");
                        break;
                    },
                    _ = midi_port_listener.start(&ctx_midi, listeners_midi.clone(), &mut buf, &mut receive_states) => {}
                }
            }
        });
        handles.push(handle);

        // Control traffic arriving on the MIDI socket cannot suspend data dispatch.
        let control_receiver = self.midi_port.take_control_receiver();
        let ctx = self.task_context();
        let port = self.midi_port.clone();
        let listeners = self.listeners.clone();
        let cancel = self.cancel_token.clone();
        handles.push(tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {}
                _ = port.run_control(&ctx, control_receiver, listeners) => {}
            }
        }));

        // Host clock sync
        let ctx_clock = self.task_context();
        let syncer_clock = Arc::clone(&self.host_syncer);
        let syncer_cancel_token = Arc::clone(&self.cancel_token);
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = syncer_cancel_token.cancelled() => {
                        event!(Level::DEBUG, "listen_for_clock_sync: cancellation requested");
                        break;
                    },
                    _ = async {
                        sleep(Duration::from_secs(10)).await;
                        syncer_clock.cleanup(&ctx_clock).await;
                    } => {}
                }
            }
        });
        handles.push(handle);

        self.task_handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(handles);
    }

    #[instrument(skip_all, fields(name = %self.name()))]
    pub fn stop_immediately(&self) {
        event!(Level::INFO, name = self.name(), "Stopping RTP-MIDI session");
        self.cancel_token.cancel();
        #[cfg(feature = "mdns")]
        let _ = self.mdns.shutdown();
    }
    #[instrument(skip_all, fields(name = %self.name()))]
    pub async fn stop_gracefully(&self) {
        let _stop_guard = self.stop_lock.lock().await;
        self.stop_immediately();

        // Wait for all background tasks to complete
        let handles = {
            let mut task_handles = self.task_handles.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *task_handles)
        };

        event!(
            Level::DEBUG,
            "Waiting for {} background tasks to complete",
            handles.len()
        );
        for handle in handles {
            if let Err(e) = handle.await {
                event!(Level::WARN, "Task failed to complete cleanly: {}", e);
            }
        }
        // Reception is stopped before draining peers: an invitation cannot add
        // a participant after the shutdown snapshot has been taken.
        self.remove_all_participants().await;
        self.pending_invitations.lock().await.clear();
        event!(Level::INFO, "Graceful shutdown complete");
    }

    #[instrument(skip_all, fields(name = %self.name()))]
    pub async fn remove_all_participants(&self) {
        let participants = self.participants().await;
        for participant in participants {
            self.remove_participant(&participant).await;
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    pub async fn invite_participant(&self, addr: SocketAddr) {
        self.control_port.invite_participant(self, addr).await;
    }

    pub async fn participants(&self) -> Vec<Participant> {
        let participants = self.participants.lock().await;
        participants.values().cloned().collect()
    }

    #[instrument(skip_all, fields(participant = %participant.name().to_str().unwrap_or("Unknown")))]
    pub async fn remove_participant(&self, participant: &Participant) {
        let removed = {
            let mut peers = self.participants.lock().await;
            if peers
                .get(&participant.ssrc())
                .is_some_and(|current| current.identity() == participant.identity())
            {
                peers.remove(&participant.ssrc())
            } else {
                None
            }
        };
        let Some(participant) = removed else {
            return;
        };
        event!(Level::INFO, "Removing participant");
        self.control_port
            .send_termination_packet(&participant)
            .await;
        self.midi_port.send_termination_packet(&participant).await;
        self.listeners.load().notify_participant_left(&participant);
    }

    pub(super) async fn notify_joined(&self, participant: &Participant) {
        self.listeners.load().notify_participant_joined(participant);
    }

    pub(super) async fn handle_termination(
        &self,
        ssrc: U32,
        token: U32,
        src: SocketAddr,
        midi: bool,
    ) {
        let participant = {
            let mut peers = self.participants.lock().await;
            let valid = peers.get(&ssrc).is_some_and(|p| {
                (if midi { p.midi_port_addr() } else { p.addr() }) == src
                    && p.initiator_token() == Some(token)
            });
            if valid { peers.remove(&ssrc) } else { None }
        };
        if let Some(participant) = participant {
            self.listeners.load().notify_participant_left(&participant);
        }
    }

    pub async fn add_listener<E, F>(&self, _event_type: E, callback: F)
    where
        E: EventType,
        F: for<'a> Fn(E::Data<'a>) + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);
        self.listeners.rcu(|current| {
            let mut updated = (**current).clone();
            let callback = callback.clone();
            E::add_listener_to_storage(&mut updated, move |data| callback(data));
            updated
        });
    }

    pub async fn send_midi_batch<'a>(&self, commands: &[MidiEvent<'a>]) -> std::io::Result<()> {
        self.midi_port.send_midi_batch(self, commands).await
    }

    pub async fn send_midi<'a>(&self, command: &RtpMidiMessage<'a>) -> std::io::Result<()> {
        self.midi_port.send_midi(self, command).await
    }

    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or("Unnamed Session")
    }

    /// Actual bound control address, including the selected port for port zero.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.control_port.socket().local_addr()
    }
}

pub fn current_timestamp(start_time: Instant) -> U64 {
    let time = (Instant::now() - start_time).as_micros() as u64 / 100;
    U64::new(time)
}

pub fn current_timestamp_u32(start_time: Instant) -> U32 {
    let time = (Instant::now() - start_time).as_micros() as u64 / 100;
    U32::new(time as u32)
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[tokio::test]
    async fn stale_cleanup_cannot_remove_a_reconnected_peer_with_the_same_ssrc() {
        let session = RtpMidiSession::bind(0, "Cleanup", 1).await.unwrap();
        let previous = Participant::new(
            "127.0.0.1:5004".parse().unwrap(),
            true,
            Some(U32::new(3)),
            c"peer",
            U32::new(7),
        );
        let current = Participant::new(
            previous.addr(),
            true,
            Some(U32::new(4)),
            c"peer",
            previous.ssrc(),
        );
        session
            .participants
            .lock()
            .await
            .insert(current.ssrc(), current.clone());
        session.remove_participant(&previous).await;
        assert_eq!(
            session.participants.snapshot()[0].identity(),
            current.identity()
        );
        session
            .handle_termination(previous.ssrc(), U32::new(3), previous.addr(), false)
            .await;
        assert_eq!(session.participants.snapshot().len(), 1);
        session.remove_participant(&current).await;
        assert!(session.participants.snapshot().is_empty());
    }

    #[tokio::test]
    async fn destination_reset_allows_journal_note_recovery_again() {
        use crate::packets::midi_packets::midi_packet::MidiPacket;
        use crate::sessions::events::event_handling::MidiMessageEvent;
        use midi_types::{Channel, MidiMessage, Note, Value7};
        use std::sync::atomic::Ordering;
        use zerocopy::network_endian::U16;

        tokio::time::timeout(Duration::from_secs(3), async {
            let receiver = RtpMidiSession::bind(0, "Recovery", 1).await.unwrap();
            let sender = RtpMidiSession::bind(0, "Source", 2).await.unwrap();
            let mut source_addr = sender.local_addr().unwrap();
            source_addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            receiver.participants.lock().await.insert(
                U32::new(2),
                Participant::new(source_addr, true, None, c"Source", U32::new(2)),
            );
            let mut destination = receiver.midi_port.socket().local_addr().unwrap();
            destination.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            receiver
                .add_listener(MidiMessageEvent, move |(message, _)| {
                    tx.send(message).unwrap();
                })
                .await;
            let note = MidiMessage::NoteOn(Channel::C1, Note::from(61), Value7::from(100));
            let commands = [MidiEvent::new(None, RtpMidiMessage::MidiMessage(note))];
            let first =
                MidiPacket::new_as_bytes(U16::new(0), U32::new(0), U32::new(2), &commands, false);
            let mut first = first.to_vec();
            first[12] |= 0x40;
            first.extend_from_slice(&[0x20, 0, 0, 0, 6, 128, 7, 0, 0]);
            let mut buffer = [0; MAX_MIDI_PACKET_SIZE];
            let mut receive_states = crate::sessions::receive_state::ReceiveStates::new();
            sender
                .midi_port
                .socket()
                .send_to(&first, destination)
                .await
                .unwrap();
            receiver
                .midi_port
                .start(
                    &receiver,
                    receiver.listeners.clone(),
                    &mut buffer,
                    &mut receive_states,
                )
                .await;
            assert_eq!(
                rx.recv().await.unwrap(),
                MidiMessage::ProgramChange(Channel::C1, 7.into())
            );
            assert_eq!(rx.recv().await.unwrap(), note);

            let _held_management_lock = receiver.participants.lock().await;
            for (sequence, reset) in [(2, false), (4, true)] {
                if reset {
                    receiver.recovery_generation.fetch_add(1, Ordering::AcqRel);
                }
                let mut packet = MidiPacket::new_as_bytes(
                    U16::new(sequence),
                    U32::new(0),
                    U32::new(2),
                    &[],
                    false,
                )
                .to_vec();
                packet[12] |= 0x40;
                // Covering Chapter N with a recommended Note-On and no Note-Off bitmap.
                packet.extend_from_slice(&[0x20, 0, 1, 0, 7, 8, 1, 0xF0, 61, 0xE4]);
                sender
                    .midi_port
                    .socket()
                    .send_to(&packet, destination)
                    .await
                    .unwrap();
                receiver
                    .midi_port
                    .start(
                        &receiver,
                        receiver.listeners.clone(),
                        &mut buffer,
                        &mut receive_states,
                    )
                    .await;
                if reset {
                    assert_eq!(rx.try_recv().unwrap(), note);
                } else {
                    assert!(
                        rx.try_recv().is_err(),
                        "known note was retriggered without a reset"
                    );
                }
            }
            // A Reset State SysEx followed by a note in the same command list
            // must clear the old history before observing that new note.
            let ordered = [
                MidiEvent::new(None, RtpMidiMessage::SysEx(&[0x7E, 0x7F, 9, 1])),
                MidiEvent::new(Some(0), RtpMidiMessage::MidiMessage(note)),
            ];
            let packet =
                MidiPacket::new_as_bytes(U16::new(5), U32::new(0), U32::new(2), &ordered, false);
            sender
                .midi_port
                .socket()
                .send_to(&packet, destination)
                .await
                .unwrap();
            receiver
                .midi_port
                .start(
                    &receiver,
                    receiver.listeners.clone(),
                    &mut buffer,
                    &mut receive_states,
                )
                .await;
            assert_eq!(rx.try_recv().unwrap(), note);
            let mut packet =
                MidiPacket::new_as_bytes(U16::new(7), U32::new(0), U32::new(2), &[], false)
                    .to_vec();
            packet[12] |= 0x40;
            packet.extend_from_slice(&[0x20, 0, 6, 0, 7, 8, 1, 0xF0, 61, 0xE4]);
            sender
                .midi_port
                .socket()
                .send_to(&packet, destination)
                .await
                .unwrap();
            receiver
                .midi_port
                .start(
                    &receiver,
                    receiver.listeners.clone(),
                    &mut buffer,
                    &mut receive_states,
                )
                .await;
            assert!(
                rx.try_recv().is_err(),
                "post-SysEx note was lost from receive history"
            );
            let (fault_tx, mut fault_rx) = tokio::sync::mpsc::unbounded_channel();
            receiver
                .add_listener(
                    crate::sessions::events::event_handling::StreamFaultEvent,
                    move |ssrc| {
                        fault_tx.send(ssrc).unwrap();
                    },
                )
                .await;
            let oversized = vec![0u8; MAX_MIDI_PACKET_SIZE + 1];
            sender
                .midi_port
                .socket()
                .send_to(&oversized, destination)
                .await
                .unwrap();
            receiver
                .midi_port
                .start(
                    &receiver,
                    receiver.listeners.clone(),
                    &mut buffer,
                    &mut receive_states,
                )
                .await;
            let source = fault_rx
                .try_recv()
                .expect("oversized final release did not request recovery");
            assert!(source == 0 || source == 2);
        })
        .await
        .expect("RTP recovery test timed out");
    }

    #[tokio::test]
    async fn ten_thousand_overlapping_notes_match_releases_across_udp_journal_loss() {
        use crate::packets::midi_packets::midi_packet::MidiPacket;
        use crate::sessions::events::event_handling::{MidiMessageEvent, PacketLossEvent};
        use midi_types::{Channel, MidiMessage};
        use zerocopy::network_endian::U16;

        tokio::time::timeout(Duration::from_secs(15), async {
            let receiver = RtpMidiSession::bind(0, "Polyphonic recovery", 1)
                .await
                .unwrap();
            let sender = RtpMidiSession::bind(0, "Source", 2).await.unwrap();
            let mut source = sender.local_addr().unwrap();
            source.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            receiver.participants.lock().await.insert(
                U32::new(2),
                Participant::new(source, true, None, c"Source", U32::new(2)),
            );
            let mut destination = receiver.midi_port.socket().local_addr().unwrap();
            destination.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            receiver
                .add_listener(MidiMessageEvent, move |(message, _)| {
                    tx.try_send(message).unwrap();
                })
                .await;
            let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel(1);
            receiver
                .add_listener(PacketLossEvent, move |loss| {
                    loss_tx.try_send(loss).unwrap();
                })
                .await;
            let mut buffer = [0; MAX_MIDI_PACKET_SIZE];
            let mut states = crate::sessions::receive_state::ReceiveStates::new();
            let mut sequence = 65_000u16;
            let mut notes = [0u16; 128];
            let (mut ons, mut offs) = (0, 0);
            for _ in 0..625 {
                let on: Vec<_> = (0..16)
                    .map(|index| {
                        MidiEvent::new(
                            Some(0),
                            RtpMidiMessage::MidiMessage(MidiMessage::NoteOn(
                                Channel::C1,
                                (48 + index % 8).into(),
                                100.into(),
                            )),
                        )
                    })
                    .collect();
                let off: Vec<_> = (48..56)
                    .map(|note| {
                        MidiEvent::new(
                            Some(0),
                            RtpMidiMessage::MidiMessage(MidiMessage::NoteOff(
                                Channel::C1,
                                note.into(),
                                64.into(),
                            )),
                        )
                    })
                    .collect();
                for commands in [&on[..], &off[..], &[][..]] {
                    let mut packet = MidiPacket::new_as_bytes(
                        U16::new(sequence),
                        U32::new(0),
                        U32::new(2),
                        commands,
                        false,
                    )
                    .to_vec();
                    if commands.is_empty() {
                        // The second set of eight releases was lost. The next
                        // packet's Chapter N must release all remaining voices.
                        packet[12] |= 0x40;
                        let checkpoint = sequence.wrapping_sub(1).to_be_bytes();
                        packet.extend_from_slice(&[
                            0x20,
                            checkpoint[0],
                            checkpoint[1],
                            0,
                            6,
                            8,
                            0,
                            0x66,
                            255,
                        ]);
                    }
                    sender
                        .midi_port
                        .socket()
                        .send_to(&packet, destination)
                        .await
                        .unwrap();
                    receiver
                        .midi_port
                        .start(
                            &receiver,
                            receiver.listeners.clone(),
                            &mut buffer,
                            &mut states,
                        )
                        .await;
                    while let Ok(message) = rx.try_recv() {
                        match message {
                            MidiMessage::NoteOn(_, note, _) => {
                                notes[usize::from(u8::from(note))] += 1;
                                ons += 1;
                            }
                            MidiMessage::NoteOff(_, note, _) => {
                                let count = &mut notes[usize::from(u8::from(note))];
                                assert!(*count > 0, "unmatched release");
                                *count -= 1;
                                offs += 1;
                            }
                            _ => {}
                        }
                    }
                    sequence = sequence.wrapping_add(if commands.len() == 8 { 2 } else { 1 });
                }
                let loss = loss_rx.try_recv().unwrap();
                assert!(loss.recovered);
                assert_eq!(loss.lost_packets, 1);
                assert_eq!(notes, [0; 128], "journal left an overlapping voice active");
            }
            assert_eq!((ons, offs), (10_000, 10_000));
        })
        .await
        .expect("polyphonic journal UDP regression timed out");
    }

    #[tokio::test]
    async fn journal_recovers_missing_sysex_fragment_before_current_packet_continuation() {
        use crate::packets::midi_packets::midi_packet::MidiPacket;
        use crate::sessions::events::event_handling::{PacketLossEvent, SysExPacketEvent};
        use zerocopy::network_endian::U16;
        tokio::time::timeout(Duration::from_secs(3), async {
            let receiver = RtpMidiSession::bind(0, "Fragment recovery", 1)
                .await
                .unwrap();
            let sender = RtpMidiSession::bind(0, "Source", 2).await.unwrap();
            let mut source = sender.local_addr().unwrap();
            source.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            receiver.participants.lock().await.insert(
                U32::new(2),
                Participant::new(source, true, None, c"Source", U32::new(2)),
            );
            let mut destination = receiver.midi_port.socket().local_addr().unwrap();
            destination.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            let (tx, mut rx) = tokio::sync::mpsc::channel(2);
            receiver
                .add_listener(SysExPacketEvent, move |payload| {
                    tx.try_send(payload.to_vec()).unwrap();
                })
                .await;
            let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel(1);
            receiver
                .add_listener(PacketLossEvent, move |loss| {
                    loss_tx.try_send(loss).unwrap();
                })
                .await;
            let mut states = crate::sessions::receive_state::ReceiveStates::new();
            let mut buffer = [0; MAX_MIDI_PACKET_SIZE];
            for (sequence, head, payload, tail, journal) in [
                (0, 0xF0, &[1, 2][..], 0xF0, &[64, 0, 0, 4, 4, 0x21, 6][..]),
                (
                    2,
                    0xF7,
                    &[4][..],
                    0xF7,
                    &[64, 0, 1, 4, 6, 0x38, 7, 2, 0x83][..],
                ),
            ] {
                let commands = [MidiEvent::new(
                    None,
                    RtpMidiMessage::SysExSegment {
                        head,
                        data: payload,
                        tail,
                    },
                )];
                let mut packet = MidiPacket::new_as_bytes(
                    U16::new(sequence),
                    U32::new(0),
                    U32::new(2),
                    &commands,
                    false,
                )
                .to_vec();
                packet[12] |= 64;
                packet.extend_from_slice(journal);
                sender
                    .midi_port
                    .socket()
                    .send_to(&packet, destination)
                    .await
                    .unwrap();
                receiver
                    .midi_port
                    .start(
                        &receiver,
                        receiver.listeners.clone(),
                        &mut buffer,
                        &mut states,
                    )
                    .await;
            }
            assert_eq!(rx.try_recv().unwrap(), [1, 2, 3, 4]);
            assert!(rx.try_recv().is_err());
            let loss = loss_rx.try_recv().unwrap();
            assert_eq!(loss.lost_packets, 1);
            assert!(loss.recovered);
        })
        .await
        .expect("SysEx journal recovery UDP test timed out");
    }

    #[tokio::test]
    async fn listener_publication_is_concurrent_and_preserves_packet_snapshots() {
        use crate::sessions::events::event_handling::MidiMessageEvent;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let session = Arc::new(RtpMidiSession::bind(0, "Listeners", 4).await.unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let old = session.listeners.load();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let mut writers = Vec::new();
        for _ in 0..4 {
            let session = session.clone();
            let count = count.clone();
            let done = done_tx.clone();
            writers.push(std::thread::spawn(move || {
                for _ in 0..16 {
                    let count = count.clone();
                    futures::executor::block_on(session.add_listener(
                        MidiMessageEvent,
                        move |_| {
                            count.fetch_add(1, Ordering::Relaxed);
                        },
                    ));
                }
                done.send(()).unwrap();
            }));
        }
        let completed = (0..4).all(|_| done_rx.recv_timeout(Duration::from_secs(2)).is_ok());
        old.notify_midi_message(midi_types::MidiMessage::Reset, 0, 4, Instant::now());
        assert_eq!(count.load(Ordering::Relaxed), 0);
        // Release the snapshot before joining even if a regression blocked a writer.
        drop(old);
        for writer in writers {
            writer.join().unwrap();
        }
        assert!(completed, "listener writers waited for a packet snapshot");
        session.listeners.load().notify_midi_message(
            midi_types::MidiMessage::Reset,
            0,
            4,
            Instant::now(),
        );
        assert_eq!(count.load(Ordering::Relaxed), 64);
    }

    #[tokio::test]
    async fn ephemeral_sessions_hold_distinct_adjacent_port_pairs() {
        let first = RtpMidiSession::bind(0, "Ephemeral A", 1).await.unwrap();
        let second = RtpMidiSession::bind(0, "Ephemeral B", 2).await.unwrap();
        let first_control = first.control_port.socket().local_addr().unwrap().port();
        let first_midi = first.midi_port.socket().local_addr().unwrap().port();
        let second_control = second.control_port.socket().local_addr().unwrap().port();
        let second_midi = second.midi_port.socket().local_addr().unwrap().port();
        assert_ne!(first_control, 0);
        assert_eq!(first_midi, first_control + 1);
        assert_eq!(second_midi, second_control + 1);
        for port in [first_control, first_midi] {
            assert!(![second_control, second_midi].contains(&port));
            assert!(std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).is_err());
        }
        drop(first);
        for port in [first_control, first_midi] {
            let released =
                std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).unwrap();
            drop(released);
        }
        assert!(
            matches!(RtpMidiSession::start(u16::MAX, "Invalid", 3, InviteResponder::Accept).await,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput)
        );
    }

    #[tokio::test]
    async fn concurrent_stops_wait_for_tasks_and_clear_pending_invitations() {
        let session = loop {
            let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            if let Ok(session) = RtpMidiSession::bind(port, "Shutdown", 1).await {
                break Arc::new(session);
            }
        };
        session.pending_invitations.lock().await.insert(
            U32::new(2),
            PendingInvitation {
                addr: "127.0.0.1:5004".parse().unwrap(),
                token: U32::new(2),
                name: CString::default(),
                created: Instant::now(),
                ssrc: U32::ZERO,
            },
        );
        let release = Arc::new(tokio::sync::Notify::new());
        let task_release = release.clone();
        session
            .task_handles
            .lock()
            .unwrap()
            .push(tokio::spawn(async move {
                task_release.notified().await;
            }));
        let first_session = session.clone();
        let first = tokio::spawn(async move { first_session.stop_gracefully().await });
        session.cancel_token.cancelled().await;
        let second_session = session.clone();
        let mut second = tokio::spawn(async move { second_session.stop_gracefully().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut second)
                .await
                .is_err()
        );
        assert!(!first.is_finished());
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            first.await.unwrap();
            second.await.unwrap();
        })
        .await
        .expect("shutdown did not join registered tasks");
        assert!(session.pending_invitations.lock().await.is_empty());
        session
            .invite_participant("127.0.0.1:5004".parse().unwrap())
            .await;
        assert!(session.pending_invitations.lock().await.is_empty());
    }
}
