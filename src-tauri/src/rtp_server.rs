use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::async_runtime::{self, JoinHandle};
use tauri::Emitter;
use tokio::sync::{oneshot, Notify};
use uuid::Uuid;

use rtpmidi::sessions::{
    events::event_handling::{MidiMessageEvent, ParticipantJoinedEvent, ParticipantLeftEvent},
    invite_responder::InviteResponder,
    rtp_midi_session::RtpMidiSession,
};

use crate::{
    logger::{logs_enabled, FrontendLogger},
    midi::MidiFrame,
    types::RtpParticipantInfo,
};

use super::rtp_midi::{midi_to_bytes, participant_matches_target};

const RTP_PARTICIPANTS_EVENT: &str = "rtp:participants";

/// Compteur de messages MIDI abandonnés faute de place dans le canal.
/// Incrémenté de façon atomique dans le callback temps-réel, lu depuis
/// le thread de log périodique.
static DROPPED_MIDI_COUNT: AtomicU64 = AtomicU64::new(0);

/// Intervalle entre deux avertissements de drops consécutifs.
const DROP_WARN_INTERVAL: Duration = Duration::from_secs(5);

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
}

impl RtpServer {
    pub fn start(
        name: String,
        port: u16,
        remote_targets: Vec<RtpRemoteTarget>,
        log_rtp: bool,
        sink: Arc<Mutex<Option<Sender<MidiFrame>>>>,
        logger: FrontendLogger,
    ) -> Result<Self, String> {
        let (stop_tx, mut stop_rx) = oneshot::channel();

        if port == u16::MAX {
            return Err("Port RTP invalide: 65535 reserve".to_string());
        }

        let (session, bound_port) = async_runtime::block_on(async {
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
                match RtpMidiSession::start(p, &name, ssrc, InviteResponder::Accept).await {
                    Ok(session) => return Ok((session, p)),
                    Err(err) => {
                        if err.kind() == std::io::ErrorKind::AddrInUse {
                            if let (Some(next_ctrl), Some(next_data)) =
                                (p.checked_add(2), p.checked_add(3))
                            {
                                logger.warn(format!(
                                    "Port RTP {p} occupé, tentative sur {next_ctrl}/{next_data}"
                                ));
                            } else {
                                logger.warn(format!("Port RTP {p} occupé, autre tentative"));
                            }
                            continue;
                        } else {
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

        // Timestamp du dernier avertissement de drop — partagé via Arc<Mutex>
        // car add_listener exige une closure Fn (pas FnMut).
        // Initialisé à (maintenant - intervalle) pour que le 1er drop soit
        // toujours loggué immédiatement.
        let last_drop_warn: Arc<Mutex<Instant>> =
            Arc::new(Mutex::new(Instant::now() - DROP_WARN_INTERVAL));

        let handle = async_runtime::spawn(async move {
            // ── Listener MIDI ────────────────────────────────────────────────
            // HOT PATH : ce callback est appelé sur chaque message reçu.
            // Règles :
            //   • Pas de verrou long-durée.
            //   • Pas d'allocation évitable.
            //   • Jamais bloquant : on abandonne le message plutôt que
            //     de bloquer le thread de la lib rtpmidi.
            let rtp_logger = logger.clone();
            let last_drop_warn_cb = Arc::clone(&last_drop_warn);
            session
                .add_listener(MidiMessageEvent, move |(message, _delta)| {
                    let bytes = midi_to_bytes(message);

                    // Logging RTP optionnel : format + IPC frontend sont coûteux,
                    // activer uniquement pour débogage ciblé.
                    if log_rtp && logs_enabled() {
                        rtp_logger.info(format!("RTP MIDI: {:02X?}", bytes.as_slice()));
                    }

                    // Acquisition du verrou uniquement pour cloner le Sender
                    // (opération très rapide, pas d'IO).
                    // TODO perf : remplacer Arc<Mutex<Option<Sender>>> par
                    // arc_swap::ArcSwap<Option<Sender>> pour un accès lock-free.
                    let Some(tx) = midi_sink.lock().as_ref().cloned() else {
                        return;
                    };

                    // FIX CRITIQUE : l'ancien `try_send` avec `let _ =` abandonnait
                    // silencieusement les messages quand le canal était plein.
                    // On tente d'abord try_send (non bloquant) ; en cas de saturation
                    // on logue périodiquement pour diagnostiquer sans inonder les logs.
                    match tx.try_send(MidiFrame {
                        data: bytes.to_vec(),
                        source: rtp_source.to_string(),
                    }) {
                        Ok(()) => {}
                        Err(crossbeam_channel::TrySendError::Full(_)) => {
                            let dropped = DROPPED_MIDI_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                            let now = Instant::now();
                            // Intériorité mutable via Mutex : add_listener exige Fn,
                            // on ne peut pas capturer une variable mut directement.
                            let mut guard = last_drop_warn_cb.lock();
                            if now.duration_since(*guard) >= DROP_WARN_INTERVAL {
                                *guard = now;
                                // On libère le verrou avant le log (potentiellement lent).
                                drop(guard);
                                rtp_logger.warn(format!(
                                    "RTP-MIDI : canal MIDI saturé — {dropped} message(s) \
                                     abandonnés. Envisager d'augmenter la capacité du canal \
                                     ou de réduire la charge du consommateur."
                                ));
                            }
                        }
                        // Le canal est fermé = serveur en cours d'arrêt, comportement normal.
                        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
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
                            next_wake
                                .map_or(state.next_attempt, |c| c.min(state.next_attempt)),
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
        });

        Ok(Self {
            stop_tx: Some(stop_tx),
            handle,
            bound_port,
            participants,
        })
    }

    pub async fn stop(self) {
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
}

fn emit_participants(logger: &FrontendLogger, participants: &Arc<Mutex<Vec<RtpParticipantInfo>>>) {
    let snapshot = participants.lock().clone();
    let _ = logger.app_handle().emit(RTP_PARTICIPANTS_EVENT, snapshot);
}
