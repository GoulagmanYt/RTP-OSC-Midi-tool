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

fn profile_for_source<'a>(
    s: &'a ConfigSnapshot,
    source: &str,
) -> Option<&'a RoutingProfileRuntime> {
    let index = s.routing_assignments.get(source).or_else(|| {
        let rest = source.strip_prefix("RTP:")?;
        let cut = rest.rfind("#ssrc_").or_else(|| rest.rfind('@'))?;
        s.routing_assignments.get(&source[..4 + cut])
    })?;
    s.routing_profiles.get(*index)
}

pub(super) fn prepare_input_routing(tx: &Sender<MidiFrame>, cfg: &Config) {
    let snapshot = ConfigSnapshot::from(cfg);
    tx.set_preparation(move |frame| {
        accepts_channel(snapshot.channel_filter, &frame.data)
            && profile_for_source(&snapshot, &frame.source)
                .filter(|profile| profile.enabled)
                .is_none_or(|profile| apply_routing_profile(profile, frame))
    });
}

fn accepts_channel(filter: Option<u8>, data: &[u8]) -> bool {
    !data.first().is_some_and(|status| {
        (0x80..0xF0).contains(status) && filter.is_some_and(|channel| (status & 15) + 1 != channel)
    })
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

/// OSC parameters combine every accepted source and channel into one switch.
#[derive(Default)]
pub(super) struct OscOutputState {
    sources: HashMap<std::sync::Arc<str>, OscSourceState>,
    sent_notes: u128,
    sent_sustain: bool,
}

#[derive(Default)]
struct OscSourceState {
    notes: [u128; 16],
    sustain: [bool; 16],
}

impl OscOutputState {
    pub(super) fn reset_sent(&mut self) {
        self.sent_notes = 0;
        self.sent_sustain = false;
    }

    pub(super) fn clear(&mut self) {
        self.sources.clear();
        self.reset_sent();
    }

    fn observe(&mut self, frame: &MidiFrame) {
        let Some(&status) = frame.data.first() else {
            return;
        };
        if frame.data.as_slice() == [0xFF] {
            self.sources.clear();
            return;
        }
        if !(0x80..0xF0).contains(&status) {
            return;
        }
        if frame.source.as_ref() == "ReliablePLV:server-stop"
            && status & 0xF0 == 0xB0
            && frame.data.len() == 3
            && matches!(frame.data[1], 120 | 123)
        {
            self.sources
                .retain(|key, _| !key.starts_with("ReliablePLV:"));
            return;
        }
        let channel = usize::from(status & 15);
        let source = self.sources.entry(frame.source.clone()).or_default();
        if let Some(note) = parse_note(&frame.data) {
            if let Some(index) = note.index {
                let mask = 1u128 << (index - 1);
                if matches!(note.kind, MidiKind::NoteOn) {
                    source.notes[channel] |= mask;
                } else {
                    source.notes[channel] &= !mask;
                }
            }
        } else if let Some(sustain) = parse_sustain(&frame.data) {
            source.sustain[channel] = sustain.pressed;
        } else if status & 0xF0 == 0xB0 && frame.data.len() == 3 {
            match frame.data[1] {
                120 => {
                    source.notes[channel] = 0;
                    source.sustain[channel] = false;
                }
                123 => source.notes[channel] = 0,
                121 => source.sustain[channel] = false,
                _ => {}
            }
        }
        if source.notes.iter().all(|notes| *notes == 0)
            && source.sustain.iter().all(|pressed| !pressed)
        {
            self.sources.remove(&frame.source);
        }
    }

    /// Commit each switch only after a successful send so failures remain retryable.
    pub(super) fn flush(
        &mut self,
        mut send: impl FnMut(Option<u8>, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let notes = self
            .sources
            .values()
            .flat_map(|source| source.notes)
            .fold(0, |all, notes| all | notes);
        let sustain = self
            .sources
            .values()
            .any(|source| source.sustain.iter().any(|pressed| *pressed));
        for index in 1..=crate::midi::NOTE_MAX - crate::midi::NOTE_MIN + 1 {
            let mask = 1u128 << (index - 1);
            let pressed = notes & mask != 0;
            if pressed != (self.sent_notes & mask != 0) {
                send(Some(index), pressed)?;
                if pressed {
                    self.sent_notes |= mask;
                } else {
                    self.sent_notes &= !mask;
                }
            }
        }
        if sustain != self.sent_sustain {
            send(None, sustain)?;
            self.sent_sustain = sustain;
        }
        Ok(())
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
    osc_state: &mut OscOutputState,
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

    if !accepts_channel(config.channel_filter, &frame.data) {
        if deliver_audio {
            PIPELINE_FILTERED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        return;
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

    if let Some(event) = midi_note_event(&frame) {
        let _ = midi_event_tx.try_send(event);
    }
    if !config.osc_enabled || frame.source.starts_with("OSC:") {
        return;
    }
    osc_state.observe(&frame);
    if let Some(client) = osc {
        if let Err(error) = osc_state.flush(|index, pressed| {
            let result = match index {
                Some(index) => client.send_note(index, pressed),
                None => client.send_sustain(pressed),
            };
            if result.is_err() && !pressed {
                if let Some(recovery) = route_recovery {
                    recovery.request_reset();
                } else {
                    request_critical_midi_reset();
                }
            }
            result?;
            osc_counter.fetch_add(1, Ordering::Relaxed);
            if config.log_osc && logs_enabled() {
                let parameter = index
                    .map(crate::osc::parameter_name)
                    .unwrap_or_else(|| OSC_SUSTAIN_PARAM.to_owned());
                logger.info(format!("{parameter} -> {}", i32::from(pressed)));
            }
            Ok(())
        }) {
            if logs_enabled() {
                logger.error(format!("Envoi OSC echoue: {error}"));
            }
        }
    }
}

fn midi_note_event(frame: &MidiFrame) -> Option<MidiNoteEvent> {
    let note = parse_note(&frame.data)?;
    Some(MidiNoteEvent {
        source: frame.source.to_string(),
        note: note.note,
        channel: note.channel,
        pressed: matches!(note.kind, MidiKind::NoteOn),
        timestamp_ms: now_ms(),
    })
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

    #[test]
    fn global_channel_filter_applies_at_admission_and_preserves_system_messages() {
        let mut config = Config::default();
        config.midi.channel_filter = Some(1);
        let (tx, rx) = bounded(32);
        prepare_input_routing(&tx, &config);
        for data in [
            &[0x91, 60, 100][..],
            &[0x81, 60, 0],
            &[0xB1, 64, 127],
            &[0xC1, 10],
        ] {
            assert_eq!(
                try_enqueue_midi_frame(&tx, frame(data)).unwrap(),
                EnqueueOutcome::Filtered
            );
        }
        for data in [
            &[0x90, 60, 100][..],
            &[0x80, 60, 0],
            &[0xF8],
            &[0xF0, 1, 0xF7],
            &[0xFF],
        ] {
            assert_eq!(
                try_enqueue_midi_frame(&tx, frame(data)).unwrap(),
                EnqueueOutcome::Enqueued
            );
            assert_eq!(rx.recv().unwrap().data.as_slice(), data);
        }
    }

    #[test]
    fn successful_reset_resends_notes_observed_during_network_failure() {
        let mut state = OscOutputState::default();
        let mut sent = Vec::new();
        observe_osc(&mut state, "test", &[0x90, 60, 100], &mut sent);
        state.observe(&frame(&[0x90, 61, 100]));
        state.reset_sent();
        state
            .flush(|index, pressed| {
                sent.push((index, pressed));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            sent,
            vec![(Some(40), true), (Some(40), true), (Some(41), true)]
        );
    }

    #[test]
    fn ui_events_cover_the_entire_midi_note_range_without_extending_osc_parameters() {
        for pitch in 0..=127 {
            for (status, velocity, pressed) in
                [(0x9F, 100, true), (0x8F, 0, false), (0x9F, 0, false)]
            {
                let event = midi_note_event(&frame(&[status, pitch, velocity])).unwrap();
                assert_eq!(
                    (event.note, event.channel, event.pressed),
                    (pitch, 16, pressed)
                );
            }
        }
        let mut state = OscOutputState::default();
        let mut sent = Vec::new();
        observe_osc(&mut state, "test", &[0x90, 0, 100], &mut sent);
        observe_osc(&mut state, "test", &[0x90, 127, 100], &mut sent);
        assert!(sent.is_empty());
    }

    fn observe_osc(
        state: &mut OscOutputState,
        source: &str,
        data: &[u8],
        sent: &mut Vec<(Option<u8>, bool)>,
    ) {
        let mut input = frame(data);
        input.source = Arc::from(source);
        state.observe(&input);
        state
            .flush(|index, pressed| {
                sent.push((index, pressed));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn osc_sustain_aggregates_channels_and_sources() {
        let mut state = OscOutputState::default();
        let mut sent = Vec::new();
        observe_osc(&mut state, "a", &[0xB0, 64, 127], &mut sent);
        observe_osc(&mut state, "a", &[0xB1, 64, 0], &mut sent);
        observe_osc(&mut state, "a", &[0xB0, 64, 127], &mut sent);
        assert_eq!(sent, vec![(None, true)]);
        observe_osc(&mut state, "b", &[0xB0, 64, 127], &mut sent);
        observe_osc(&mut state, "a", &[0xB0, 121, 0], &mut sent);
        assert_eq!(sent, vec![(None, true)]);
        observe_osc(&mut state, "b", &[0xB0, 121, 0], &mut sent);
        assert_eq!(sent, vec![(None, true), (None, false)]);
    }

    #[test]
    fn reliable_playback_stop_and_disconnect_release_osc_notes_and_sustain() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;
        for disconnect in [false, true] {
            let (tx, rx) = bounded(256);
            let server =
                crate::reliable_playback::ReliablePlaybackServer::start("127.0.0.1:0", tx).unwrap();
            let mut stream = TcpStream::connect(server.local_addr()).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let send = |stream: &mut TcpStream, payload: serde_json::Value| {
                let data = serde_json::to_vec(&payload).unwrap();
                stream
                    .write_all(&(data.len() as u32).to_be_bytes())
                    .unwrap();
                stream.write_all(&data).unwrap();
            };
            send(
                &mut stream,
                serde_json::json!({
                    "type": "prepare", "version": 1, "sessionId": "osc-release",
                    "song": "release.mid", "total": 3, "events": [
                        {"seq": 0, "dueUs": 0, "data": [0x90, 60, 100]},
                        {"seq": 1, "dueUs": 0, "data": [0xB0, 64, 127]},
                        {"seq": 2, "dueUs": 60_000_000, "data": [0x80, 60, 0]}
                    ]
                }),
            );
            let mut header = [0; 4];
            stream.read_exact(&mut header).unwrap();
            let mut response = vec![0; u32::from_be_bytes(header) as usize];
            stream.read_exact(&mut response).unwrap();
            send(
                &mut stream,
                serde_json::json!({"type": "start", "sessionId": "osc-release", "startDelayMs": 0}),
            );
            let mut state = OscOutputState::default();
            for _ in 0..2 {
                state.observe(&rx.recv_timeout(Duration::from_secs(2)).unwrap());
            }
            let mut sent = Vec::new();
            state
                .flush(|index, pressed| {
                    sent.push((index, pressed));
                    Ok(())
                })
                .unwrap();
            assert_eq!(sent, vec![(Some(40), true), (None, true)]);
            if !disconnect {
                send(
                    &mut stream,
                    serde_json::json!({"type": "stop", "sessionId": "osc-release"}),
                );
            }
            drop(stream);
            for _ in 0..32 {
                state.observe(&rx.recv_timeout(Duration::from_secs(2)).unwrap());
            }
            state
                .flush(|index, pressed| {
                    sent.push((index, pressed));
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                sent,
                vec![
                    (Some(40), true),
                    (None, true),
                    (Some(40), false),
                    (None, false)
                ]
            );
            assert!(state.sources.is_empty());
            server.stop();
        }
    }

    #[test]
    fn osc_channel_mode_releases_preserve_other_held_notes() {
        for controller in [120, 123] {
            let mut state = OscOutputState::default();
            let mut sent = Vec::new();
            observe_osc(&mut state, "a", &[0x90, 60, 100], &mut sent);
            observe_osc(&mut state, "a", &[0x91, 60, 100], &mut sent);
            observe_osc(&mut state, "b", &[0x90, 60, 100], &mut sent);
            observe_osc(&mut state, "a", &[0xB0, controller, 0], &mut sent);
            observe_osc(&mut state, "a", &[0x81, 60, 0], &mut sent);
            assert_eq!(sent, vec![(Some(40), true)]);
            observe_osc(&mut state, "b", &[0xB0, controller, 0], &mut sent);
            assert_eq!(sent, vec![(Some(40), true), (Some(40), false)]);
            assert!(state.sources.is_empty());
        }
    }

    #[test]
    fn osc_failed_release_remains_pending_and_clear_forgets_previous_route() {
        let mut state = OscOutputState::default();
        let mut sent = Vec::new();
        observe_osc(&mut state, "a", &[0x90, 60, 100], &mut sent);
        // Release the actual source, then fail the destination write.
        let mut off = frame(&[0x80, 60, 0]);
        off.source = Arc::from("a");
        state.observe(&off);
        assert!(state.flush(|_, _| Err("socket failure".into())).is_err());
        state
            .flush(|index, pressed| {
                sent.push((index, pressed));
                Ok(())
            })
            .unwrap();
        assert_eq!(sent, vec![(Some(40), true), (Some(40), false)]);
        observe_osc(&mut state, "a", &[0x90, 60, 100], &mut sent);
        state.clear();
        state
            .flush(|_, _| panic!("old route must have been reset"))
            .unwrap();
        observe_osc(&mut state, "a", &[0x90, 60, 100], &mut sent);
        assert_eq!(sent.last(), Some(&(Some(40), true)));
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

    #[test]
    fn rtp_routing_assignment_falls_back_from_peer_source_to_session_name() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, rx) = bounded(8);
        let mut config = Config::default();
        config.midi.routing_profiles.push(RoutingProfile {
            id: "rtp-filter".into(),
            name: "RTP Filter".into(),
            enabled: true,
            channel_filter: Some(1),
            note_min: Some(60),
            note_max: Some(72),
            cc_map: vec![RoutingMapping { from: 1, to: 2 }],
            program_map: vec![],
        });
        // UI assigns to session base name "RTP:OSCMidi"
        config.midi.routing_assignments.push(RoutingAssignment {
            source: "RTP:OSCMidi".into(),
            profile_id: "rtp-filter".into(),
        });
        prepare_input_routing(&tx, &config);

        // Frame from peer with SSRC qualified source
        let ssrc_frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0x90, 64, 100],
            source: Arc::from("RTP:OSCMidi#ssrc_00112233"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, ssrc_frame).unwrap(),
            EnqueueOutcome::Enqueued
        );
        let admitted = rx.recv().unwrap();
        assert_eq!(admitted.data.as_slice(), &[0x90, 64, 100]);

        // Note filtered out by profile note_max (75 > 72)
        let filtered_note = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0x90, 75, 100],
            source: Arc::from("RTP:OSCMidi#ssrc_00112233"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, filtered_note).unwrap(),
            EnqueueOutcome::Filtered
        );

        // CC remapped by profile (1 -> 2)
        let cc_frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0xB0, 1, 64],
            source: Arc::from("RTP:OSCMidi#ssrc_00112233"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, cc_frame).unwrap(),
            EnqueueOutcome::Enqueued
        );
        let admitted_cc = rx.recv().unwrap();
        assert_eq!(admitted_cc.data.as_slice(), &[0xB0, 2, 64]);

        // Backward compatibility: frame with address qualified source
        let addr_frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0x90, 65, 100],
            source: Arc::from("RTP:OSCMidi@127.0.0.1:5004"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, addr_frame).unwrap(),
            EnqueueOutcome::Enqueued
        );

        // Frame from different session does not match profile
        let other_session_frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0x90, 75, 100], // note 75 would be filtered if profile applied
            source: Arc::from("RTP:OtherSession#ssrc_00112233"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, other_session_frame).unwrap(),
            EnqueueOutcome::Enqueued
        );
    }

    #[test]
    fn rtp_routing_assignment_prefers_exact_peer_source_over_session_fallback() {
        let _guard = TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, rx) = bounded(8);
        let mut config = Config::default();
        config.midi.routing_profiles.push(RoutingProfile {
            id: "general-profile".into(),
            name: "General".into(),
            enabled: true,
            channel_filter: None,
            note_min: None,
            note_max: None,
            cc_map: vec![RoutingMapping { from: 10, to: 20 }],
            program_map: vec![],
        });
        config.midi.routing_profiles.push(RoutingProfile {
            id: "specific-peer-profile".into(),
            name: "Specific Peer".into(),
            enabled: true,
            channel_filter: None,
            note_min: None,
            note_max: None,
            cc_map: vec![RoutingMapping { from: 10, to: 99 }],
            program_map: vec![],
        });
        config.midi.routing_assignments.push(RoutingAssignment {
            source: "RTP:OSCMidi".into(),
            profile_id: "general-profile".into(),
        });
        config.midi.routing_assignments.push(RoutingAssignment {
            source: "RTP:OSCMidi#ssrc_11111111".into(),
            profile_id: "specific-peer-profile".into(),
        });
        prepare_input_routing(&tx, &config);

        // Specific peer uses specific profile (10 -> 99)
        let specific_frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0xB0, 10, 64],
            source: Arc::from("RTP:OSCMidi#ssrc_11111111"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, specific_frame).unwrap(),
            EnqueueOutcome::Enqueued
        );
        assert_eq!(rx.recv().unwrap().data.as_slice(), &[0xB0, 99, 64]);

        // Other peer in same session falls back to general profile (10 -> 20)
        let general_frame = MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![0xB0, 10, 64],
            source: Arc::from("RTP:OSCMidi#ssrc_22222222"),
        };
        assert_eq!(
            try_enqueue_midi_frame(&tx, general_frame).unwrap(),
            EnqueueOutcome::Enqueued
        );
        assert_eq!(rx.recv().unwrap().data.as_slice(), &[0xB0, 20, 64]);
    }

    #[test]
    fn osc_peer_source_releases_notes_cleanly() {
        let mut state = OscOutputState::default();
        let mut sent = Vec::new();
        let peer_src: Arc<str> = Arc::from("RTP:Session#ssrc_00112233");
        let mut note_on = frame(&[0x90, 60, 100]);
        note_on.source = peer_src.clone();
        state.observe(&note_on);
        state
            .flush(|index, pressed| {
                sent.push((index, pressed));
                Ok(())
            })
            .unwrap();
        assert_eq!(sent, vec![(Some(40), true)]);

        let mut note_off = frame(&[0x80, 60, 0]);
        note_off.source = peer_src;
        state.observe(&note_off);
        state
            .flush(|index, pressed| {
                sent.push((index, pressed));
                Ok(())
            })
            .unwrap();
        assert_eq!(sent, vec![(Some(40), true), (Some(40), false)]);
        assert!(state.sources.is_empty());
    }
}
