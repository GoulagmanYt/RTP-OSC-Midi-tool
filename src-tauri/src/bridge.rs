use crate::{
    audio::AudioEngine,
    config::{
        Config, RoutingAssignment, RoutingMapping, RoutingProfile, RTP_VIRTUAL_INPUT,
        VST_INTERNAL_OUTPUT, VST_INTERNAL_OUTPUT_LEGACY,
    },
    logger::{logs_enabled, should_log_debug, FrontendLogger},
    midi::{parse_note, parse_sustain, MidiFrame, MidiKind},
    osc::OscClient,
    rtp::{RtpRemoteTarget, RtpServer},
    types::{BridgeMetrics, BridgeStatus, MidiActivityInfo, MidiNoteEvent, RtpParticipantInfo},
};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Emitter;
use tauri::Window;

pub struct BridgeHandle {
    inner: Arc<Mutex<Option<BridgeRuntime>>>,
    rtp_server: Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: Arc<Mutex<Option<Sender<MidiFrame>>>>,
    rtp_config: Arc<Mutex<Option<RtpConfigSnapshot>>>,
}

const MIDI_ACTIVITY_EVENT: &str = "midi_activity";
const MIDI_NOTE_EVENT: &str = "midi_note";
const BRIDGE_METRICS_EVENT: &str = "bridge_metrics";
const OSC_SUSTAIN_PARAM: &str = "/avatar/parameters/sustain";

#[derive(Clone)]
struct RtpConfigSnapshot {
    name: String,
    requested_port: u16,
    bound_port: u16,
    remote_enabled: bool,
    remote_targets: Vec<RtpRemoteTarget>,
    log_rtp: bool,
}

struct BridgeRuntime {
    status: BridgeStatus,
    stop: Arc<AtomicBool>,
    processing: Option<thread::JoinHandle<()>>,
    midi_watcher: Option<thread::JoinHandle<()>>,
    activity_emitter: Option<thread::JoinHandle<()>>,
    config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    actual_midi_in: Arc<Mutex<Option<String>>>,
    actual_midi_out: Arc<Mutex<Option<String>>>,
}

#[derive(Clone)]
struct RoutingProfileRuntime {
    id: String,
    channel_filter: Option<u8>,
    note_min: Option<u8>,
    note_max: Option<u8>,
    cc_map: [u8; 128],
    program_map: [u8; 128],
    enabled: bool,
}

impl RoutingProfileRuntime {
    fn from_profile(profile: &RoutingProfile) -> Self {
        let mut cc_map = [0u8; 128];
        let mut program_map = [0u8; 128];
        for i in 0..128u8 {
            cc_map[i as usize] = i;
            program_map[i as usize] = i;
        }
        for RoutingMapping { from, to } in &profile.cc_map {
            if *from < 128 && *to < 128 {
                cc_map[*from as usize] = *to;
            }
        }
        for RoutingMapping { from, to } in &profile.program_map {
            if *from < 128 && *to < 128 {
                program_map[*from as usize] = *to;
            }
        }
        Self {
            id: profile.id.clone(),
            channel_filter: profile.channel_filter,
            note_min: profile.note_min,
            note_max: profile.note_max,
            cc_map,
            program_map,
            enabled: profile.enabled,
        }
    }
}

#[derive(Clone, Default)]
struct MidiActivityState {
    messages: u32,
    last_note: Option<u8>,
    last_channel: Option<u8>,
    last_seen_ms: Option<u64>,
}

#[derive(Default)]
struct MidiActivityTracker {
    stats: HashMap<Arc<str>, MidiActivityState>,
}

impl MidiActivityTracker {
    fn record(&mut self, source: &Arc<str>, data: &[u8]) {
        let entry = self.stats.entry(source.clone()).or_default();
        entry.messages = entry.messages.saturating_add(1);
        entry.last_seen_ms = Some(now_ms());
        if let Some(note) = parse_note(data) {
            entry.last_note = Some(note.note);
            entry.last_channel = Some(note.channel);
        }
    }

    fn snapshot_and_reset(&mut self) -> Vec<MidiActivityInfo> {
        let now = now_ms();
        let mut snapshot = Vec::with_capacity(self.stats.len());
        self.stats.retain(|source, state| {
            snapshot.push(MidiActivityInfo {
                source: source.to_string(),
                messages_per_sec: state.messages,
                last_note: state.last_note,
                last_channel: state.last_channel,
                last_seen_ms: state.last_seen_ms,
            });
            state.messages = 0;
            if let Some(last) = state.last_seen_ms {
                now.saturating_sub(last) < 300_000
            } else {
                true
            }
        });
        snapshot
    }
}

impl BridgeHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            rtp_server: Arc::new(Mutex::new(None)),
            rtp_sink: Arc::new(Mutex::new(None)),
            rtp_config: Arc::new(Mutex::new(None)),
        }
    }

    fn rtp_requested(config: &Config) -> bool {
        config.rtp_enabled
            || config.rtp_remote_enabled
            || config
                .midi_in
                .as_ref()
                .map(|s| s == RTP_VIRTUAL_INPUT)
                .unwrap_or(false)
    }

    pub fn sync_rtp(
        &self,
        config: &Config,
        logger: &FrontendLogger,
        force_restart: bool,
    ) -> Result<(), String> {
        let should_run = Self::rtp_requested(config);
        let mut server_guard = self.rtp_server.lock();
        let mut cfg_guard = self.rtp_config.lock();

        if should_run {
            let remote_targets = resolve_remote_targets(config, logger);
            let needs_restart = force_restart
                || server_guard.is_none()
                || cfg_guard
                    .as_ref()
                    .map(|cfg| {
                        cfg.name != config.rtp_session_name
                            || cfg.requested_port != config.rtp_port
                            || cfg.remote_enabled != config.rtp_remote_enabled
                            || cfg.remote_targets != remote_targets
                            || cfg.log_rtp != config.log_rtp
                    })
                    .unwrap_or(true);

            if needs_restart {
                if let Some(existing) = server_guard.take() {
                    tauri::async_runtime::block_on(existing.stop());
                }
                let server = RtpServer::start(
                    config.rtp_session_name.clone(),
                    config.rtp_port,
                    remote_targets.clone(),
                    config.log_rtp,
                    self.rtp_sink.clone(),
                    logger.clone(),
                )?;
                let bound_port = server.bound_port();
                *cfg_guard = Some(RtpConfigSnapshot {
                    name: config.rtp_session_name.clone(),
                    requested_port: config.rtp_port,
                    bound_port,
                    remote_enabled: config.rtp_remote_enabled,
                    remote_targets,
                    log_rtp: config.log_rtp,
                });
                *server_guard = Some(server);
            }
        } else {
            if let Some(existing) = server_guard.take() {
                tauri::async_runtime::block_on(existing.stop());
            }
            *cfg_guard = None;
            let _ = logger
                .app_handle()
                .emit("rtp_participants", Vec::<RtpParticipantInfo>::new());
        }

        Ok(())
    }

    pub fn restart_rtp(&self, config: &Config, logger: &FrontendLogger) -> Result<(), String> {
        self.sync_rtp(config, logger, true)
    }

    pub fn start(
        &self,
        window: Window,
        config: Config,
        dev_logging: Arc<AtomicBool>,
        audio: AudioEngine,
    ) -> Result<BridgeStatus, String> {
        self.stop_internal();

        let logger = FrontendLogger::new(window.clone(), dev_logging);
        let osc = OscClient::new(&config.osc_target_ip, config.osc_target_port)?;
        let rtp_requested = Self::rtp_requested(&config);
        let mut status = BridgeStatus {
            running: true,
            midi_in: config.midi_in.clone(),
            midi_out: config.midi_out.clone(),
            osc_target: format!("{}:{}", config.osc_target_ip, config.osc_target_port),
            rtp_active: rtp_requested,
            rtp_bound_port: None,
            last_error: None,
            vst_loaded: audio.is_running(),
            audio_running: audio.is_running(),
            audio_latency_ms: audio.current_latency_ms(),
            audio_backend: audio.current_backend(),
            audio_device: audio.current_device(),
            audio_sample_rate: audio.current_sample_rate(),
            audio_buffer_size: audio.current_buffer_size(),
            audio_requested_buffer_size: audio.requested_buffer_size(),
            audio_stream_buffer_size: audio.stream_buffer_size(),
            audio_buffer_mismatch: audio.buffer_size_mismatch(),
            vst_midi_compatible: audio.vst_midi_compatible(),
            audio_xruns: audio.xrun_count(),
            audio_limiter_enabled: audio.limiter_enabled(),
        };

        let stop = Arc::new(AtomicBool::new(false));
        use crossbeam_channel::bounded;
        let (midi_tx, midi_rx) = bounded::<MidiFrame>(2048);let shared_config = Arc::new(Mutex::new(config.clone()));
        let config_rev = Arc::new(AtomicU64::new(1));
        let activity_tracker = Arc::new(Mutex::new(MidiActivityTracker::default()));
        let osc_counter = Arc::new(AtomicU32::new(0));
        let metrics_audio = audio.clone();
        let actual_midi_in = Arc::new(Mutex::new(initial_connected_input(&config)));
        let actual_midi_out = Arc::new(Mutex::new(initial_connected_output(&config)));

        status.midi_in = actual_midi_in.lock().clone();
        status.midi_out = actual_midi_out.lock().clone();

        *self.rtp_sink.lock() = Some(midi_tx.clone());
        if let Err(err) = self.sync_rtp(&config, &logger, false) {
            *self.rtp_sink.lock() = None;
            return Err(err);
        }

        status.rtp_active = self.rtp_server.lock().is_some();
        status.rtp_bound_port = self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port);

        let midi_stop = stop.clone();
        let midi_logger = logger.clone();
        let midi_tx_for_watcher = midi_tx.clone();
        let midi_config = shared_config.clone();
        let midi_rev = config_rev.clone();
        let midi_actual_in = actual_midi_in.clone();
        let midi_watcher = Some(thread::spawn(move || {
            watch_midi_input(
                midi_config,
                midi_rev,
                midi_tx_for_watcher,
                midi_stop,
                midi_logger,
                midi_actual_in,
            );
        }));

        let (midi_out_conn, connected_output) = open_output(&config, &logger);
        *actual_midi_out.lock() = connected_output;

        let processing_stop = stop.clone();
        let processing_logger = logger.clone();
        let processing_config = shared_config.clone();
        let processing_rev = config_rev.clone();
        let processing_activity = activity_tracker.clone();
        let processing_osc_counter = osc_counter.clone();
        let processing_actual_out = actual_midi_out.clone();
        let processing = Some(thread::spawn(move || {
            processing_loop(
                processing_config,
                processing_rev,
                osc,
                midi_rx,
                processing_stop,
                midi_out_conn,
                processing_logger,
                audio,
                processing_activity,
                processing_osc_counter,
                processing_actual_out,
            );
        }));

        let activity_stop = stop.clone();
        let activity_window = window.clone();
        let activity_state = activity_tracker.clone();
        let activity_osc_counter = osc_counter.clone();
        let activity_emitter = Some(thread::spawn(move || {
            while !activity_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(1));
                let snapshot = activity_state.lock().snapshot_and_reset();
                let midi_messages = snapshot.iter().fold(0u32, |acc, entry| {
                    acc.saturating_add(entry.messages_per_sec)
                });
                let _ = activity_window.emit(MIDI_ACTIVITY_EVENT, snapshot);
                let osc_messages = activity_osc_counter.swap(0, Ordering::Relaxed);
                let peaks = metrics_audio.peak_levels();
                let (audio_peak_l, audio_peak_r) = match peaks {
                    Some((left, right)) => (Some(left), Some(right)),
                    None => (None, None),
                };
                let metrics = BridgeMetrics {
                    audio_peak_l,
                    audio_peak_r,
                    audio_latency_ms: metrics_audio.current_latency_ms(),
                    audio_xruns: metrics_audio.xrun_count(),
                    audio_midi_drops: metrics_audio.midi_drop_count(),
                    audio_lock_misses: metrics_audio.audio_lock_miss_count(),
                    audio_emergency_resets: metrics_audio.emergency_reset_count(),
                    midi_messages_per_sec: midi_messages,
                    osc_messages_per_sec: osc_messages,
                };
                let _ = activity_window.emit(BRIDGE_METRICS_EVENT, metrics);
            }
        }));

        let runtime = BridgeRuntime {
            status: status.clone(),
            stop,
            processing,
            midi_watcher,
            activity_emitter,
            config: shared_config,
            config_rev,
            actual_midi_in,
            actual_midi_out,
        };

        *self.inner.lock() = Some(runtime);
        Ok(status)
    }

    pub fn stop(&self) -> Result<(), String> {
        self.stop_internal();
        Ok(())
    }

    pub fn reset_keys(&self) -> Result<(), String> {
        let guard = self.inner.lock();
        if let Some(runtime) = guard.as_ref() {
            let cfg = runtime.config.lock().clone();
            let osc = OscClient::new(&cfg.osc_target_ip, cfg.osc_target_port)?;
            osc.send_reset_all()?;
        }
        Ok(())
    }

    pub fn send_test_note(&self, note: u8, velocity: u8, channel: u8) -> Result<(), String> {
        let ch = channel.clamp(1, 16) - 1;
        let note = note.min(127);
        let velocity = velocity.min(127);
        let source: Arc<str> = Arc::from("Test MIDI");
        let data = SmallVec::from_slice(&[0x90 | ch, note, velocity]);
        self.inject_frame(data, source.clone())?;

        let sink = self.rtp_sink.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            let off = SmallVec::from_slice(&[0x80 | ch, note, 0]);
            if let Some(tx) = sink.lock().as_ref().cloned() {
                let _ = tx.send(MidiFrame { data: off, source });
            }
        });

        Ok(())
    }

    pub fn send_test_cc(&self, cc: u8, value: u8, channel: u8) -> Result<(), String> {
        let ch = channel.clamp(1, 16) - 1;
        let cc = cc.min(127);
        let value = value.min(127);
        let data = SmallVec::from_slice(&[0xB0 | ch, cc, value]);
        self.inject_frame(data, Arc::from("Test MIDI"))
    }

    fn inject_frame(&self, data: SmallVec<[u8; 32]>, source: Arc<str>) -> Result<(), String> {
        if let Some(tx) = self.rtp_sink.lock().as_ref().cloned() {
            tx.send(MidiFrame { data, source })
                .map_err(|_| "Bridge not running".to_string())
        } else {
            Err("Bridge not running".to_string())
        }
    }

    pub fn update_config(&self, config: Config, _logger: &FrontendLogger) -> Result<(), String> {
        let mut guard = self.inner.lock();
        if let Some(runtime) = guard.as_mut() {
            *runtime.config.lock() = config.clone();
            *runtime.actual_midi_in.lock() = initial_connected_input(&config);
            *runtime.actual_midi_out.lock() = initial_connected_output(&config);
            runtime.status.midi_in = runtime.actual_midi_in.lock().clone();
            runtime.status.midi_out = runtime.actual_midi_out.lock().clone();
            runtime.status.osc_target =
                format!("{}:{}", config.osc_target_ip, config.osc_target_port);
            runtime.status.rtp_active = self.rtp_server.lock().is_some();
            runtime.status.rtp_bound_port =
                self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port);
            runtime.config_rev.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn status(&self, config: &Config) -> BridgeStatus {
        let rtp_active = self.rtp_server.lock().is_some();
        let bound_port = self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port);
        let guard = self.inner.lock();
        let mut status = guard
            .as_ref()
            .map(|r| r.status.clone())
            .unwrap_or_else(|| BridgeStatus::from(config));
        if let Some(runtime) = guard.as_ref() {
            status.midi_in = runtime.actual_midi_in.lock().clone();
            status.midi_out = runtime.actual_midi_out.lock().clone();
        }
        status.rtp_active = rtp_active;
        status.rtp_bound_port = bound_port;
        status
    }

    pub fn rtp_participants(&self) -> Vec<RtpParticipantInfo> {
        self.rtp_server
            .lock()
            .as_ref()
            .map(|server| server.participants())
            .unwrap_or_default()
    }

    pub fn owns_rtp_port(&self, port: u16) -> bool {
        let server_present = self.rtp_server.lock().is_some();
        if !server_present {
            return false;
        }
        self.rtp_config
            .lock()
            .as_ref()
            .map(|cfg| cfg.bound_port == port || cfg.requested_port == port)
            .unwrap_or(false)
    }

    fn stop_internal(&self) {
        let mut guard = self.inner.lock();
        if let Some(runtime) = guard.take() {
            let cfg_for_reset = runtime.config.lock().clone();
            runtime.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = runtime.processing {
                let _ = handle.join();
            }
            if let Some(handle) = runtime.midi_watcher {
                let _ = handle.join();
            }
            if let Some(handle) = runtime.activity_emitter {
                let _ = handle.join();
            }
            if cfg_for_reset.osc_enabled {
                if let Ok(osc) =
                    OscClient::new(&cfg_for_reset.osc_target_ip, cfg_for_reset.osc_target_port)
                {
                    let _ = osc.send_reset_all();
                }
            }
        }
        *self.rtp_sink.lock() = None;
        if let Some(server) = self.rtp_server.lock().take() {
            tauri::async_runtime::block_on(server.stop());
        }
        *self.rtp_config.lock() = None;
    }
}

fn resolve_remote_targets(config: &Config, logger: &FrontendLogger) -> Vec<RtpRemoteTarget> {
    if !config.rtp_remote_enabled {
        return Vec::new();
    }

    let mut entries = config.rtp_remotes.clone();
    if entries.is_empty() {
        let legacy_host = config.rtp_remote_host.trim();
        if !legacy_host.is_empty() {
            entries.push(crate::config::RtpRemoteEntry {
                id: "legacy".to_string(),
                name: legacy_host.to_string(),
                host: legacy_host.to_string(),
                port: config.rtp_remote_port,
                auto_connect: true,
            });
        }
    }

    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let entry_name = entry.name.trim().to_string();
        if !entry.auto_connect {
            continue;
        }
        let host = entry.host.trim();
        if host.is_empty() {
            logger.warn(format!("RTP-MIDI remote host is empty for {}", entry_name));
            continue;
        }
        if entry.port == 0 {
            logger.warn(format!("RTP-MIDI remote port is 0 for {}", entry_name));
            continue;
        }

        let resolved_addr = resolve_remote_socket_addr(host, entry.port, logger);

        let Some(addr) = resolved_addr else {
            continue;
        };
        if !seen.insert(addr) {
            continue;
        }
        resolved.push(RtpRemoteTarget {
            name: if entry_name.is_empty() {
                host.to_string()
            } else {
                entry_name
            },
            addr,
        });
    }
    resolved
}

fn resolve_remote_socket_addr(
    host: &str,
    port: u16,
    logger: &FrontendLogger,
) -> Option<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(SocketAddr::new(ip, port));
    }

    let addr = format!("{host}:{port}");
    match addr.to_socket_addrs() {
        Ok(iter) => {
            let mut addresses: Vec<SocketAddr> = iter.collect();
            addresses.sort_by_key(socket_addr_sort_key);
            addresses.dedup();
            let selected = addresses.first().copied();
            if selected.is_none() {
                logger.warn(format!("RTP-MIDI remote host unresolved: {host}"));
            }
            selected
        }
        Err(err) => {
            logger.warn(format!("RTP-MIDI remote host invalid: {host} ({err})"));
            None
        }
    }
}

fn socket_addr_sort_key(addr: &SocketAddr) -> (u8, IpAddr, u16) {
    let family_rank = match addr.ip() {
        IpAddr::V4(_) => 0u8,
        IpAddr::V6(_) => 1u8,
    };
    (family_rank, addr.ip(), addr.port())
}
fn watch_midi_input(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    midi_tx: Sender<MidiFrame>,
    stop: Arc<AtomicBool>,
    logger: FrontendLogger,
    actual_midi_in: Arc<Mutex<Option<String>>>,
) {
    let mut seen_rev = config_rev.load(Ordering::Relaxed);
    let mut cached_cfg = shared_config.lock().clone();
    let mut current_conn: Option<MidiInputConnection<Sender<MidiFrame>>> = None;
    let mut last_connect_error: Option<String> = None;
    let mut wait_for_config_change_only = false;

    while !stop.load(Ordering::Relaxed) {
        let rev_now = config_rev.load(Ordering::Relaxed);
        if rev_now != seen_rev {
            let new_cfg = shared_config.lock().clone();
            let midi_changed =
                new_cfg.midi_in != cached_cfg.midi_in || new_cfg.hotplug != cached_cfg.hotplug;
            cached_cfg = new_cfg;
            seen_rev = rev_now;
            wait_for_config_change_only = false;
            if midi_changed && current_conn.is_some() {
                logger.info("Reconnexion MIDI IN (changement de peripherique)");
                current_conn = None;
            }
        }

        if cached_cfg
            .midi_in
            .as_ref()
            .map(|s| s == RTP_VIRTUAL_INPUT)
            .unwrap_or(false)
        {
            current_conn = None;
            *actual_midi_in.lock() = Some(RTP_VIRTUAL_INPUT.to_string());
            last_connect_error = None;
            thread::sleep(Duration::from_millis(300));
            continue;
        }

        if current_conn.is_none() && !wait_for_config_change_only {
            match open_input(&cached_cfg, midi_tx.clone(), &logger) {
                Ok((conn, name)) => {
                    current_conn = Some(conn);
                    *actual_midi_in.lock() = Some(name);
                    last_connect_error = None;
                }
                Err(err) => {
                    *actual_midi_in.lock() = None;
                    if last_connect_error.as_ref() != Some(&err) {
                        logger.warn(format!("Entree MIDI indisponible: {err}"));
                        last_connect_error = Some(err.clone());
                    }
                    if !cached_cfg.hotplug {
                        wait_for_config_change_only = true;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn processing_loop(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    osc: OscClient,
    midi_rx: Receiver<MidiFrame>,
    stop: Arc<AtomicBool>,
    mut midi_out: Option<MidiOutputConnection>,
    logger: FrontendLogger,
    audio: AudioEngine,
    activity: Arc<Mutex<MidiActivityTracker>>,
    osc_counter: Arc<AtomicU32>,
    actual_midi_out: Arc<Mutex<Option<String>>>,
) {
    let initial_cfg = shared_config.lock().clone();
    logger.info(format!(
        "Bridge actif vers {}:{}",
        initial_cfg.osc_target_ip, initial_cfg.osc_target_port
    ));

    if let Some(out) = midi_out.as_mut() {
        reset_midi_output(out, &logger);
    }

    let mut osc_client = Some(osc);
    let mut current_target = (
        initial_cfg.osc_target_ip.clone(),
        initial_cfg.osc_target_port,
    );
    let mut snapshot = ConfigSnapshot::from(&initial_cfg);
    let mut cached_rev = config_rev.load(Ordering::Relaxed);
    let mut midi_out_name = initial_cfg.midi_out.clone();
    let mut sustain_pressed_state = [None; 16];

    while !stop.load(Ordering::Relaxed) {
        let rev_now = config_rev.load(Ordering::Relaxed);
        if rev_now != cached_rev {
            let cfg = shared_config.lock().clone();
            snapshot = ConfigSnapshot::from(&cfg);
            cached_rev = rev_now;
            if cfg.midi_out != midi_out_name {
                let (conn, connected_output) = open_output(&cfg, &logger);
                midi_out = conn;
                *actual_midi_out.lock() = connected_output;
                if let Some(out) = midi_out.as_mut() {
                    reset_midi_output(out, &logger);
                }
                midi_out_name = cfg.midi_out.clone();
            }
        }

        if snapshot.osc_enabled {
            let target_tuple = (snapshot.osc_target_ip.clone(), snapshot.osc_target_port);
            if target_tuple != current_target || osc_client.is_none() {
                match OscClient::new(&snapshot.osc_target_ip, snapshot.osc_target_port) {
                    Ok(new_osc) => {
                        current_target = target_tuple.clone();
                        osc_client = Some(new_osc);
                        logger.info(format!(
                            "Cible OSC mise a jour -> {}:{}",
                            target_tuple.0, target_tuple.1
                        ));
                    }
                    Err(err) => {
                        logger.error(format!("Impossible de creer le client OSC: {err}"));
                        osc_client = None;
                    }
                }
            }
        } else {
            if osc_client.is_some() {
                logger.debug("OSC desactive (client suspendu)");
            }
            osc_client = None;
            current_target = (snapshot.osc_target_ip.clone(), snapshot.osc_target_port);
        }

        match midi_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(frame) => {
                record_activity(&activity, &frame);
                handle_midi_frame(
                    &snapshot,
                    osc_client.as_ref(),
                    &mut midi_out,
                    &logger,
                    frame,
                    &audio,
                    &osc_counter,
                    &mut sustain_pressed_state,
                )
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }

    if let Some(out) = midi_out.as_mut() {
        reset_midi_output(out, &logger);
    }
}

fn handle_midi_frame(
    config: &ConfigSnapshot,
    osc: Option<&OscClient>,
    midi_out: &mut Option<MidiOutputConnection>,
    logger: &FrontendLogger,
    frame: MidiFrame,
    audio: &AudioEngine,
    osc_counter: &AtomicU32,
    sustain_pressed_state: &mut [Option<bool>; 16],
) {
    let mut frame = frame;
    if let Some(profile_idx) = config
        .routing_assignments
        .get(frame.source.as_ref())
        .copied()
    {
        if let Some(profile) = config.routing_profiles.get(profile_idx) {
            if profile.enabled && !apply_routing_profile(profile, &mut frame) {
                return;
            }
        }
    }

    if let Some(status) = frame.data.first() {
        // Ignore System Realtime messages (0xF8..0xFF)
        if *status >= 0xF8 {
            return;
        }
    }

    if config.verbose && should_log_debug() && frame.source.starts_with("RTP:") {
        logger.debug(format!(
            "MIDI IN (RTP) -> pipeline: {:02X?}",
            frame.data.as_slice()
        ));
    }

    // Forward every channel voice message to the VST, regardless of OSC filters,
    // so the internal synth always receives the full MIDI stream.
    if let Some(status) = frame.data.first() {
        let is_channel_voice = (0x80..0xF0).contains(status);
        if is_channel_voice {
            audio.send_midi(frame.data.as_slice());
            if config.verbose && should_log_debug() {
                logger.debug(format!(
                    "BRIDGE: VST MIDI de {}: {:02X?}",
                    frame.source,
                    frame.data.as_slice()
                ));
            }
        }
    }

    if config.midi_thru {
        if let Some(out) = midi_out.as_mut() {
            let _ = out.send(frame.data.as_slice());
        }
    }

    if let Some(sustain) = parse_sustain(frame.data.as_slice()) {
        // Appliquer le filtre canal UNIQUEMENT pour OSC, pas pour le VST
        if let Some(filter) = config.channel_filter {
            if sustain.channel != filter {
                if config.verbose && should_log_debug() {
                    logger.debug(format!(
                        "Sustain canal {} filtre (filtre: {})",
                        sustain.channel, filter
                    ));
                }
                return;
            }
        }

        let channel_idx = usize::from(sustain.channel.saturating_sub(1)).min(15);
        if sustain_pressed_state[channel_idx]
            .map(|previous| previous == sustain.pressed)
            .unwrap_or(false)
        {
            return;
        }
        sustain_pressed_state[channel_idx] = Some(sustain.pressed);

        if !config.osc_enabled {
            if config.verbose && should_log_debug() {
                logger.debug(format!(
                    "OSC desactive -> sustain ignoree pour OSC depuis {}",
                    frame.source
                ));
            }
            return;
        }

        if let Some(osc_client) = osc {
            if let Err(err) = osc_client.send_param(OSC_SUSTAIN_PARAM, sustain.pressed) {
                if logs_enabled() {
                    logger.error(format!("Envoi OSC echoue: {err}"));
                }
            } else if config.log_osc && logs_enabled() {
                logger.info(format!(
                    "{OSC_SUSTAIN_PARAM} -> {}",
                    if sustain.pressed { 1 } else { 0 }
                ));
                osc_counter.fetch_add(1, Ordering::Relaxed);
            } else {
                osc_counter.fetch_add(1, Ordering::Relaxed);
            }
        } else if config.verbose && logs_enabled() {
            logger.warn(format!(
                "OSC actif mais client indisponible pour {}:{}",
                config.osc_target_ip, config.osc_target_port
            ));
        }
    } else if let Some(note) = parse_note(frame.data.as_slice()) {
        if note.index.is_some() {
            let pressed = matches!(note.kind, MidiKind::NoteOn);
            let _ = logger.app_handle().emit(
                MIDI_NOTE_EVENT,
                MidiNoteEvent {
                    source: frame.source.to_string(),
                    note: note.note,
                    channel: note.channel,
                    pressed,
                    timestamp_ms: now_ms(),
                },
            );
        }

        // Appliquer le filtre canal UNIQUEMENT pour OSC, pas pour le VST
        if let Some(filter) = config.channel_filter {
            if note.channel != filter {
                // Log mais ne retourne pas - le VST a deja recu le MIDI ci-dessus
                if config.verbose && should_log_debug() {
                    logger.debug(format!(
                        "Note canal {} filtree (filtre: {})",
                        note.channel, filter
                    ));
                }
                return;
            }
        }

        if note.index.is_none() && config.verbose && logs_enabled() {
            logger.warn(format!(
                "Note hors plage ({}) recue depuis {}",
                note.note, frame.source
            ));
            return;
        }

        if let Some(index) = note.index {
            if !config.osc_enabled {
                if config.verbose && should_log_debug() {
                    logger.debug(format!(
                        "OSC desactive -> note {} ignoree pour OSC depuis {}",
                        index, frame.source
                    ));
                }
                return;
            }
            let param = crate::osc::parameter_name(index);
            let pressed = match note.kind {
                MidiKind::NoteOn => true,
                MidiKind::NoteOff => false,
            };

            if let Some(osc_client) = osc {
                if let Err(err) = osc_client.send_param(&param, pressed) {
                    if logs_enabled() {
                        logger.error(format!("Envoi OSC echoue: {err}"));
                    }
                } else if config.log_osc && logs_enabled() {
                    logger.info(format!("{param} -> {}", if pressed { 1 } else { 0 }));
                    osc_counter.fetch_add(1, Ordering::Relaxed);
                } else {
                    osc_counter.fetch_add(1, Ordering::Relaxed);
                }
            } else if config.verbose && logs_enabled() {
                logger.warn(format!(
                    "OSC actif mais client indisponible pour {}:{}",
                    config.osc_target_ip, config.osc_target_port
                ));
            }
        }
    } else if config.verbose && should_log_debug() {
        logger.debug(format!(
            "Message ignore depuis {}: {:?}",
            frame.source,
            frame.data.as_slice()
        ));
    }
}

fn apply_routing_profile(profile: &RoutingProfileRuntime, frame: &mut MidiFrame) -> bool {
    let Some(status) = frame.data.first().copied() else {
        return true;
    };
    if status >= 0xF0 {
        return true;
    }
    let channel = (status & 0x0F) + 1;
    if let Some(filter) = profile.channel_filter {
        if channel != filter {
            return false;
        }
    }

    match status & 0xF0 {
        0x80 | 0x90 => {
            if let Some(note) = frame.data.get(1).copied() {
                if let Some(min) = profile.note_min {
                    if note < min {
                        return false;
                    }
                }
                if let Some(max) = profile.note_max {
                    if note > max {
                        return false;
                    }
                }
            }
        }
        0xB0 => {
            if let Some(cc) = frame.data.get_mut(1) {
                let idx = *cc as usize;
                *cc = profile.cc_map[idx];
            }
        }
        0xC0 => {
            if let Some(program) = frame.data.get_mut(1) {
                let idx = *program as usize;
                *program = profile.program_map[idx];
            }
        }
        _ => {}
    }
    true
}

#[derive(Clone)]
struct ConfigSnapshot {
    osc_target_ip: String,
    osc_target_port: u16,
    osc_enabled: bool,
    channel_filter: Option<u8>,
    midi_thru: bool,
    log_osc: bool,
    verbose: bool,
    routing_profiles: Vec<RoutingProfileRuntime>,
    routing_assignments: HashMap<String, usize>,
}

impl From<&Config> for ConfigSnapshot {
    fn from(cfg: &Config) -> Self {
        let routing_profiles: Vec<RoutingProfileRuntime> = cfg
            .routing_profiles
            .iter()
            .map(RoutingProfileRuntime::from_profile)
            .collect();
        let mut profile_index: HashMap<String, usize> = HashMap::new();
        for (idx, profile) in routing_profiles.iter().enumerate() {
            profile_index.insert(profile.id.clone(), idx);
        }
        let mut routing_assignments = HashMap::new();
        for RoutingAssignment { source, profile_id } in &cfg.routing_assignments {
            if let Some(idx) = profile_index.get(profile_id) {
                routing_assignments.insert(source.clone(), *idx);
            }
        }

        Self {
            osc_target_ip: cfg.osc_target_ip.clone(),
            osc_target_port: cfg.osc_target_port,
            osc_enabled: cfg.osc_enabled,
            channel_filter: cfg.channel_filter,
            midi_thru: cfg.midi_thru,
            log_osc: cfg.log_osc,
            verbose: cfg.verbose,
            routing_profiles,
            routing_assignments,
        }
    }
}

fn open_input(
    config: &Config,
    midi_tx: Sender<MidiFrame>,
    logger: &FrontendLogger,
) -> Result<(MidiInputConnection<Sender<MidiFrame>>, String), String> {
    let mut input = MidiInput::new("OSCMidi").map_err(|e| e.to_string())?;
    input.ignore(Ignore::TimeAndActiveSense);

    let ports = input.ports();
    let port = select_input_port(&input, &ports, config.midi_in.as_ref())
        .ok_or_else(|| "Aucune entree MIDI trouvee ou correspondante".to_string())?;

    let name = input
        .port_name(&port)
        .unwrap_or_else(|_| "MIDI IN".to_string());
    logger.info(format!("Entree MIDI connectee: {name}"));
    let source: Arc<str> = Arc::from(name.clone());
    let tx = midi_tx.clone();
    input
        .connect(
            &port,
            "osc-midi-in",
            move |_timestamp, message, _| {
                let _ = tx.send(MidiFrame {
                    data: SmallVec::from_slice(message),
                    source: source.clone(),
                });
            },
            midi_tx,
        )
        .map(|conn| (conn, name))
        .map_err(|e| e.to_string())
}

fn open_output(
    config: &Config,
    logger: &FrontendLogger,
) -> (Option<MidiOutputConnection>, Option<String>) {
    if let Some(name) = config.midi_out.as_ref() {
        if name == VST_INTERNAL_OUTPUT || name == VST_INTERNAL_OUTPUT_LEGACY {
            return (None, Some(VST_INTERNAL_OUTPUT.to_string()));
        }
    } else {
        return (None, None);
    }
    let output = match MidiOutput::new("OSCMidi") {
        Ok(output) => output,
        Err(err) => {
            logger.warn(format!("Sortie MIDI indisponible: {err}"));
            return (None, None);
        }
    };
    let ports = output.ports();
    let Some(port) = select_output_port(&output, &ports, config.midi_out.as_ref()) else {
        if let Some(requested) = config.midi_out.as_ref() {
            logger.warn(format!("Sortie MIDI introuvable: {requested}"));
        }
        return (None, None);
    };

    let name = output
        .port_name(&port)
        .unwrap_or_else(|_| "MIDI OUT".to_string());
    logger.info(format!("Sortie MIDI connectee: {name}"));
    match output.connect(&port, "osc-midi-out") {
        Ok(conn) => (Some(conn), Some(name)),
        Err(err) => {
            logger.warn(format!("Sortie MIDI indisponible: {err}"));
            (None, None)
        }
    }
}

fn select_input_port<'a>(
    input: &'a MidiInput,
    ports: &'a [MidiInputPort],
    preferred: Option<&String>,
) -> Option<MidiInputPort> {
    let name = preferred?;
    for port in ports {
        if let Ok(port_name) = input.port_name(port) {
            if port_name.contains(name) || port_name == *name {
                return Some(port.clone());
            }
        }
    }
    None
}

fn select_output_port<'a>(
    output: &'a MidiOutput,
    ports: &'a [MidiOutputPort],
    preferred: Option<&String>,
) -> Option<MidiOutputPort> {
    let name = preferred?;
    for port in ports {
        if let Ok(port_name) = output.port_name(port) {
            if port_name.contains(name) || port_name == *name {
                return Some(port.clone());
            }
        }
    }
    None
}

fn initial_connected_input(config: &Config) -> Option<String> {
    config
        .midi_in
        .as_ref()
        .filter(|name| name.as_str() == RTP_VIRTUAL_INPUT)
        .cloned()
}

fn initial_connected_output(config: &Config) -> Option<String> {
    config.midi_out.as_ref().and_then(|name| {
        if name == VST_INTERNAL_OUTPUT || name == VST_INTERNAL_OUTPUT_LEGACY {
            Some(VST_INTERNAL_OUTPUT.to_string())
        } else {
            None
        }
    })
}

fn reset_midi_output(out: &mut MidiOutputConnection, logger: &FrontendLogger) {
    for ch in 0u8..16 {
        let _ = out.send(&[0xB0 | ch, 64, 0]); // Sustain off
        let _ = out.send(&[0xB0 | ch, 120, 0]); // All Sound Off
        let _ = out.send(&[0xB0 | ch, 121, 0]); // Reset All Controllers
        let _ = out.send(&[0xB0 | ch, 123, 0]); // All Notes Off
        let _ = out.send(&[0xE0 | ch, 0x00, 0x40]); // Pitch bend center
    }
    logger.debug("MIDI OUT: sustain/all-sound/controllers/all-notes reset + pitch bend");
}

fn record_activity(activity: &Arc<Mutex<MidiActivityTracker>>, frame: &MidiFrame) {
    let mut guard = activity.lock();
    guard.record(&frame.source, frame.data.as_slice());
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
