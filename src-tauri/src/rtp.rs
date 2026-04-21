use crossbeam_channel::Sender;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use midi_types::{status, MidiMessage as RtMidiMessage};
use parking_lot::Mutex;
use rtpmidi::sessions::{
    events::event_handling::{MidiMessageEvent, ParticipantJoinedEvent, ParticipantLeftEvent},
    invite_responder::InviteResponder,
    rtp_midi_session::RtpMidiSession,
};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::async_runtime::{self, JoinHandle};
use tauri::Emitter;
use tokio::sync::{oneshot, Notify};
use tokio::time::interval;
use uuid::Uuid;

use crate::{
    logger::{background_log, logs_enabled, FrontendLogger},
    midi::MidiFrame,
    types::{RtpParticipantInfo, RtpSessionInfo},
};

const RTP_PARTICIPANTS_EVENT: &str = "rtp_participants";
const RTP_SESSIONS_EVENT: &str = "rtp_sessions";

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

        // Démarre la session synchronement pour connaître le port réellement lié (utile
        // pour éviter de s'auto-désactiver lors du test des ports).
        let (session, bound_port) = async_runtime::block_on(async {
            let ssrc = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
            let try_ports = [0u16, 2, 4]
                .into_iter()
                .filter_map(|offset| port.checked_add(offset))
                .filter(|candidate| *candidate < u16::MAX)
                .collect::<SmallVec<[u16; 3]>>();
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

        let name_clone = name.clone();
        let rtp_source: Arc<str> = Arc::from(format!("RTP:{name_clone}"));
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
            let rtp_logger = logger.clone();
            session
                .add_listener(MidiMessageEvent, move |(message, _delta)| {
                    // Forward every RTP-MIDI byte sequence as-is; OSC filtering happens later.
                    let bytes = midi_to_bytes(message);
                    if log_rtp && logs_enabled() {
                        rtp_logger.info(format!("RTP MIDI: {:02X?}", bytes.as_slice()));
                    }
                    if let Some(tx) = midi_sink.lock().as_ref().cloned() {
                        let _ = tx.try_send(MidiFrame { data: bytes.to_vec(), source: rtp_source.to_string() });
                    }
                })
                .await;

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

            if !remote_targets.is_empty() {
                let mut remote_states = remote_targets
                    .into_iter()
                    .map(|target| {
                        RemoteTargetState {
                            target,
                            backoff: Duration::from_secs(1),
                            next_attempt: Instant::now(),
                        }
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
                            state.backoff = Duration::from_secs(1);
                            state.next_attempt = now + state.backoff;
                            continue;
                        }
                        if now >= state.next_attempt {
                            session.invite_participant(state.target.addr).await;
                            state.backoff = (state.backoff * 2).min(Duration::from_secs(30));
                            state.next_attempt = now + state.backoff;
                        }
                        next_wake = match next_wake {
                            Some(current) if current <= state.next_attempt => Some(current),
                            _ => Some(state.next_attempt),
                        };
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

pub fn ports_available(port: u16) -> Result<bool, String> {
    if port == u16::MAX {
        return Ok(false);
    }
    let ctrl_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    let data_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port.saturating_add(1));
    let ctrl = match UdpSocket::bind(ctrl_addr) {
        Ok(sock) => sock,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::AddrInUse {
                return Ok(false);
            } else {
                return Err(err.to_string());
            }
        }
    };
    let data = match UdpSocket::bind(data_addr) {
        Ok(sock) => sock,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::AddrInUse {
                return Ok(false);
            } else {
                return Err(err.to_string());
            }
        }
    };
    drop(ctrl);
    drop(data);
    Ok(true)
}

pub fn discover_sessions(timeout: Duration) -> Result<Vec<RtpSessionInfo>, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receiver = mdns
        .browse("_apple-midi._udp.local.")
        .map_err(|e| e.to_string())?;

    let deadline = Instant::now() + timeout;
    let mut sessions: HashMap<String, RtpSessionInfo> = HashMap::new();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(200));
        let event = match receiver.recv_timeout(wait) {
            Ok(event) => event,
            Err(_) => continue,
        };

        if let ServiceEvent::ServiceResolved(info) = event {
            if info.get_addresses().is_empty() {
                continue;
            }

            let fullname = info.get_fullname();
            let name = fullname
                .strip_suffix("._apple-midi._udp.local.")
                .unwrap_or(fullname)
                .to_string();
            let host = info.get_hostname().trim_end_matches('.').to_string();
            let mut addresses: Vec<String> = info
                .get_addresses()
                .iter()
                .map(|ip| ip.to_string())
                .collect();
            addresses.sort_by(|a, b| {
                let a_v6 = a.contains(':');
                let b_v6 = b.contains(':');
                (a_v6, a).cmp(&(b_v6, b))
            });

            let key = format!("{}:{}", fullname, info.get_port());
            sessions
                .entry(key)
                .and_modify(|existing| {
                    for addr in &addresses {
                        if !existing.addresses.contains(addr) {
                            existing.addresses.push(addr.clone());
                        }
                    }
                })
                .or_insert_with(|| RtpSessionInfo {
                    name: if name.is_empty() { host.clone() } else { name },
                    host,
                    port: info.get_port(),
                    addresses,
                });
        }
    }

    let _ = mdns.shutdown();

    let mut list: Vec<RtpSessionInfo> = sessions.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

fn emit_participants(logger: &FrontendLogger, participants: &Arc<Mutex<Vec<RtpParticipantInfo>>>) {
    let snapshot = participants.lock().clone();
    let _ = logger.app_handle().emit(RTP_PARTICIPANTS_EVENT, snapshot);
}

fn emit_sessions(app_handle: &tauri::AppHandle, sessions: &[RtpSessionInfo]) {
    let _ = app_handle.emit(RTP_SESSIONS_EVENT, sessions);
}

const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(500);
const DISCOVERY_TTL: Duration = Duration::from_secs(30);  // cache 30s
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(10); // scan toutes les 10s
const DISCOVERY_INTERVAL_IDLE: Duration = Duration::from_secs(30); // si aucun participant

#[derive(Default)]
#[derive(Debug)]
struct RtpDiscoveryCache {
    sessions: Vec<RtpSessionInfo>,
    updated_at: Option<Instant>,
}

impl RtpDiscoveryCache {
    fn is_stale(&self) -> bool {
        self.updated_at
            .map(|ts| ts.elapsed() >= DISCOVERY_TTL)
            .unwrap_or(true)
    }

    fn update(&mut self, sessions: Vec<RtpSessionInfo>) -> bool {
        let changed = self.sessions != sessions;
        self.sessions = sessions;
        self.updated_at = Some(Instant::now());
        changed
    }
}

#[derive(Debug)]
pub struct RtpDiscoveryManager {
    cache: Arc<Mutex<RtpDiscoveryCache>>,
    task: Mutex<Option<JoinHandle<()>>>,
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl Clone for RtpDiscoveryManager {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            task: Mutex::new(None),
            stop_tx: Mutex::new(None),
        }
    }
}

impl RtpDiscoveryManager {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(RtpDiscoveryCache::default())),
            task: Mutex::new(None),
            stop_tx: Mutex::new(None),
        }
    }

    pub fn cached(&self) -> Vec<RtpSessionInfo> {
        self.cache.lock().sessions.clone()
    }

    pub fn start(&self, app_handle: tauri::AppHandle) {
        let mut task_guard = self.task.lock();
        if task_guard.is_some() {
            return;
        }
        let (stop_tx, mut stop_rx) = oneshot::channel();
        *self.stop_tx.lock() = Some(stop_tx);
        let cache = self.cache.clone();
        // Dans RtpDiscoveryManager::start() — boucle améliorée ✅
        *task_guard = Some(async_runtime::spawn(async move {
            let initial = cache.lock().sessions.clone();
            emit_sessions(&app_handle, &initial);
            let mut ticker = interval(DISCOVERY_INTERVAL);
            let mut idle_streak = 0u32;
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = ticker.tick() => {
                        let should_refresh = { cache.lock().is_stale() };
                        if !should_refresh { continue; }

                        let handle = async_runtime::spawn_blocking(|| discover_sessions(DISCOVERY_TIMEOUT));
                        match handle.await {
                            Ok(Ok(sessions)) => {
                                let changed = { cache.lock().update(sessions.clone()) };
                                if changed {
                                 idle_streak = 0;
                                    emit_sessions(&app_handle, &sessions);
                                } else {
                                    idle_streak += 1;
                                }
                                // Ralentir si rien ne change
                                let next = if idle_streak > 3 {
                                    DISCOVERY_INTERVAL_IDLE
                                } else {
                                    DISCOVERY_INTERVAL
                                };
                                ticker = interval(next);
                                ticker.tick().await; // consume immediate first tick
                            }
                            Ok(Err(err)) => background_log("warn", format!("RTP discovery: {err}")),
                            Err(err) => background_log("warn", format!("RTP discovery task: {err}")),
                        }
                    }
                }
            }
        }));
    }

    pub fn stop(&self) {
        if let Some(stop_tx) = self.stop_tx.lock().take() {
            let _ = stop_tx.send(());
        }
        if let Some(handle) = self.task.lock().take() {
            let _ = async_runtime::block_on(handle);
        }
    }

    pub fn refresh_now(
        &self,
        app_handle: &tauri::AppHandle,
        force: bool,
    ) -> Result<Vec<RtpSessionInfo>, String> {
        if !force {
            let cache = self.cache.lock();
            if !cache.is_stale() {
                let sessions = cache.sessions.clone();
                drop(cache);
                emit_sessions(app_handle, &sessions);
                return Ok(sessions);
            }
        }

        let handle = async_runtime::spawn_blocking(|| discover_sessions(DISCOVERY_TIMEOUT));
        match async_runtime::block_on(handle) {
            Ok(Ok(sessions)) => {
                let _changed = self.cache.lock().update(sessions.clone());
                emit_sessions(app_handle, &sessions);
                Ok(sessions)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => Err(err.to_string()),
        }
    }
}

fn midi_to_bytes(message: RtMidiMessage) -> SmallVec<[u8; 32]> {
    use RtMidiMessage::*;
    match message {
        NoteOn(channel, note, velocity) => SmallVec::from_slice(&[
            status::NOTE_ON | u8::from(channel),
            u8::from(note),
            u8::from(velocity),
        ]),
        NoteOff(channel, note, velocity) => SmallVec::from_slice(&[
            status::NOTE_OFF | u8::from(channel),
            u8::from(note),
            u8::from(velocity),
        ]),
        KeyPressure(channel, note, pressure) => SmallVec::from_slice(&[
            status::KEY_PRESSURE | u8::from(channel),
            u8::from(note),
            u8::from(pressure),
        ]),
        ControlChange(channel, control, value) => SmallVec::from_slice(&[
            status::CONTROL_CHANGE | u8::from(channel),
            u8::from(control),
            u8::from(value),
        ]),
        ProgramChange(channel, program) => SmallVec::from_slice(&[
            status::PROGRAM_CHANGE | u8::from(channel),
            u8::from(program),
        ]),
        ChannelPressure(channel, pressure) => SmallVec::from_slice(&[
            status::CHANNEL_PRESSURE | u8::from(channel),
            u8::from(pressure),
        ]),
        PitchBendChange(channel, bend) => {
            // `rtpmidi` parses Value14 with the incoming byte order (LSB, MSB),
            // so we replay the raw order to avoid inverting the pitch wheel.
            let (b1, b2): (u8, u8) = bend.into();
            SmallVec::from_slice(&[status::PITCH_BEND_CHANGE | u8::from(channel), b1, b2])
        }
        QuarterFrame(frame) => SmallVec::from_slice(&[status::QUARTER_FRAME, frame.into()]),
        SongPositionPointer(pos) => {
            let (b1, b2): (u8, u8) = pos.into();
            SmallVec::from_slice(&[status::SONG_POSITION_POINTER, b1, b2])
        }
        SongSelect(song) => SmallVec::from_slice(&[status::SONG_SELECT, song.into()]),
        TuneRequest => SmallVec::from_slice(&[status::TUNE_REQUEST]),
        TimingClock => SmallVec::from_slice(&[status::TIMING_CLOCK]),
        Start => SmallVec::from_slice(&[status::START]),
        Continue => SmallVec::from_slice(&[status::CONTINUE]),
        Stop => SmallVec::from_slice(&[status::STOP]),
        ActiveSensing => SmallVec::from_slice(&[status::ACTIVE_SENSING]),
        Reset => SmallVec::from_slice(&[status::RESET]),
    }
}

fn participant_matches_target(participant_addr: &str, target: &SocketAddr) -> bool {
    if participant_addr == target.to_string() {
        return true;
    }
    let Ok(participant) = participant_addr.parse::<SocketAddr>() else {
        return false;
    };
    if participant.ip() != target.ip() {
        return false;
    }
    let tp = target.port();
    let pp = participant.port();
    pp == tp
        || (tp.checked_add(1) == Some(pp))
        || (tp.checked_sub(1) == Some(pp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use midi_types::{Channel, Control, Note, Program, Value14, Value7};
    use std::net::{Ipv4Addr, UdpSocket};

    #[test]
    fn converts_channel_messages_without_filtering() {
        let note = RtMidiMessage::NoteOn(Channel::C1, Note::new(60), Value7::new(100));
        assert_eq!(
            midi_to_bytes(note).as_slice(),
            [status::NOTE_ON | u8::from(Channel::C1), 60, 100]
        );

        let cc = RtMidiMessage::ControlChange(Channel::C10, Control::new(74), Value7::new(42));
        assert_eq!(
            midi_to_bytes(cc).as_slice(),
            [status::CONTROL_CHANGE | u8::from(Channel::C10), 74, 42]
        );

        let prog = RtMidiMessage::ProgramChange(Channel::C5, Program::new(10));
        assert_eq!(
            midi_to_bytes(prog).as_slice(),
            [status::PROGRAM_CHANGE | u8::from(Channel::C5), 10]
        );
    }

    #[test]
    fn keeps_raw_order_for_14bit_and_system_messages() {
        // Incoming RTP gives bytes as (LSB, MSB); keep the raw order so pitch down/up stays correct.
        let bend = RtMidiMessage::PitchBendChange(Channel::C2, Value14::from((2, 1)));
        assert_eq!(
            midi_to_bytes(bend).as_slice(),
            [status::PITCH_BEND_CHANGE | u8::from(Channel::C2), 2, 1]
        );

        let spp = RtMidiMessage::SongPositionPointer(Value14::from((4, 3)));
        assert_eq!(
            midi_to_bytes(spp).as_slice(),
            [status::SONG_POSITION_POINTER, 4, 3]
        );

        assert_eq!(
            midi_to_bytes(RtMidiMessage::Start).as_slice(),
            [status::START]
        );
        assert_eq!(
            midi_to_bytes(RtMidiMessage::Stop).as_slice(),
            [status::STOP]
        );
    }

    #[test]
    fn detects_busy_rtp_ports() {
        let mut base = 52000u16;
        let (s1, s2) = loop {
            if let Ok(sock1) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, base)) {
                if let Ok(sock2) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, base + 1)) {
                    break (sock1, sock2);
                }
            }
            base = base.saturating_add(2);
            assert!(base < 65000, "Could not find free port pair for test");
        };

        assert!(!ports_available(base).unwrap());
        drop(s1);
        drop(s2);
        assert!(ports_available(base).unwrap());
    }

    #[test]
    fn participant_match_accepts_control_and_data_ports_for_same_host() {
        let target: SocketAddr = "192.168.1.50:5004".parse().expect("target parse");
        assert!(participant_matches_target("192.168.1.50:5004", &target));
        assert!(participant_matches_target("192.168.1.50:5005", &target));
        assert!(!participant_matches_target("192.168.1.50:5006", &target));
        assert!(!participant_matches_target("192.168.1.51:5004", &target));
    }

    #[test]
    fn participant_match_rejects_invalid_socket_addr_text() {
        let target: SocketAddr = "127.0.0.1:5004".parse().expect("target parse");
        assert!(!participant_matches_target("not-an-addr", &target));
    }
}
