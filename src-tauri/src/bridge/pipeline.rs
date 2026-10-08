use crate::midi_queue::Sender;
use crate::{
    audio::AudioEngine,
    config::{Config, RoutingAssignment, RoutingMapping, RoutingProfile},
    logger::{logs_enabled, should_log_debug, FrontendLogger},
    midi::{is_critical_release_message, parse_note, parse_sustain, MidiFrame, MidiKind},
    osc::OscClient,
    types::MidiNoteEvent,
};
use crossbeam_channel::TrySendError;
use midir::MidiOutputConnection;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use super::activity::now_ms;

const OSC_SUSTAIN_PARAM: &str = "/avatar/parameters/sustain";

// Compteurs statiques pour les métriques de pipeline (stress test)
static PIPELINE_IN_COUNT: AtomicU64 = AtomicU64::new(0);
static PIPELINE_OUT_COUNT: AtomicU64 = AtomicU64::new(0);
static PIPELINE_FILTERED_COUNT: AtomicU64 = AtomicU64::new(0);
static PIPELINE_QUEUE_DEPTH: AtomicU64 = AtomicU64::new(0);
static PIPELINE_QUEUE_MAX_DEPTH: AtomicU64 = AtomicU64::new(0);
static PIPELINE_DROPPED_COUNT: AtomicU64 = AtomicU64::new(0);
static MIDI_DEBUG_SAMPLE_LAST_MS: AtomicU64 = AtomicU64::new(0);
static CRITICAL_MIDI_RESET_REQUESTED: AtomicBool = AtomicBool::new(false);
static MIDI_RESET_GENERATION: std::sync::LazyLock<std::sync::Arc<AtomicU64>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(AtomicU64::new(0)));
#[cfg(test)]
pub(crate) static TEST_PIPELINE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn midi_reset_generation() -> u64 {
    MIDI_RESET_GENERATION.load(Ordering::Acquire)
}

pub(crate) fn midi_reset_generation_source() -> std::sync::Arc<AtomicU64> {
    std::sync::Arc::clone(&MIDI_RESET_GENERATION)
}

pub(super) fn take_critical_midi_reset_request() -> bool {
    CRITICAL_MIDI_RESET_REQUESTED.swap(false, Ordering::AcqRel)
}

pub(crate) fn request_critical_midi_reset() {
    MIDI_RESET_GENERATION.fetch_add(1, Ordering::AcqRel);
    CRITICAL_MIDI_RESET_REQUESTED.store(true, Ordering::Release);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Enqueued,
    DroppedNonCritical,
    /// Profile rejection or an invalidated preparation during route maintenance.
    Filtered,
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueError {
    #[error("Bridge queue full; MIDI message dropped")]
    QueueFull,
    #[error("Invalid MIDI frame")]
    InvalidMessage,
    #[error("Bridge queue full; critical MIDI release dropped")]
    CriticalReleaseDropped,
    #[error("Bridge not running")]
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineMetricsSnapshot {
    pub queue_depth: u64,
    pub queue_max_depth: u64,
    pub messages_in: u64,
    pub messages_out: u64,
    pub messages_filtered: u64,
    pub messages_dropped: u64,
}

#[derive(Clone, PartialEq, Eq)]
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

#[derive(Clone)]
pub(super) struct ConfigSnapshot {
    pub(super) rtp_output: Option<crate::rtp::rtp_output::RtpOutputRoute>,
    rtp_thru: bool,
    pub(super) osc_target_ip: String,
    pub(super) osc_target_port: u16,
    pub(super) osc_enabled: bool,
    pub(super) channel_filter: Option<u8>,
    pub(super) midi_thru: bool,
    pub(super) log_osc: bool,
    pub(super) verbose: bool,
    routing_profiles: Vec<RoutingProfileRuntime>,
    routing_assignments: HashMap<String, usize>,
}

impl ConfigSnapshot {
    pub(super) fn same_profiles(&self, other: &Self) -> bool {
        self.routing_profiles == other.routing_profiles
            && self.routing_assignments == other.routing_assignments
    }
}

/// Upstream deadline queues need the destination's release priority too. A
/// channel message fits inline; SysEx cannot release a note or sustain.
pub(crate) fn classify_input_frame(tx: &Sender<MidiFrame>, frame: &MidiFrame) -> bool {
    if frame.data.len() > 3 {
        return false;
    }
    let Some(mut candidate) = frame.clone_realtime() else {
        return false;
    };
    tx.prepare(&mut candidate) && is_critical_release_message(&candidate.data)
}

pub(super) fn prepare_input_routing(tx: &Sender<MidiFrame>, cfg: &Config) {
    let snapshot = ConfigSnapshot::from(cfg);
    tx.set_preparation(move |frame| {
        snapshot
            .routing_assignments
            .get(&*frame.source)
            .and_then(|index| snapshot.routing_profiles.get(*index))
            .filter(|profile| profile.enabled)
            .is_none_or(|profile| apply_routing_profile(profile, frame))
    });
}

impl From<&Config> for ConfigSnapshot {
    fn from(cfg: &Config) -> Self {
        let routing_profiles: Vec<RoutingProfileRuntime> = cfg
            .midi
            .routing_profiles
            .iter()
            .map(RoutingProfileRuntime::from_profile)
            .collect();
        let mut profile_index: HashMap<String, usize> = HashMap::new();
        for (idx, profile) in routing_profiles.iter().enumerate() {
            profile_index.insert(profile.id.clone(), idx);
        }
        let mut routing_assignments = HashMap::new();
        for RoutingAssignment { source, profile_id } in &cfg.midi.routing_assignments {
            if let Some(idx) = profile_index.get(profile_id) {
                routing_assignments.insert(source.clone(), *idx);
            }
        }

        Self {
            rtp_output: None,
            rtp_thru: cfg.rtp.thru_enabled,
            osc_target_ip: cfg.osc.target_ip.clone(),
            osc_target_port: cfg.osc.target_port,
            osc_enabled: cfg.osc.enabled,
            channel_filter: cfg.midi.channel_filter,
            midi_thru: cfg.midi.thru_enabled,
            log_osc: cfg.osc.log_messages && !cfg.audio.game_mode,
            verbose: cfg.logging.verbose && !cfg.audio.game_mode,
            routing_profiles,
            routing_assignments,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_midi_frame(
    config: &ConfigSnapshot,
    osc: Option<&OscClient>,
    midi_out: &mut Option<MidiOutputConnection>,
    logger: &FrontendLogger,
    frame: MidiFrame,
    audio: &AudioEngine,
    osc_counter: &AtomicU32,
    sustain_pressed_state: &mut [Option<bool>; 16],
    midi_event_tx: &Sender<MidiNoteEvent>,
    deliver_audio: bool,
    deliver_midi_thru: bool,
    deliver_osc_ui: bool,
    active_notes: &mut crate::midi::ActiveNotes,
    route_recovery: Option<&crate::midi_queue::Recovery>,
) {
    // Compteur entrée pipeline (stress test metrics)
    if deliver_audio {
        PIPELINE_IN_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    if deliver_audio && frame.data.as_slice() == [0xFF] {
        if let Some(recovery) = route_recovery {
            recovery.request_reset();
        } else {
            request_critical_midi_reset();
        }
        return;
    }
    let mut filtered = false;
    // A load test must measure the queues/audio callback, not benchmark the
    // synchronous file logger by writing thousands of lines per second.
    let log_frame = !frame.source.starts_with("StressTest");
    let log_sample =
        config.verbose && log_frame && should_log_debug() && should_sample_midi_debug(now_ms());

    // Profiles were applied atomically at input admission, before priority classification.

    if deliver_audio && config.rtp_thru {
        if let Some(output) = config.rtp_output.as_ref() {
            output.send(&frame);
        }
    }

    if let Some(status) = frame.data.first() {
        if *status >= 0xF8 {
            // Messages temps-réel système (clock, etc) - pas des notes MIDI
            // Ne pas compter comme filtré, juste ignorés pour le pipeline audio
            filtered = true;
        }
    }

    if log_sample {
        let input_label = if frame.source.starts_with("RTP:") {
            Some("RTP")
        } else if frame.source.starts_with("ReliablePLV:") {
            Some("ReliablePLV")
        } else {
            None
        };
        if let Some(label) = input_label {
            logger.debug(format!(
                "MIDI IN ({label}) -> pipeline: {:02X?}",
                frame.data.as_slice()
            ));
        }
    }

    if deliver_audio {
        if let Some(status) = frame.data.first() {
            let is_channel_voice = (0x80..0xF0).contains(status);
            if is_channel_voice {
                match audio.send_midi_at(frame.data.as_slice(), frame.ingress_qpc) {
                    crate::audio::MidiSendOutcome::Sent => {
                        PIPELINE_OUT_COUNT.fetch_add(1, Ordering::Relaxed);
                    }
                    crate::audio::MidiSendOutcome::AudioStopped => {
                        PIPELINE_FILTERED_COUNT.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        PIPELINE_DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if log_sample {
                    logger.debug(format!(
                        "BRIDGE: VST MIDI de {}: {:02X?}",
                        frame.source,
                        frame.data.as_slice()
                    ));
                }
            } else if !filtered {
                // Message MIDI non-voix (sys ex, etc) - compté comme filtré pour les stats
                PIPELINE_FILTERED_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    if deliver_midi_thru && config.midi_thru {
        if let Some(out) = midi_out.as_mut() {
            if let Err(error) = out.send(frame.data.as_slice()) {
                logger.warn(format!("Sortie MIDI déconnectée: {error}"));
                *midi_out = None;
                if let Some(recovery) = route_recovery {
                    recovery.request_reset();
                } else {
                    request_critical_midi_reset();
                }
            } else {
                active_notes.observe(frame.data.as_slice());
            }
        }
    }

    if !deliver_osc_ui {
        return;
    }

    if let Some(sustain) = parse_sustain(frame.data.as_slice()) {
        if let Some(filter) = config.channel_filter {
            if sustain.channel != filter {
                if log_sample {
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

        if !config.osc_enabled || frame.source.starts_with("OSC:") {
            if log_sample {
                logger.debug(format!(
                    "OSC desactive -> sustain ignoree pour OSC depuis {}",
                    frame.source
                ));
            }
            return;
        }

        if let Some(osc_client) = osc {
            if let Err(err) = osc_client.send_sustain(sustain.pressed) {
                if !sustain.pressed {
                    if let Some(recovery) = route_recovery {
                        recovery.request_reset();
                    } else {
                        request_critical_midi_reset();
                    }
                }
                if logs_enabled() {
                    logger.error(format!("Envoi OSC echoue: {err}"));
                }
            } else if config.log_osc && logs_enabled() {
                sustain_pressed_state[channel_idx] = Some(sustain.pressed);
                logger.info(format!(
                    "{OSC_SUSTAIN_PARAM} -> {}",
                    if sustain.pressed { 1 } else { 0 }
                ));
                osc_counter.fetch_add(1, Ordering::Relaxed);
            } else {
                sustain_pressed_state[channel_idx] = Some(sustain.pressed);
                osc_counter.fetch_add(1, Ordering::Relaxed);
            }
        } else if log_sample && logs_enabled() {
            logger.warn(format!(
                "OSC actif mais client indisponible pour {}:{}",
                config.osc_target_ip, config.osc_target_port
            ));
        }
    } else if let Some(note) = parse_note(frame.data.as_slice()) {
        if note.index.is_some() {
            let pressed = matches!(note.kind, MidiKind::NoteOn);
            let event = MidiNoteEvent {
                source: frame.source.to_string(),
                note: note.note,
                channel: note.channel,
                pressed,
                timestamp_ms: now_ms(),
            };
            let _ = midi_event_tx.try_send(event);
        }

        if let Some(filter) = config.channel_filter {
            if note.channel != filter {
                if log_sample {
                    logger.debug(format!(
                        "Note canal {} filtree (filtre: {})",
                        note.channel, filter
                    ));
                }
                return;
            }
        }

        if note.index.is_none() && log_sample && logs_enabled() {
            logger.warn(format!(
                "Note hors plage ({}) recue depuis {}",
                note.note, frame.source
            ));
            return;
        }

        if let Some(index) = note.index {
            if !config.osc_enabled || frame.source.starts_with("OSC:") {
                if log_sample {
                    logger.debug(format!(
                        "OSC desactive -> note {} ignoree pour OSC depuis {}",
                        index, frame.source
                    ));
                }
                return;
            }
            let pressed = match note.kind {
                MidiKind::NoteOn => true,
                MidiKind::NoteOff => false,
            };

            if let Some(osc_client) = osc {
                if let Err(err) = osc_client.send_note(index, pressed) {
                    if !pressed {
                        if let Some(recovery) = route_recovery {
                            recovery.request_reset();
                        } else {
                            request_critical_midi_reset();
                        }
                    }
                    if logs_enabled() {
                        logger.error(format!("Envoi OSC echoue: {err}"));
                    }
                } else if config.log_osc && logs_enabled() {
                    let param = crate::osc::parameter_name(index);
                    logger.info(format!("{param} -> {}", if pressed { 1 } else { 0 }));
                    osc_counter.fetch_add(1, Ordering::Relaxed);
                } else {
                    osc_counter.fetch_add(1, Ordering::Relaxed);
                }
            } else if log_sample && logs_enabled() {
                logger.warn(format!(
                    "OSC actif mais client indisponible pour {}:{}",
                    config.osc_target_ip, config.osc_target_port
                ));
            }
        }
    } else if log_sample {
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
                let Some(mapped) = profile.cc_map.get(idx) else {
                    return false;
                };
                *cc = *mapped;
            }
        }
        0xC0 => {
            if let Some(program) = frame.data.get_mut(1) {
                let idx = *program as usize;
                let Some(mapped) = profile.program_map.get(idx) else {
                    return false;
                };
                *program = *mapped;
            }
        }
        _ => {}
    }
    true
}

/// Retourne les statistiques de pipeline (in, out, filtered).
pub fn pipeline_stats() -> (u64, u64, u64) {
    (
        PIPELINE_IN_COUNT.load(Ordering::Relaxed),
        PIPELINE_OUT_COUNT.load(Ordering::Relaxed),
        PIPELINE_FILTERED_COUNT.load(Ordering::Relaxed),
    )
}

pub fn update_pipeline_queue_depth(depth: usize) {
    let depth = depth as u64;
    PIPELINE_QUEUE_DEPTH.store(depth, Ordering::Relaxed);

    let mut current = PIPELINE_QUEUE_MAX_DEPTH.load(Ordering::Relaxed);
    while depth > current {
        match PIPELINE_QUEUE_MAX_DEPTH.compare_exchange(
            current,
            depth,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

pub fn pipeline_metrics_snapshot() -> PipelineMetricsSnapshot {
    PipelineMetricsSnapshot {
        queue_depth: PIPELINE_QUEUE_DEPTH.load(Ordering::Relaxed),
        queue_max_depth: PIPELINE_QUEUE_MAX_DEPTH.load(Ordering::Relaxed),
        messages_in: PIPELINE_IN_COUNT.load(Ordering::Relaxed),
        messages_out: PIPELINE_OUT_COUNT.load(Ordering::Relaxed),
        messages_filtered: PIPELINE_FILTERED_COUNT.load(Ordering::Relaxed),
        messages_dropped: PIPELINE_DROPPED_COUNT.load(Ordering::Relaxed),
    }
}

fn should_sample_midi_debug(now_ms: u64) -> bool {
    let last = MIDI_DEBUG_SAMPLE_LAST_MS.load(Ordering::Relaxed);
    now_ms.saturating_sub(last) >= 1_000
        && MIDI_DEBUG_SAMPLE_LAST_MS
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

pub(super) fn record_fanout_drop() {
    PIPELINE_DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn try_enqueue_midi_frame(
    tx: &Sender<MidiFrame>,
    mut frame: MidiFrame,
) -> Result<EnqueueOutcome, EnqueueError> {
    if !crate::midi::normalize_message(&mut frame.data) {
        return Err(EnqueueError::InvalidMessage);
    }
    // Apply the published input profile once, before priority admission.
    if !tx.prepare(&mut frame) {
        PIPELINE_IN_COUNT.fetch_add(1, Ordering::Relaxed);
        PIPELINE_FILTERED_COUNT.fetch_add(1, Ordering::Relaxed);
        return Ok(EnqueueOutcome::Filtered);
    }
    match tx.try_send_retryable(frame) {
        Ok(()) => Ok(EnqueueOutcome::Enqueued),
        Err(TrySendError::Full(frame)) => {
            if is_critical_release_message(frame.data.as_slice()) {
                std::thread::yield_now();
                match tx.try_send_retryable(frame) {
                    Ok(()) => Ok(EnqueueOutcome::Enqueued),
                    Err(TrySendError::Full(frame)) => {
                        tx.record_abandoned(&frame);
                        PIPELINE_DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
                        request_critical_midi_reset();
                        Err(EnqueueError::CriticalReleaseDropped)
                    }
                    Err(TrySendError::Disconnected(frame)) => {
                        tx.record_abandoned(&frame);
                        request_critical_midi_reset();
                        Err(EnqueueError::Disconnected)
                    }
                }
            } else {
                tx.record_abandoned(&frame);
                PIPELINE_DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
                Ok(EnqueueOutcome::DroppedNonCritical)
            }
        }
        Err(TrySendError::Disconnected(frame)) => {
            tx.record_abandoned(&frame);
            if is_critical_release_message(frame.data.as_slice()) {
                request_critical_midi_reset();
            }
            Err(EnqueueError::Disconnected)
        }
    }
}

/// Réinitialise les compteurs de pipeline (utile pour les tests).
pub fn reset_pipeline_stats() {
    PIPELINE_IN_COUNT.store(0, Ordering::Relaxed);
    PIPELINE_OUT_COUNT.store(0, Ordering::Relaxed);
    PIPELINE_FILTERED_COUNT.store(0, Ordering::Relaxed);
    PIPELINE_QUEUE_DEPTH.store(0, Ordering::Relaxed);
    PIPELINE_QUEUE_MAX_DEPTH.store(0, Ordering::Relaxed);
    PIPELINE_DROPPED_COUNT.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_queue::bounded;
    use std::sync::Arc;

    fn frame(data: &[u8]) -> MidiFrame {
        MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::SmallVec::from_slice(data),
            source: Arc::from("test"),
        }
    }

    fn remapped_sustain_config() -> Config {
        let mut config = Config::default();
        config.midi.routing_profiles.push(RoutingProfile {
            id: "sustain-map".into(),
            name: "Sustain".into(),
            enabled: true,
            channel_filter: None,
            note_min: None,
            note_max: None,
            cc_map: vec![
                RoutingMapping { from: 7, to: 64 },
                RoutingMapping { from: 64, to: 123 },
            ],
            program_map: vec![
                RoutingMapping { from: 1, to: 2 },
                RoutingMapping { from: 2, to: 3 },
            ],
        });
        config.midi.routing_assignments.push(RoutingAssignment {
            source: "test".into(),
            profile_id: "sustain-map".into(),
        });
        config
    }

    #[test]
    fn mapped_sustain_release_uses_reserved_input_admission_and_is_not_mapped_twice() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let config = remapped_sustain_config();
        let (tx, rx) = bounded(8);
        prepare_input_routing(&tx, &config);
        for _ in 0..7 {
            assert_eq!(
                try_enqueue_midi_frame(&tx, frame(&[0x90, 60, 100])).unwrap(),
                EnqueueOutcome::Enqueued
            );
        }
        assert!(classify_input_frame(&tx, &frame(&[0xB0, 7, 0])));
        assert!(!classify_input_frame(&tx, &frame(&[0xB0, 7, 127])));
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xB0, 7, 0])).unwrap(),
            EnqueueOutcome::Enqueued
        );
        for _ in 0..7 {
            rx.recv().unwrap();
        }
        let release = rx.recv().unwrap();
        assert_eq!(release.data.as_slice(), &[0xB0, 64, 0]);
        assert!(is_critical_release_message(&release.data));
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xC0, 1])).unwrap(),
            EnqueueOutcome::Enqueued
        );
        let program = rx.recv().unwrap();
        assert_eq!(program.data.as_slice(), &[0xC0, 2]);
    }

    #[test]
    fn sustain_remapped_to_noncritical_control_drops_without_reset() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, _rx) = bounded(8);
        let mut config = remapped_sustain_config();
        config.midi.routing_profiles[0].cc_map = vec![RoutingMapping { from: 64, to: 7 }];
        prepare_input_routing(&tx, &config);
        for _ in 0..7 {
            try_enqueue_midi_frame(&tx, frame(&[0x90, 60, 100])).unwrap();
        }
        let generation = midi_reset_generation();
        assert!(!classify_input_frame(&tx, &frame(&[0xB0, 64, 0])));
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xB0, 64, 0])).unwrap(),
            EnqueueOutcome::DroppedNonCritical
        );
        assert_eq!(midi_reset_generation(), generation);
        assert_eq!(tx.recovery().snapshot().critical_drops, 0);
        assert_eq!(tx.recovery().snapshot().noncritical_drops, 1);
    }

    #[test]
    fn in_flight_old_profile_cannot_admit_after_new_profile_publication() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, rx) = bounded(8);
        let old_config = remapped_sustain_config();
        let old_profile = RoutingProfileRuntime::from_profile(&old_config.midi.routing_profiles[0]);
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
        let resume_rx = std::sync::Mutex::new(resume_rx);
        tx.set_preparation(move |frame| {
            entered_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv().unwrap();
            apply_routing_profile(&old_profile, frame)
        });
        let old_tx = tx.clone();
        let pending =
            std::thread::spawn(move || try_enqueue_midi_frame(&old_tx, frame(&[0xB0, 7, 0])));
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        tx.invalidate_preparation();
        request_critical_midi_reset();
        // An arrival in the reset/publication gap must not run the old mapping
        // while carrying the newly advanced common epoch.
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xB0, 7, 0])).unwrap(),
            EnqueueOutcome::Filtered
        );
        assert!(rx.try_recv().is_err());
        let mut next = old_config.clone();
        next.midi.routing_profiles[0].cc_map[0].to = 123;
        prepare_input_routing(&tx, &next);
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xB0, 7, 0])).unwrap(),
            EnqueueOutcome::Enqueued
        );
        resume_tx.send(()).unwrap();
        assert_eq!(pending.join().unwrap().unwrap(), EnqueueOutcome::Filtered);
        let admitted = rx.recv().unwrap();
        assert_eq!(admitted.data.as_slice(), &[0xB0, 123, 0]);
        assert_eq!(admitted.generation, midi_reset_generation());
        assert!(rx.try_recv().is_err());
        assert_eq!(tx.recovery().snapshot().critical_drops, 0);
    }

    #[test]
    fn profile_filters_run_before_admission_and_publish_with_mapping() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, rx) = bounded(8);
        let mut config = remapped_sustain_config();
        config.midi.routing_profiles[0].channel_filter = Some(2);
        config.midi.routing_profiles[0].note_min = Some(60);
        config.midi.routing_profiles[0].note_max = Some(72);
        prepare_input_routing(&tx, &config);
        for data in [
            [0xB0, 7, 0],
            [0x90, 60, 100],
            [0x91, 59, 100],
            [0x81, 73, 0],
        ] {
            assert_eq!(
                try_enqueue_midi_frame(&tx, frame(&data)).unwrap(),
                EnqueueOutcome::Filtered
            );
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xB1, 7, 0])).unwrap(),
            EnqueueOutcome::Enqueued
        );
        assert_eq!(rx.recv().unwrap().data.as_slice(), &[0xB1, 64, 0]);
        assert_eq!(tx.recovery().snapshot().noncritical_drops, 0);
        assert_eq!(tx.recovery().snapshot().critical_drops, 0);
    }

    #[test]
    fn lost_mapped_release_requests_reset_and_counts_one_abandonment() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, _rx) = bounded(1);
        prepare_input_routing(&tx, &remapped_sustain_config());
        try_enqueue_midi_frame(&tx, frame(&[0x80, 60, 0])).unwrap();
        let generation = midi_reset_generation();
        assert_eq!(
            try_enqueue_midi_frame(&tx, frame(&[0xB0, 7, 0])),
            Err(EnqueueError::CriticalReleaseDropped)
        );
        assert_eq!(midi_reset_generation(), generation + 1);
        let diagnostics = tx.recovery().snapshot();
        assert_eq!(diagnostics.critical_drops, 1);
        assert_eq!(diagnostics.critical_rejections, 2);
    }

    #[test]
    fn pipeline_metrics_snapshot_tracks_queue_depth_and_counts() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        reset_pipeline_stats();
        PIPELINE_IN_COUNT.store(10, Ordering::Relaxed);
        PIPELINE_OUT_COUNT.store(9, Ordering::Relaxed);
        update_pipeline_queue_depth(7);
        update_pipeline_queue_depth(3);

        let snapshot = pipeline_metrics_snapshot();

        assert_eq!(snapshot.queue_depth, 3);
        assert_eq!(snapshot.queue_max_depth, 7);
        assert_eq!(snapshot.messages_in, 10);
        assert_eq!(snapshot.messages_out, 9);
        assert_eq!(snapshot.messages_dropped, 0);
    }

    #[test]
    fn pipeline_debug_logging_names_reliable_playback_input() {
        let source = include_str!("pipeline.rs");

        assert!(source.contains("ReliablePLV:"));
        assert!(source.contains("MIDI IN (ReliablePLV) -> pipeline"));
    }

    #[test]
    fn midi_debug_sampling_is_rate_limited() {
        MIDI_DEBUG_SAMPLE_LAST_MS.store(0, Ordering::Relaxed);
        assert!(should_sample_midi_debug(1_000));
        assert!(!should_sample_midi_debug(1_001));
        assert!(!should_sample_midi_debug(1_999));
        assert!(should_sample_midi_debug(2_000));
    }

    #[test]
    fn bounded_enqueue_reports_non_critical_drop() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, _rx) = bounded(1);
        tx.try_send(frame(&[0x90, 60, 100])).expect("prefill");

        let outcome = try_enqueue_midi_frame(&tx, frame(&[0x90, 61, 100]))
            .expect("non-critical overflow is an outcome");

        assert_eq!(outcome, EnqueueOutcome::DroppedNonCritical);
    }

    #[test]
    fn bounded_enqueue_reports_critical_drop_as_error() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, _rx) = bounded(1);
        tx.try_send(frame(&[0x90, 60, 100])).expect("prefill");

        let error = try_enqueue_midi_frame(&tx, frame(&[0x80, 60, 0]))
            .expect_err("critical release must not look successfully queued");

        assert_eq!(error, EnqueueError::CriticalReleaseDropped);
        assert!(take_critical_midi_reset_request());
        assert!(!take_critical_midi_reset_request());
    }
}
