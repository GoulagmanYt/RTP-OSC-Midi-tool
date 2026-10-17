use crate::midi_queue::Sender;
use parking_lot::{Mutex, RwLock};
use std::collections::{BinaryHeap, HashMap};
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::async_runtime;
use tauri::Emitter;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;
use uuid::Uuid;

use rtpmidi::sessions::{
    events::event_handling::{
        PacketLossEvent, ParticipantJoinedEvent, ParticipantLeftEvent, StreamFaultEvent,
        TimestampedMidiMessageEvent, TimestampedSysExEvent,
    },
    invite_responder::InviteResponder,
    rtp_midi_session::RtpMidiSession,
};

use crate::{
    bridge::pipeline::{
        classify_input_frame, midi_reset_generation, request_critical_midi_reset,
        try_enqueue_midi_frame,
    },
    logger::{logs_enabled, FrontendLogger},
    midi::MidiFrame,
    tauri::utils::safe_block_on,
    types::RtpParticipantInfo,
};

use super::rtp_advertisement::{RtpAdvertisementStatus, RtpMdnsAdvertisement};
use super::rtp_midi::{midi_to_bytes, participant_matches_target};

const RTP_PARTICIPANTS_EVENT: &str = "rtp:participants";
const RTP_MAX_SCHEDULE_AHEAD: Duration = Duration::from_secs(10);
const RTP_SCHEDULER_CAPACITY: usize = 2_048;
const RTP_PENDING_CAPACITY: usize = 4_096;

/// Compteur de messages MIDI abandonnés faute de place dans le canal.
/// Incrémenté de façon atomique dans le callback temps-réel, lu depuis
/// le thread de log périodique.
static DROPPED_MIDI_COUNT: AtomicU64 = AtomicU64::new(0);
static RTP_LOG_SAMPLE_LAST_MS: AtomicU64 = AtomicU64::new(0);
static RTP_LOSS_LOG_SAMPLE_LAST_MS: AtomicU64 = AtomicU64::new(0);

/// Retourne le nombre de messages MIDI RTP abandonnés (canal plein).
pub(crate) fn record_rtp_drop() {
    DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
}

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
    _runtime: crate::realtime::MidiRuntime,
    outgoing: crate::midi_queue::AsyncSender<super::rtp_output::OutgoingFrame>,
    stop_tx: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
    bound_port: u16,
    participants: Arc<Mutex<Vec<RtpParticipantInfo>>>,
    advertisement: Option<RtpMdnsAdvertisement>,
}

pub fn candidate_ports(port: u16) -> smallvec::SmallVec<[u16; 3]> {
    if port == 0 {
        smallvec::smallvec![0u16]
    } else {
        [0u16, 2, 4]
            .into_iter()
            .filter_map(|offset| port.checked_add(offset))
            .filter(|candidate| *candidate < u16::MAX)
            .collect()
    }
}

pub(crate) fn peer_source(name: &str, ssrc: u32) -> Arc<str> {
    Arc::from(format!("RTP:{name}#ssrc_{ssrc:08x}"))
}

fn cached_peer_source(peers: &RwLock<HashMap<u32, Arc<str>>>, name: &str, ssrc: u32) -> Arc<str> {
    // End the read guard before acquiring the write lock on a cache miss.
    let cached = peers.read().get(&ssrc).cloned();
    if let Some(source) = cached {
        source
    } else {
        peers
            .write()
            .entry(ssrc)
            .or_insert_with(|| peer_source(name, ssrc))
            .clone()
    }
}

impl RtpServer {
    pub fn start(
        name: String,
        port: u16,
        remote_targets: Vec<RtpRemoteTarget>,
        log_rtp: bool,
        sink: Arc<arc_swap::ArcSwapOption<Sender<MidiFrame>>>,
        logger: FrontendLogger,
    ) -> Result<Self, String> {
        crate::midi::initialize_midi_buffers();
        let (stop_tx, mut stop_rx) = oneshot::channel();

        if port == u16::MAX {
            return Err("Port RTP invalide: 65535 reserve".to_string());
        }

        let runtime = crate::realtime::MidiRuntime::new("rtp-network")?;
        let session_name = name.clone();
        let setup_logger = logger.clone();
        let (session, bound_port) = safe_block_on(runtime.spawn(async move {
            let name = session_name;
            let logger = setup_logger;
            let ssrc = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
            let try_ports = candidate_ports(port);
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
                    match RtpMidiSession::start_with_recovery_generation(
                        p,
                        &name,
                        ssrc,
                        InviteResponder::Accept,
                        crate::bridge::pipeline::midi_reset_generation_source(),
                    )
                    .await
                    {
                        Ok(session) => {
                            let bound_port =
                                session.local_addr().map_err(|err| err.to_string())?.port();
                            return Ok((session, bound_port));
                        }
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
        }))
        .map_err(|error| error.to_string())??;

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

        // La source par défaut et le registre des sources qualifiées par pair distant.
        let rtp_source: Arc<str> = Arc::from(format!("RTP:{name}"));
        let peer_sources = Arc::new(RwLock::new(HashMap::<u32, Arc<str>>::new()));
        let peer_sources_for_join = Arc::clone(&peer_sources);
        let peer_sources_for_left = Arc::clone(&peer_sources);
        let peer_sources_for_midi = Arc::clone(&peer_sources);
        let peer_sources_for_sysex = Arc::clone(&peer_sources);
        let session_name_for_join = name.clone();
        let session_name_for_midi = name.clone();
        let session_name_for_sysex = name.clone();
        let source_faults = Arc::new(SourceRecovery::default());
        let midi_sink = sink.clone();
        let participants = Arc::new(Mutex::new(Vec::<RtpParticipantInfo>::new()));
        let participants_for_join = participants.clone();
        let participants_for_left = participants.clone();
        let participants_for_invite = participants.clone();
        let notify = Arc::new(Notify::new());
        let notify_for_join = notify.clone();
        let notify_for_left = notify.clone();
        emit_participants(&logger, &participants);

        let (outgoing, outgoing_rx) = crate::midi_queue::async_channel(2048);
        let handle = runtime.spawn(async move {
            let (sender_stop, sender_cancelled) = oneshot::channel();
            let mut sender_stop = Some(sender_stop);
            let mut sender_task = tokio::spawn(super::rtp_output::run(
                session.clone(),
                outgoing_rx,
                sender_cancelled,
            ));
            let (scheduled_tx, scheduled_rx) =
                crate::midi_queue::async_channel(RTP_SCHEDULER_CAPACITY);
            let scheduler = tokio::spawn(run_rtp_scheduler(
                scheduled_rx,
                midi_sink.clone(),
                logger.clone(),
                source_faults.clone(),
            ));
            // ── Listener MIDI ────────────────────────────────────────────────
            // HOT PATH : ce callback est appelé sur chaque message reçu.
            // Règles :
            //   • Pas de verrou long-durée.
            //   • Pas d'allocation évitable.
            //   • File bornée et ordonnancement selon le timestamp RTP.
            let fault_sources = peer_sources.clone();
            let fault_fallback = rtp_source.clone();
            let fault_requests = source_faults.clone();
            session
                .add_listener(StreamFaultEvent, move |ssrc| {
                    record_rtp_drop();
                    request_peer_recovery(ssrc, &fault_sources, &fault_fallback, &fault_requests);
                })
                .await;
            let sysex_tx = scheduled_tx.clone();
            let sysex_source = rtp_source.clone();
            let sysex_recovery = source_faults.clone();
            session
                .add_listener(TimestampedSysExEvent, move |event| {
                    let source = if event.ssrc != 0 {
                        cached_peer_source(
                            &peer_sources_for_sysex,
                            &session_name_for_sysex,
                            event.ssrc,
                        )
                    } else {
                        sysex_source.clone()
                    };
                    let Some(mut frame) = MidiFrame::sysex_realtime(event.payload, source) else {
                        record_rtp_drop();
                        request_critical_midi_reset();
                        return;
                    };
                    frame.ingress_qpc = crate::vst_worker::protocol::qpc_at(event.received_at);
                    if sysex_tx
                        .try_send(TimedRtpFrame {
                            source_epoch: sysex_recovery.epoch(&frame.source),
                            generation: frame.generation,
                            critical: false,
                            frame,
                            deadline: event.deadline,
                            order: 0,
                        })
                        .is_err()
                    {
                        DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .await;
            let rtp_logger = logger.clone();
            let midi_tx = scheduled_tx.clone();
            let midi_source = rtp_source.clone();
            let midi_recovery = source_faults.clone();
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
                        && should_sample_rtp_log(&RTP_LOG_SAMPLE_LAST_MS)
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

                    let source = if event.ssrc != 0 {
                        cached_peer_source(
                            &peer_sources_for_midi,
                            &session_name_for_midi,
                            event.ssrc,
                        )
                    } else {
                        std::sync::Arc::clone(&midi_source)
                    };
                    let generation = MidiFrame::current_generation();
                    let timed = TimedRtpFrame {
                        source_epoch: midi_recovery.epoch(&source),
                        frame: MidiFrame {
                            generation,
                            ingress_qpc: crate::vst_worker::protocol::qpc_at(event.received_at),
                            data: bytes,
                            source,
                        },
                        deadline: event.deadline,
                        generation,
                        critical: false,
                        order: 0,
                    };
                    let mut timed = timed;
                    #[cfg(feature = "latency-diagnostics")]
                    crate::latency_trace::record_rtp(
                        timed.frame.data.as_slice(),
                        timed.frame.ingress_qpc,
                        event
                            .deadline
                            .saturating_duration_since(event.received_at)
                            .as_micros()
                            .min(u64::MAX as u128) as u64,
                    );
                    timed.critical = midi_sink
                        .load()
                        .as_ref()
                        .is_some_and(|sink| classify_input_frame(sink, &timed.frame));
                    if let Err(error) = midi_tx.try_send(timed) {
                        DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
                        if error.into_inner().critical {
                            request_critical_midi_reset();
                        }
                    }
                })
                .await;

            // ── Listener : participant rejoint ────────────────────────────────
            let join_logger = logger.clone();
            session
                .add_listener(ParticipantJoinedEvent, move |participant| {
                    let peer_ssrc = participant.ssrc().get();
                    let peer_addr = participant.addr();
                    peer_sources_for_join
                        .write()
                        .entry(peer_ssrc)
                        .or_insert_with(|| peer_source(&session_name_for_join, peer_ssrc));

                    let name = participant.name().to_str().unwrap_or("Unknown");
                    let info = RtpParticipantInfo {
                        name: name.to_string(),
                        addr: peer_addr.to_string(),
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
                        peer_addr
                    ));
                })
                .await;

            // ── Listener : participant parti ──────────────────────────────────
            let left_logger = logger.clone();
            let left_faults = source_faults.clone();
            let left_fallback = rtp_source.clone();
            session
                .add_listener(ParticipantLeftEvent, move |participant| {
                    let peer_ssrc = participant.ssrc().get();
                    request_peer_recovery(
                        peer_ssrc,
                        &peer_sources_for_left,
                        &left_fallback,
                        &left_faults,
                    );
                    peer_sources_for_left.write().remove(&peer_ssrc);
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
            let loss_faults = source_faults.clone();
            let loss_sources = peer_sources.clone();
            let loss_fallback = rtp_source.clone();
            session
                .add_listener(PacketLossEvent, move |loss| {
                    crate::realtime::RTP_LOST_PACKETS
                        .fetch_add(u64::from(loss.lost_packets), Ordering::Relaxed);
                    // RFC 6295 recovery journals are not emitted by every peer. A
                    // deterministic controller reset is safer than leaving notes or
                    // sustain latched after a proven RTP sequence gap.
                    if !loss.recovered {
                        request_peer_recovery(
                            loss.ssrc,
                            &loss_sources,
                            &loss_fallback,
                            &loss_faults,
                        );
                    }
                    if should_sample_rtp_log(&RTP_LOSS_LOG_SAMPLE_LAST_MS) {
                        let logger = loss_logger.clone();
                        async_runtime::spawn(async move {
                            logger.warn(format!(
                                "RTP-MIDI packet loss: ssrc={} expected={} received={} lost={}",
                                loss.ssrc,
                                loss.expected_sequence,
                                loss.received_sequence,
                                loss.lost_packets
                            ));
                        });
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
                                if let Some(stop) = sender_stop.take() { let _ = stop.send(()); }
                                let _ = (&mut sender_task).await;
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
                                if let Some(stop) = sender_stop.take() { let _ = stop.send(()); }
                                let _ = (&mut sender_task).await;
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
                        if let Some(stop) = sender_stop.take() { let _ = stop.send(()); }
                                let _ = (&mut sender_task).await;
                                session.stop_gracefully().await;
                    }
                }
            }
            scheduler.abort();
            let _ = scheduler.await;
        });

        Ok(Self {
            _runtime: runtime,
            outgoing,
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

    pub(crate) fn outgoing(
        &self,
    ) -> crate::midi_queue::AsyncSender<super::rtp_output::OutgoingFrame> {
        self.outgoing.clone()
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
    source_epoch: u64,
    critical: bool,
    frame: MidiFrame,
    deadline: Instant,
    generation: u64,
    order: u64,
}

impl PartialEq for TimedRtpFrame {
    fn eq(&self, other: &Self) -> bool {
        (self.deadline, self.order) == (other.deadline, other.order)
    }
}
impl Eq for TimedRtpFrame {}
impl PartialOrd for TimedRtpFrame {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TimedRtpFrame {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (other.deadline, other.order).cmp(&(self.deadline, self.order))
    }
}

#[derive(Default)]
struct SourceRecovery {
    state: Mutex<SourceRecoveryState>,
}
#[derive(Default)]
struct SourceRecoveryState {
    epochs: HashMap<Arc<str>, u64>,
    pending: HashMap<Arc<str>, u64>,
}
impl SourceRecovery {
    fn epoch(&self, source: &str) -> u64 {
        self.state.lock().epochs.get(source).copied().unwrap_or(0)
    }
    fn request(&self, source: Arc<str>) {
        let mut state = self.state.lock();
        let epoch = state.epochs.entry(source.clone()).or_default();
        *epoch = epoch.wrapping_add(1);
        let epoch = *epoch;
        state.pending.insert(source, epoch);
    }
}

fn request_peer_recovery(
    ssrc: u32,
    peers: &RwLock<HashMap<u32, Arc<str>>>,
    fallback: &Arc<str>,
    requests: &SourceRecovery,
) {
    let peers = peers.read();
    if ssrc == 0 {
        for source in peers.values() {
            requests.request(source.clone());
        }
        requests.request(fallback.clone());
    } else if let Some(source) = peers.get(&ssrc) {
        requests.request(source.clone());
    } else {
        requests.request(fallback.clone());
    }
}

fn source_frame_is_stale(frame: &TimedRtpFrame, cutoffs: &HashMap<Arc<str>, u64>) -> bool {
    cutoffs
        .get(&frame.frame.source)
        .is_some_and(|cutoff| frame.source_epoch < *cutoff)
}

fn recover_scheduled_source(
    source: Arc<str>,
    cutoff: u64,
    pending: &mut BinaryHeap<TimedRtpFrame>,
    cutoffs: &mut HashMap<Arc<str>, u64>,
    sink: &arc_swap::ArcSwapOption<Sender<MidiFrame>>,
) {
    pending.retain(|event| event.frame.source != source || event.source_epoch >= cutoff);
    cutoffs.insert(source.clone(), cutoff);
    let frame = MidiFrame::copy_realtime(&[0xFF], source).expect("inline reset");
    dispatch_scheduled_frame(sink, frame);
}

async fn run_rtp_scheduler(
    receiver: crate::midi_queue::AsyncReceiver<TimedRtpFrame>,
    sink: Arc<arc_swap::ArcSwapOption<Sender<MidiFrame>>>,
    logger: FrontendLogger,
    source_faults: Arc<SourceRecovery>,
) {
    run_rtp_scheduler_core(receiver, sink, source_faults).await;
    logger.debug("RTP-MIDI timestamp scheduler stopped");
}

async fn run_rtp_scheduler_core(
    mut receiver: crate::midi_queue::AsyncReceiver<TimedRtpFrame>,
    sink: Arc<arc_swap::ArcSwapOption<Sender<MidiFrame>>>,
    source_faults: Arc<SourceRecovery>,
) {
    let mut cutoffs = HashMap::<Arc<str>, u64>::new();
    let mut pending = BinaryHeap::<TimedRtpFrame>::with_capacity(RTP_PENDING_CAPACITY);
    let mut order = 0u64;
    let mut generation = midi_reset_generation();
    loop {
        let current = midi_reset_generation();
        if current != generation {
            pending.clear();
            cutoffs.clear();
            generation = current;
        }
        let faults = std::mem::take(&mut source_faults.state.lock().pending);
        for (source, cutoff) in faults {
            recover_scheduled_source(source, cutoff, &mut pending, &mut cutoffs, &sink);
        }
        let now = Instant::now();
        for _ in 0..256 {
            if pending.peek().is_none_or(|event| event.deadline > now) {
                break;
            }
            let event = pending.pop().expect("peeked scheduled frame");
            if event.generation != midi_reset_generation() {
                continue;
            }
            dispatch_scheduled_frame(&sink, event.frame);
        }
        let wake = pending
            .peek()
            .map_or(now + Duration::from_millis(10), |event| {
                // An overdue batch must yield back into the drain loop even
                // when the source sends nothing else. The bounded idle wake
                // also processes peer recovery requests independently of MIDI.
                event.deadline.min(now + Duration::from_millis(10))
            });
        tokio::select! {
            event = receiver.recv() => {
                let Some(mut event) = event else { break; };
                if event.generation != midi_reset_generation() || source_frame_is_stale(&event, &cutoffs) { continue; }
                // Observe recovery before admitting this new generation; otherwise
                // the next scheduler turn would clear a freshly admitted event.
                if event.generation != generation { pending.clear(); generation = event.generation; }
                let limit = if event.critical { RTP_PENDING_CAPACITY } else { RTP_PENDING_CAPACITY - crate::realtime::reserve(RTP_PENDING_CAPACITY) };
                if pending.len() >= limit || event.deadline > Instant::now() + RTP_MAX_SCHEDULE_AHEAD {
                    DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
                    if event.critical { request_critical_midi_reset(); }
                    continue;
                }
                if pending.is_empty() && event.deadline <= Instant::now() {
                    dispatch_scheduled_frame(&sink, event.frame);
                } else {
                    event.order = order;
                    order = order.wrapping_add(1);
                    pending.push(event);
                }
                for _ in 1..256 {
                    let Ok(mut event) = receiver.try_recv() else { break; };
                    if event.generation != midi_reset_generation() || source_frame_is_stale(&event, &cutoffs) { continue; }
                    if event.generation != generation { pending.clear(); generation = event.generation; }
                    let limit = if event.critical { RTP_PENDING_CAPACITY } else { RTP_PENDING_CAPACITY - crate::realtime::reserve(RTP_PENDING_CAPACITY) };
                    if pending.len() >= limit || event.deadline > Instant::now() + RTP_MAX_SCHEDULE_AHEAD {
                        DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
                        if event.critical { request_critical_midi_reset(); }
                        continue;
                    }
                    if pending.is_empty() && event.deadline <= Instant::now() {
                        dispatch_scheduled_frame(&sink, event.frame);
                    } else {
                        event.order = order;
                        order = order.wrapping_add(1);
                        pending.push(event);
                    }
                }
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(wake)) => {}
        }
    }
}

fn dispatch_scheduled_frame(sink: &arc_swap::ArcSwapOption<Sender<MidiFrame>>, frame: MidiFrame) {
    #[cfg(feature = "latency-diagnostics")]
    crate::latency_trace::record(
        crate::latency_trace::Stage::RtpDispatched,
        frame.data.as_slice(),
        frame.ingress_qpc,
        crate::vst_worker::protocol::monotonic_qpc(),
    );
    let route = sink.load();
    if let Some(tx) = route.as_ref() {
        if !matches!(
            try_enqueue_midi_frame(tx, frame),
            Ok(crate::bridge::pipeline::EnqueueOutcome::Enqueued
                | crate::bridge::pipeline::EnqueueOutcome::Filtered)
        ) {
            DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn should_sample_rtp_log(slot: &AtomicU64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let last = slot.load(Ordering::Relaxed);
    now.saturating_sub(last) >= 1_000
        && slot
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

fn emit_participants(logger: &FrontendLogger, participants: &Arc<Mutex<Vec<RtpParticipantInfo>>>) {
    let snapshot = participants.lock().clone();
    let _ = logger.app_handle().emit(RTP_PARTICIPANTS_EVENT, snapshot);
}

impl crate::midi_queue::Priority for TimedRtpFrame {
    fn critical(&self) -> bool {
        self.critical
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtp_midi_and_loss_logs_have_independent_rate_limiters() {
        let midi_slot = AtomicU64::new(0);
        let loss_slot = AtomicU64::new(0);

        assert!(should_sample_rtp_log(&midi_slot));
        assert!(!should_sample_rtp_log(&midi_slot));
        assert!(should_sample_rtp_log(&loss_slot));
        assert!(!should_sample_rtp_log(&loss_slot));
    }

    #[test]
    fn scheduler_drains_more_than_one_due_batch_without_another_packet() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (tx, receiver) = crate::midi_queue::async_channel(1024);
                let (output, notes) = crate::midi_queue::bounded(1024);
                let sink = Arc::new(arc_swap::ArcSwapOption::from(Some(Arc::new(output))));
                let scheduler = tokio::spawn(run_rtp_scheduler_core(
                    receiver,
                    sink,
                    Arc::new(SourceRecovery::default()),
                ));
                // Queue several batches for a single future deadline, then keep the
                // sender alive but silent, as when a chord is followed by no new input.
                let deadline = Instant::now() + Duration::from_millis(50);
                for _ in 0..600 {
                    tx.try_send(TimedRtpFrame {
                        source_epoch: 0,
                        critical: false,
                        frame: MidiFrame::copy_realtime(&[0x90, 60, 100], Arc::from("RTP:test"))
                            .unwrap(),
                        deadline,
                        generation: midi_reset_generation(),
                        order: 0,
                    })
                    .unwrap_or_else(|_| panic!("test queue full"));
                }
                let received = tokio::task::spawn_blocking(move || {
                    let deadline = Instant::now() + Duration::from_millis(500);
                    let mut count = 0;
                    while count < 600 {
                        if notes
                            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                            .is_err()
                        {
                            break;
                        }
                        count += 1;
                    }
                    count
                })
                .await
                .unwrap();
                scheduler.abort();
                let _ = scheduler.await;
                drop(tx);
                assert_eq!(
                    received, 600,
                    "overdue MIDI must drain without waiting for new input"
                );
            });
    }

    #[test]
    fn scheduler_delivers_peer_recovery_while_input_is_idle() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (tx, receiver) = crate::midi_queue::async_channel(16);
                let (output, notes) = crate::midi_queue::bounded(16);
                let sink = Arc::new(arc_swap::ArcSwapOption::from(Some(Arc::new(output))));
                let recovery = Arc::new(SourceRecovery::default());
                let scheduler =
                    tokio::spawn(run_rtp_scheduler_core(receiver, sink, recovery.clone()));
                // First allow the scheduler to enter its idle wait.
                tokio::time::sleep(Duration::from_millis(20)).await;
                recovery.request(Arc::from("RTP:test"));
                let reset = tokio::task::spawn_blocking(move || {
                    notes.recv_timeout(Duration::from_millis(200))
                })
                .await
                .unwrap();
                scheduler.abort();
                let _ = scheduler.await;
                drop(tx);
                let reset = reset.expect("idle peer fault must release its held notes");
                assert_eq!(reset.source.as_ref(), "RTP:test");
                assert_eq!(reset.data.as_slice(), [0xFF]);
            });
    }

    #[test]
    fn peer_fault_recovery_discards_only_pre_fault_work_and_preserves_global_epoch() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let generation = midi_reset_generation();
        let peers = RwLock::new(HashMap::from([
            (1, Arc::from("RTP:A")),
            (2, Arc::from("RTP:B")),
        ]));
        let requests = SourceRecovery::default();
        request_peer_recovery(1, &peers, &Arc::from("RTP:Session"), &requests);
        assert_eq!(requests.state.lock().pending.len(), 1);
        assert!(requests.state.lock().pending.contains_key("RTP:A"));
        let (tx, rx) = crate::midi_queue::bounded(16);
        let sink = arc_swap::ArcSwapOption::from(Some(Arc::new(tx)));
        let frame = |source: &str, qpc| MidiFrame {
            generation,
            ingress_qpc: qpc,
            data: smallvec::smallvec![0x90, 60, 100],
            source: Arc::from(source),
        };
        let mut pending = BinaryHeap::new();
        for (order, (source, qpc)) in [("RTP:A", 10), ("RTP:A", 30), ("RTP:B", 10)]
            .into_iter()
            .enumerate()
        {
            pending.push(TimedRtpFrame {
                source_epoch: qpc,
                critical: false,
                frame: frame(source, qpc),
                deadline: Instant::now(),
                generation,
                order: order as u64,
            });
        }
        let mut cutoffs = HashMap::new();
        recover_scheduled_source(Arc::from("RTP:A"), 20, &mut pending, &mut cutoffs, &sink);
        assert_eq!(pending.len(), 2);
        let reset = rx.try_recv().unwrap();
        assert_eq!(reset.source.as_ref(), "RTP:A");
        assert_eq!(reset.data.as_slice(), [0xFF]);
        let event = |source, epoch| TimedRtpFrame {
            source_epoch: epoch,
            frame: frame(source, 1),
            critical: false,
            deadline: Instant::now(),
            generation,
            order: 0,
        };
        assert!(source_frame_is_stale(&event("RTP:A", 10), &cutoffs));
        assert!(!source_frame_is_stale(&event("RTP:A", 30), &cutoffs));
        assert!(!source_frame_is_stale(&event("RTP:B", 10), &cutoffs));
        assert_eq!(midi_reset_generation(), generation);
    }

    #[test]
    fn scheduler_routes_during_concurrent_publication_without_contention_drops() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let (first_tx, first_rx) = crate::midi_queue::bounded(1000);
        let (second_tx, second_rx) = crate::midi_queue::bounded(1000);
        let first_tx = Arc::new(first_tx);
        let second_tx = Arc::new(second_tx);
        let sink = Arc::new(arc_swap::ArcSwapOption::from(Some(first_tx.clone())));
        let writer_sink = sink.clone();
        let writer = std::thread::spawn(move || {
            for _ in 0..1000 {
                writer_sink.store(Some(second_tx.clone()));
                writer_sink.store(Some(first_tx.clone()));
            }
        });
        let generation = midi_reset_generation();
        for _ in 0..1000 {
            dispatch_scheduled_frame(
                &sink,
                MidiFrame {
                    generation: MidiFrame::current_generation(),
                    ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    data: smallvec::smallvec![0x80, 60, 0],
                    source: Arc::from("RTP:test"),
                },
            );
        }
        writer.join().unwrap();
        assert_eq!(first_rx.len() + second_rx.len(), 1000);
        assert_eq!(midi_reset_generation(), generation);
    }

    #[test]
    fn scheduler_preserves_equal_deadline_arrival_order() {
        let deadline = Instant::now();
        let mut heap = BinaryHeap::with_capacity(3);
        for order in [2, 0, 1] {
            heap.push(TimedRtpFrame {
                source_epoch: 0,
                critical: false,
                frame: MidiFrame {
                    generation: MidiFrame::current_generation(),
                    ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    data: smallvec::smallvec![0x90, 60, 1],
                    source: Arc::from("test"),
                },
                deadline,
                generation: 0,
                order,
            });
        }
        for order in 0..3 {
            assert_eq!(heap.pop().unwrap().order, order);
        }
    }

    #[test]
    fn missing_peer_source_completes_and_reuses_cached_identity() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let peers = RwLock::new(HashMap::new());
            let first = cached_peer_source(&peers, "Session", 42);
            let second = cached_peer_source(&peers, "Session", 42);
            tx.send((first, second)).unwrap();
        });
        let (first, second) = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("peer source lookup must not deadlock on a cache miss");
        assert_eq!(&*first, "RTP:Session#ssrc_0000002a");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn peer_source_format_is_consistent_and_stable() {
        let src1 = peer_source("TestSession", 0x1234abcd);
        assert_eq!(&*src1, "RTP:TestSession#ssrc_1234abcd");
        let src2 = peer_source("OSCMidi", 1);
        assert_eq!(&*src2, "RTP:OSCMidi#ssrc_00000001");
    }
}
