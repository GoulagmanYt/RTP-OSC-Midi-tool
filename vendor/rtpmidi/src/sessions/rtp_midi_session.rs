use std::collections::HashMap;
use std::ffi::CString;
use std::net::SocketAddr;
use std::sync::Arc;
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
    pub(super) participants: Arc<Mutex<HashMap<U32, Participant>>>, // key by ssrc
    pub(super) pending_invitations: Arc<Mutex<HashMap<U32, PendingInvitation>>>, // key by token
    pub(super) midi_port: Arc<MidiPort>,

    listeners: Arc<Mutex<EventListeners>>,
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

        let mut context = RtpMidiSession {
            participants: Arc::new(Mutex::new(HashMap::new())),
            pending_invitations: Arc::new(Mutex::new(HashMap::new())),
            control_port: Arc::new(
                ControlPort::bind(port, cstr_name.to_owned(), U32::new(ssrc)).await?,
            ),
            midi_port: Arc::new(
                MidiPort::bind(port + 1, cstr_name.to_owned(), U32::new(ssrc)).await?,
            ),
            host_syncer: Arc::new(HostSyncer::new()),
            listeners: Arc::new(Mutex::new(EventListeners::new())),
            cancel_token: Arc::new(CancellationToken::new()),
            task_handles: Arc::new(std::sync::Mutex::new(Vec::new())),
            name: cstr_name,
            lifetime: None,
            stop_lock: Arc::new(Mutex::new(())),
            #[cfg(feature = "mdns")]
            mdns: advertise_mdns(name, port).map_err(|e| std::io::Error::other(e.to_string()))?,
        };
        context.lifetime = Some(Arc::new(SessionLifetime {
            cancel_token: context.cancel_token.clone(),
            #[cfg(feature = "mdns")]
            mdns: context.mdns.clone(),
        }));
        Ok(context)
    }

    #[instrument(skip(port),fields(control_port = %port, midi_port = %port + 1))]
    pub async fn start(
        port: u16,
        name: &str,
        ssrc: u32,
        invite_handler: InviteResponder,
    ) -> std::io::Result<Arc<Self>> {
        event!(tracing::Level::INFO, "Starting RTP-MIDI session");
        let ctx = Arc::new(Self::bind(port, name, ssrc).await?);
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
            loop {
                tokio::select! {
                    biased;
                    _ = midi_cancel_token.cancelled() => {
                        event!(Level::DEBUG, "listen_for_midi: cancellation requested");
                        break;
                    },
                    _ = midi_port_listener.start(&ctx_midi, listeners_midi.clone(), &mut buf) => {}
                }
            }
        });
        handles.push(handle);

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
        event!(Level::INFO, "Removing participant");
        self.control_port.send_termination_packet(participant).await;
        self.midi_port.send_termination_packet(participant).await;
        self.forget_participant(participant.ssrc()).await;
    }

    pub(super) async fn notify_joined(&self, participant: &Participant) {
        self.listeners
            .lock()
            .await
            .notify_participant_joined(participant);
    }

    pub(super) async fn forget_participant(&self, ssrc: U32) {
        let participant = self.participants.lock().await.remove(&ssrc);
        self.midi_port.clear_receive_sequence(ssrc).await;
        if let Some(participant) = participant {
            self.listeners
                .lock()
                .await
                .notify_participant_left(&participant);
        }
    }

    pub(super) async fn handle_termination(
        &self,
        ssrc: U32,
        token: U32,
        src: SocketAddr,
        midi: bool,
    ) {
        let valid = self.participants.lock().await.get(&ssrc).is_some_and(|p| {
            (if midi { p.midi_port_addr() } else { p.addr() }) == src
                && p.initiator_token() == Some(token)
        });
        if valid {
            self.forget_participant(ssrc).await;
        }
    }

    pub async fn add_listener<E, F>(&self, _event_type: E, callback: F)
    where
        E: EventType,
        F: for<'a> Fn(E::Data<'a>) + Send + 'static,
    {
        let mut listeners = self.listeners.lock().await;
        E::add_listener_to_storage(&mut listeners, callback);
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
