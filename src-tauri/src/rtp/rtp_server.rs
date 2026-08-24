use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::async_runtime::{self, JoinHandle};
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot, Notify};
use uuid::Uuid;

use rtpmidi::sessions::{
    events::event_handling::{
        PacketLossEvent, ParticipantJoinedEvent, ParticipantLeftEvent, TimestampedMidiMessageEvent,
    },
    invite_responder::InviteResponder,
    rtp_midi_session::RtpMidiSession,
};

use crate::{
    bridge::pipeline::{request_critical_midi_reset, try_enqueue_midi_frame},
    logger::{logs_enabled, FrontendLogger},
    midi::{is_critical_release_message, MidiFrame},
    tauri::utils::safe_block_on,
    types::RtpParticipantInfo,
};

use super::rtp_advertisement::{RtpAdvertisementStatus, RtpMdnsAdvertisement};
use super::rtp_midi::{midi_to_bytes, participant_matches_target};

const RTP_PARTICIPANTS_EVENT: &str = "rtp:participants";
const RTP_JITTER_BUFFER: Duration = Duration::from_millis(3);
const RTP_MAX_SCHEDULE_AHEAD: Duration = Duration::from_millis(50);
const RTP_SCHEDULER_CAPACITY: usize = 2_048;
const RTP_PENDING_CAPACITY: usize = 4_096;

/// Compteur de messages MIDI abandonnés faute de place dans le canal.
/// Incrémenté de façon atomique dans le callback temps-réel, lu depuis
/// le thread de log périodique.
static DROPPED_MIDI_COUNT: AtomicU64 = AtomicU64::new(0);
static RTP_LOG_SAMPLE_LAST_MS: AtomicU64 = AtomicU64::new(0);

/// Retourne le nombre de messages MIDI RTP abandonnés (canal plein).
pub fn rtp_dropped_count() -> u64 {
    DROPPED_MIDI_COUNT.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpRemoteTarget {
    pub name: String,
    pub addr: SocketAddr,
}

struct RemoteTargetState {
    target: RtpRemoteTarget,
    backoff: Duration,
    next_attempt: Instant,
}

pub struct RtpServer {
    stop_tx: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
    bound_port: u16,
    participants: Arc<Mutex<Vec<RtpParticipantInfo>>>,
    advertisement: Option<RtpMdnsAdvertisement>,
}

impl RtpServer {
    pub fn start(
        name: String,
        port: u16,
        remote_targets: Vec<RtpRemoteTarget>,
        log_rtp: bool,
        sink: Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
        logger: FrontendLogger,
    ) -> Result<Self, String> {
        let (stop_tx, mut stop_rx) = oneshot::channel();

        if port == u16::MAX {
            return Err("Port RTP invalide: 65535 reserve".to_string());
        }

        let (session, bound_port) = safe_block_on(async {
            let ssrc = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
            let try_ports = [0u16, 2, 4]
                .into_iter()
                .filter_map(|offset| port.checked_add(offset))
                .filter(|candidate| *candidate < u16::MAX)
                .collect::<smallvec::SmallVec<[u16; 3]>>();
            if try_ports.is_empty() {
                return Err("Impossible de demarrer RTP-MIDI (port invalide)".to_string());
            }
            for p in try_ports {
                // rtpmidi can release its UDP sockets a few scheduler ticks
                // after stop_gracefully() completes. Retry the preferred pair
                // briefly so an immediate bridge restart keeps ports 5004/5005.
                let retry_delays_ms = if p == port {
                    &[0u64, 25, 50, 100, 200][..]
                } else {
                    &[0u64][..]
                };
                for (attempt, delay_ms) in retry_delays_ms.iter().enumerate() {
                    if *delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
                    }
                    match RtpMidiSession::start(p, &name, ssrc, InviteResponder::Accept).await {
                        Ok(session) => return Ok((session, p)),
                        Err(err)
                            if err.kind() == std::io::ErrorKind::AddrInUse
                                && attempt + 1 < retry_delays_ms.len() => {}
                        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                            if let (Some(next_ctrl), Some(next_data)) =
                                (p.checked_add(2), p.checked_add(3))
                            {
                                logger.warn(format!(
                                    "Port RTP {p} occupé, tentative sur {next_ctrl}/{next_data}"
                                ));
                            } else {
                                logger.warn(format!("Port RTP {p} occupé, autre tentative"));
                            }
                            break;
                        }
                        Err(err) => {
                            logger.error(format!("Erreur RTP-MIDI: {err}"));
                            return Err(err.to_string());
                        }
                    }
                }
            }
            Err("Impossible de démarrer RTP-MIDI (ports occupés)".to_string())
        })?;

        logger.info(format!(
            "Serveur RTP-MIDI \"{name}\" ouvert sur ports {}/{} (control/data)",
            bound_port,
            bound_port + 1
        ));
        let advertisement = RtpMdnsAdvertisement::start(&name, bound_port, &logger);
        if !remote_targets.is_empty() {
            for target in &remote_targets {
                logger.info(format!(
                    "RTP-MIDI remote target configured: {} ({})",
                    target.name, target.addr
                ));
            }
        }

        // La source est construite une seule fois et partagée via Arc.
        // NOTE : si MidiFrame.source est changé en Arc<str>, remplacer
        // rtp_source.to_string() par Arc::clone(&rtp_source) dans le
        // callback pour éliminer l'allocation heap par message.
        let rtp_source: Arc<str> = Arc::from(format!("RTP:{name}"));
        let midi_sink = sink.clone();
        let participants = Arc::new(Mutex::new(Vec::<RtpParticipantInfo>::new()));
        let participants_for_join = participants.clone();
        let participants_for_left = participants.clone();
        let participants_for_invite = participants.clone();
        let notify = Arc::new(Notify::new());
        let notify_for_join = notify.clone();
        let notify_for_left = notify.clone();
        emit_participants(&logger, &participants);

        let handle = async_runtime::spawn(async move {
            let (scheduled_tx, scheduled_rx) = mpsc::channel(RTP_SCHEDULER_CAPACITY);
            let scheduler =
                async_runtime::spawn(run_rtp_scheduler(scheduled_rx, midi_sink, logger.clone()));
            // ── Listener MIDI ────────────────────────────────────────────────
            // HOT PATH : ce callback est appelé sur chaque message reçu.
            // Règles :
            //   • Pas de verrou long-durée.
            //   • Pas d'allocation évitable.
            //   • File bornée et ordonnancement selon le timestamp RTP.
            let rtp_logger = logger.clone();
            session
                .add_listener(TimestampedMidiMessageEvent, move |event| {
                    let bytes = midi_to_bytes(event.message);

                    // Logging RTP optionnel : le formatage + IPC frontend sont coûteux.
                    // On ne l'active QUE si le mode développeur (verbose) est activé,
                    // Le sampler atomique borne ce chemin à une tâche par seconde :
                    // le callback UDP ne réalise donc aucune I/O de journalisation.
                    if log_rtp
                        && logs_enabled()
                        && crate::logger::should_log_debug()
                        && should_sample_rtp_log()
                    {
                        let sampled_logger = rtp_logger.clone();
                        let sampled_bytes = bytes.clone();
                        async_runtime::spawn(async move {
                            sampled_logger.debug(format!(
                                "RTP MIDI: {:02X?} timestamp={}",
                                sampled_bytes.as_slice(),
                                event.timestamp
                            ));
                        });
                    }

                    let timed = TimedRtpFrame {
                        frame: MidiFrame {
                            data: bytes,
                            source: std::sync::Arc::clone(&rtp_source),
                        },
                        timestamp: event.timestamp,
                        ssrc: event.ssrc,
                        arrived: Instant::now(),
                    };
                    if let Err(error) = scheduled_tx.try_send(timed) {
                        DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
                        if is_critical_release_message(error.into_inner().frame.data.as_slice()) {
                            request_critical_midi_reset();
                        }
                    }
                })
                .await;

            // ── Listener : participant rejoint ────────────────────────────────
            let join_logger = logger.clone();
            session
                .add_listener(ParticipantJoinedEvent, move |participant| {
                    let name = participant.name().to_str().unwrap_or("Unknown");
                    let info = RtpParticipantInfo {
                        name: name.to_string(),
                        addr: participant.addr().to_string(),
                    };
                    {
                        let mut guard = participants_for_join.lock();
                        if !guard.iter().any(|p| p.addr == info.addr) {
                            guard.push(info);
                        }
                    }
                    emit_participants(&join_logger, &participants_for_join);
                    notify_for_join.notify_one();
                    join_logger.info(format!(
                        "RTP-MIDI participant joined: {name} ({})",
                        participant.addr()
                    ));
                })
                .await;

            // ── Listener : participant parti ──────────────────────────────────
            let left_logger = logger.clone();
            session
                .add_listener(ParticipantLeftEvent, move |participant| {
                    request_critical_midi_reset();
                    let name = participant.name().to_str().unwrap_or("Unknown");
                    {
                        let mut guard = participants_for_left.lock();
                        guard.retain(|p| p.addr != participant.addr().to_string());
                    }
                    emit_participants(&left_logger, &participants_for_left);
                    notify_for_left.notify_one();
                    left_logger.info(format!(
                        "RTP-MIDI participant left: {name} ({})",
                        participant.addr()
                    ));
                })
                .await;

            let loss_logger = logger.clone();
            session
                .add_listener(PacketLossEvent, move |loss| {
                    // RFC 6295 recovery journals are not emitted by every peer. A
                    // deterministic controller reset is safer than leaving notes or
                    // sustain latched after a proven RTP sequence gap.
                    request_critical_midi_reset();
                    if should_sample_rtp_log() {
                        loss_logger.warn(format!(
                            "RTP-MIDI packet loss: ssrc={} expected={} received={} lost={}",
                            loss.ssrc,
                            loss.expected_sequence,
                            loss.received_sequence,
                            loss.lost_packets
                        ));
                    }
                })
                .await;

            // ── Boucle de reconnexion vers les cibles distantes ───────────────
            if !remote_targets.is_empty() {
                let mut remote_states = remote_targets
                    .into_iter()
                    .map(|target| RemoteTargetState {
                        target,
                        backoff: Duration::from_secs(1),
                        next_attempt: Instant::now(),
                    })
                    .collect::<Vec<_>>();

                loop {
                    let now = Instant::now();
                    let connected_addrs: Vec<String> = {
                        let guard = participants_for_invite.lock();
                        guard.iter().map(|p| p.addr.clone()).collect()
                    };

                    let mut next_wake: Option<Instant> = None;

                    for state in remote_states.iter_mut() {
                        if connected_addrs
                            .iter()
                            .any(|addr| participant_matches_target(addr, &state.target.addr))
                        {
                            // Déjà connecté : on réinitialise le backoff pour la prochaine
                            // déconnexion éventuelle.
                            state.backoff = Duration::from_secs(1);
                            state.next_attempt = now + state.backoff;
                            continue;
                        }

                        if now >= state.next_attempt {
                            session.invite_participant(state.target.addr).await;
                            state.backoff = (state.backoff * 2).min(Duration::from_secs(30));
                            state.next_attempt = now + state.backoff;
                        }

                        // next_wake = minimum des prochaines tentatives
                        next_wake = Some(
                            next_wake.map_or(state.next_attempt, |c| c.min(state.next_attempt)),
                        );
                    }

                    if let Some(next) = next_wake {
                        let wait = next.saturating_duration_since(Instant::now());
                        tokio::select! {
                            _ = &mut stop_rx => {
                                logger.info("Arret serveur RTP-MIDI");
                                session.stop_gracefully().await;
                                break;
                            }
                            _ = notify.notified() => {}
                            _ = tokio::time::sleep(wait) => {}
                        }
                    } else {
                        tokio::select! {
                            _ = &mut stop_rx => {
                                logger.info("Arret serveur RTP-MIDI");
                                session.stop_gracefully().await;
                                break;
                            }
                            _ = notify.notified() => {}
                        }
                    }
                }
            } else {
                tokio::select! {
                    _ = &mut stop_rx => {
                        logger.info("Arret serveur RTP-MIDI");
                        session.stop_gracefully().await;
                    }
                }
            }
            scheduler.abort();
            let _ = scheduler.await;
        });

        Ok(Self {
            stop_tx: Some(stop_tx),
            handle,
            bound_port,
            participants,
            advertisement: Some(advertisement),
        })
    }

    pub async fn stop(mut self) {
        if let Some(advertisement) = self.advertisement.take() {
            advertisement.shutdown();
        }
        if let Some(stop_tx) = self.stop_tx {
            let _ = stop_tx.send(());
        }
        let _ = self.handle.await;
    }

    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }

    pub fn participants(&self) -> Vec<RtpParticipantInfo> {
        self.participants.lock().clone()
    }

    pub fn advertisement_status(&self) -> RtpAdvertisementStatus {
        self.advertisement
            .as_ref()
            .map(RtpMdnsAdvertisement::status)
            .unwrap_or_default()
    }
}

struct TimedRtpFrame {
    frame: MidiFrame,
    timestamp: u32,
    ssrc: u32,
    arrived: Instant,
}

struct RtpClockMap {
    origin_timestamp: u64,
    last_timestamp: u64,
    origin_local: Instant,
}

impl RtpClockMap {
    fn new(timestamp: u32, arrived: Instant) -> Self {
        Self {
            origin_timestamp: u64::from(timestamp),
            last_timestamp: u64::from(timestamp),
            origin_local: arrived + RTP_JITTER_BUFFER,
        }
    }

    fn deadline(&mut self, timestamp: u32, arrived: Instant) -> Instant {
        const WRAP: u64 = 1u64 << 32;
        const HALF_WRAP: u64 = WRAP / 2;
        let base = self.last_timestamp & !(WRAP - 1);
        let mut extended = base | u64::from(timestamp);
        if extended.saturating_add(HALF_WRAP) < self.last_timestamp {
            extended = extended.saturating_add(WRAP);
        } else if extended > self.last_timestamp.saturating_add(HALF_WRAP) {
            extended = extended.saturating_sub(WRAP);
        }
        self.last_timestamp = self.last_timestamp.max(extended);
        let ticks = extended.saturating_sub(self.origin_timestamp);
        let mapped = self.origin_local + Duration::from_micros(ticks.saturating_mul(100));
        mapped.max(arrived).min(arrived + RTP_MAX_SCHEDULE_AHEAD)
    }
}

async fn run_rtp_scheduler(
    mut receiver: mpsc::Receiver<TimedRtpFrame>,
    sink: Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
    logger: FrontendLogger,
) {
    let mut clocks = HashMap::<u32, RtpClockMap>::new();
    let mut pending = BTreeMap::<Instant, Vec<MidiFrame>>::new();
    let mut pending_count = 0usize;
    loop {
        if pending.is_empty() {
            let Some(event) = receiver.recv().await else {
                break;
            };
            schedule_rtp_frame(event, &mut clocks, &mut pending, &mut pending_count);
            continue;
        }

        let Some((&deadline, _)) = pending.first_key_value() else {
            continue;
        };
        tokio::select! {
            event = receiver.recv() => {
                let Some(event) = event else {
                    flush_scheduled_frames(&mut pending, &sink, &mut pending_count, true);
                    break;
                };
                schedule_rtp_frame(event, &mut clocks, &mut pending, &mut pending_count);
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                flush_scheduled_frames(&mut pending, &sink, &mut pending_count, false);
            }
        }
    }
    logger.debug("RTP-MIDI timestamp scheduler stopped");
}

fn schedule_rtp_frame(
    event: TimedRtpFrame,
    clocks: &mut HashMap<u32, RtpClockMap>,
    pending: &mut BTreeMap<Instant, Vec<MidiFrame>>,
    pending_count: &mut usize,
) {
    if *pending_count >= RTP_PENDING_CAPACITY {
        DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
        if is_critical_release_message(event.frame.data.as_slice()) {
            request_critical_midi_reset();
        }
        return;
    }
    let clock = clocks
        .entry(event.ssrc)
        .or_insert_with(|| RtpClockMap::new(event.timestamp, event.arrived));
    let deadline = clock.deadline(event.timestamp, event.arrived);
    pending.entry(deadline).or_default().push(event.frame);
    *pending_count += 1;
}

fn flush_scheduled_frames(
    pending: &mut BTreeMap<Instant, Vec<MidiFrame>>,
    sink: &Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
    pending_count: &mut usize,
    flush_all: bool,
) {
    let now = Instant::now();
    while pending
        .first_key_value()
        .is_some_and(|(deadline, _)| flush_all || *deadline <= now)
    {
        let Some((_, frames)) = pending.pop_first() else {
            break;
        };
        *pending_count = (*pending_count).saturating_sub(frames.len());
        let tx = sink.read().as_ref().cloned();
        for frame in frames {
            let Some(tx) = tx.as_ref() else {
                continue;
            };
            if try_enqueue_midi_frame(tx, frame).is_err() {
                DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn should_sample_rtp_log() -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let last = RTP_LOG_SAMPLE_LAST_MS.load(Ordering::Relaxed);
    now.saturating_sub(last) >= 1_000
        && RTP_LOG_SAMPLE_LAST_MS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

fn emit_participants(logger: &FrontendLogger, participants: &Arc<Mutex<Vec<RtpParticipantInfo>>>) {
    let snapshot = participants.lock().clone();
    let _ = logger.app_handle().emit(RTP_PARTICIPANTS_EVENT, snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtp_callback_uses_bounded_enqueue_path() {
        let source = include_str!("rtp_server.rs");

        let forbidden = ["tx", ".send(MidiFrame"].concat();
        assert!(source.contains("try_enqueue_midi_frame"));
        assert!(source.contains("&tx"));
        assert!(source.contains("MidiFrame"));
        assert!(!source.contains(&forbidden));
    }

    #[test]
    fn rtp_clock_map_preserves_delta_and_handles_wrap() {
        let arrived = Instant::now();
        let mut clock = RtpClockMap::new(u32::MAX - 5, arrived);
        let first = clock.deadline(u32::MAX - 5, arrived);
        let after_wrap = clock.deadline(4, arrived);

        assert_eq!(first, arrived + RTP_JITTER_BUFFER);
        assert_eq!(after_wrap.duration_since(first), Duration::from_millis(1));
    }

    #[test]
    fn rtp_clock_map_bounds_malicious_future_timestamp() {
        let arrived = Instant::now();
        let mut clock = RtpClockMap::new(0, arrived);
        let deadline = clock.deadline(1_000_000, arrived);

        assert_eq!(deadline, arrived + RTP_MAX_SCHEDULE_AHEAD);
    }
}
