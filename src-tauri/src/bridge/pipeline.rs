use crate::{
    audio::AudioEngine,
    config::{Config, RoutingAssignment, RoutingMapping, RoutingProfile},
    logger::{logs_enabled, should_log_debug, FrontendLogger},
    midi::{parse_note, parse_sustain, MidiFrame, MidiKind},
    osc::OscClient,
    types::MidiNoteEvent,
};
use midir::MidiOutputConnection;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};
use tauri::Emitter;

use super::activity::now_ms;

const MIDI_NOTE_EVENT: &str = "midi:note";
const OSC_SUSTAIN_PARAM: &str = "/avatar/parameters/sustain";

// Compteurs statiques pour les métriques de pipeline (stress test)
static PIPELINE_IN_COUNT: AtomicU64 = AtomicU64::new(0);
static PIPELINE_OUT_COUNT: AtomicU64 = AtomicU64::new(0);
static PIPELINE_FILTERED_COUNT: AtomicU64 = AtomicU64::new(0);

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

#[derive(Clone)]
pub(super) struct ConfigSnapshot {
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
            osc_target_ip: cfg.osc.target_ip.clone(),
            osc_target_port: cfg.osc.target_port,
            osc_enabled: cfg.osc.enabled,
            channel_filter: cfg.midi.channel_filter,
            midi_thru: cfg.midi.thru_enabled,
            log_osc: cfg.osc.log_messages,
            verbose: cfg.logging.verbose,
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
) {
    // Compteur entrée pipeline (stress test metrics)
    PIPELINE_IN_COUNT.fetch_add(1, Ordering::Relaxed);

    let mut frame = frame;
    let mut filtered = false;

    if let Some(profile_idx) = config.routing_assignments.get(&*frame.source).copied() {
        if let Some(profile) = config.routing_profiles.get(profile_idx) {
            if profile.enabled && !apply_routing_profile(profile, &mut frame) {
                PIPELINE_FILTERED_COUNT.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
    }

    if let Some(status) = frame.data.first() {
        if *status >= 0xF8 {
            // Messages temps-réel système (clock, etc) - pas des notes MIDI
            // Ne pas compter comme filtré, juste ignorés pour le pipeline audio
            filtered = true;
        }
    }

    if config.verbose && should_log_debug() && frame.source.starts_with("RTP:") {
        logger.debug(format!(
            "MIDI IN (RTP) -> pipeline: {:02X?}",
            frame.data.as_slice()
        ));
    }

    if let Some(status) = frame.data.first() {
        let is_channel_voice = (0x80..0xF0).contains(status);
        if is_channel_voice {
            audio.send_midi(frame.data.as_slice());
            PIPELINE_OUT_COUNT.fetch_add(1, Ordering::Relaxed);
            if config.verbose && should_log_debug() {
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

    if config.midi_thru {
        if let Some(out) = midi_out.as_mut() {
            let _ = out.send(frame.data.as_slice());
        }
    }

    if let Some(sustain) = parse_sustain(frame.data.as_slice()) {
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

        if let Some(filter) = config.channel_filter {
            if note.channel != filter {
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

/// Retourne les statistiques de pipeline (in, out, filtered).
pub fn pipeline_stats() -> (u64, u64, u64) {
    (
        PIPELINE_IN_COUNT.load(Ordering::Relaxed),
        PIPELINE_OUT_COUNT.load(Ordering::Relaxed),
        PIPELINE_FILTERED_COUNT.load(Ordering::Relaxed),
    )
}

/// Réinitialise les compteurs de pipeline (utile pour les tests).
pub fn reset_pipeline_stats() {
    PIPELINE_IN_COUNT.store(0, Ordering::Relaxed);
    PIPELINE_OUT_COUNT.store(0, Ordering::Relaxed);
    PIPELINE_FILTERED_COUNT.store(0, Ordering::Relaxed);
}
