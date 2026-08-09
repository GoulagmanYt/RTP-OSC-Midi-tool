use midir::{MidiInput, MidiOutput};
use tauri::{AppHandle, Window};

use crate::{
    config::{Config, RTP_VIRTUAL_INPUT, VST_INTERNAL_OUTPUT, VST_INTERNAL_OUTPUT_LEGACY},
    error::CommandError,
    logger::{set_log_all_to_file, set_logs_enabled},
    rtp,
    tauri::{
        state::AppState,
        utils::{
            audio_settings_from_config, fallback_vst_path, sync_rtp_discovery, sync_runtime_logging,
        },
    },
    types::{BridgeStatus, PreflightReport, RtpSessionInfo},
};

use super::service_common::{audio_error, frontend_logger, runtime_error, with_audio_status};

pub fn list_midi_inputs() -> Result<Vec<String>, CommandError> {
    let input = MidiInput::new("OSCMidi")
        .map_err(|err| runtime_error("midi.list-inputs-failed", err.to_string()))?;
    let mut list: Vec<String> = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();
    list.insert(0, RTP_VIRTUAL_INPUT.to_string());
    Ok(list)
}

pub fn list_midi_outputs() -> Result<Vec<String>, CommandError> {
    let output = MidiOutput::new("OSCMidi")
        .map_err(|err| runtime_error("midi.list-outputs-failed", err.to_string()))?;
    let mut list = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<String>>();
    list.push(VST_INTERNAL_OUTPUT.to_string());
    Ok(list)
}

pub fn start_bridge(
    app: &AppHandle,
    window: &Window,
    config: Config,
    state: &AppState,
) -> Result<BridgeStatus, CommandError> {
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.logging.log_all_to_file);
    set_logs_enabled(config.logging.enabled);
    state
        .config_store
        .save(&config)
        .map_err(|err| super::service_common::config_error("config.save-failed", err))?;
    sync_rtp_discovery(&config, state, app);
    let status = state
        .bridge
        .start(
            window.clone(),
            config.clone(),
            state.dev_logging.clone(),
            state.audio.clone(),
        )
        .map_err(|err| runtime_error("runtime.start-failed", err))?;
    if config.audio.enabled {
        let logger = frontend_logger(window, state);
        let fallback_vst = fallback_vst_path(app);
        if let Err(err) = state.audio.start(
            audio_settings_from_config(&config),
            fallback_vst,
            logger.clone(),
        ) {
            logger.error(format!("Audio not started: {err}"));
        }
    }
    Ok(with_audio_status(status, &state.audio))
}

pub async fn stop_bridge(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    let app = app.clone();
    let audio = state.audio.clone();
    let bridge = state.bridge.clone();
    ::tauri::async_runtime::spawn_blocking(move || {
        // Stop every MIDI producer before removing the audio consumer. This
        // also keeps all blocking thread joins away from Tokio's async worker.
        let bridge_result = bridge.stop();
        audio.stop(Some(app));
        bridge_result.map_err(|err| runtime_error("runtime.stop-failed", err))
    })
    .await
    .map_err(|error| {
        runtime_error(
            "runtime.stop-task-failed",
            format!("Bridge stop task failed: {error}"),
        )
    })?
}

pub fn panic_midi(state: &AppState) -> Result<(), CommandError> {
    state
        .audio
        .panic_all_notes()
        .map_err(|err| audio_error("audio.panic-failed", err.to_string()))?;
    state
        .bridge
        .reset_keys()
        .map_err(|err| runtime_error("runtime.reset-keys-failed", err))
}

pub fn send_test_midi(
    kind: String,
    channel: u8,
    note: u8,
    velocity: u8,
    cc: u8,
    value: u8,
    state: &AppState,
) -> Result<(), CommandError> {
    match kind.as_str() {
        "note" => state
            .bridge
            .send_test_note(note, velocity, channel)
            .map_err(|err| runtime_error("runtime.send-test-note-failed", err)),
        "cc" => state
            .bridge
            .send_test_cc(cc, value, channel)
            .map_err(|err| runtime_error("runtime.send-test-cc-failed", err)),
        _ => Err(runtime_error(
            "runtime.invalid-test-midi-kind",
            "Unknown test MIDI kind (note|cc)",
        )),
    }
}

pub fn get_status(window: &Window, state: &AppState) -> BridgeStatus {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    let logger = frontend_logger(window, state);
    if let Err(err) = state.bridge.sync_rtp(&cfg, &logger, false) {
        logger.error(format!("RTP not started: {err}"));
    }
    let status = state.bridge.status(&cfg);
    with_audio_status(status, &state.audio)
}

pub fn restart_rtp(window: &Window, state: &AppState) -> Result<BridgeStatus, CommandError> {
    let cfg = state.config_store.load();
    let logger = frontend_logger(window, state);
    state
        .bridge
        .restart_rtp(&cfg, &logger)
        .map_err(|err| runtime_error("runtime.restart-rtp-failed", err))?;
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}

pub fn refresh_rtp_sessions(
    app: &AppHandle,
    state: &AppState,
) -> Result<Vec<RtpSessionInfo>, CommandError> {
    state
        .rtp_discovery
        .refresh_now(app, true)
        .map_err(|err| runtime_error("rtp.refresh-sessions-failed", err))
}

pub fn preflight_check(state: &AppState) -> Result<PreflightReport, CommandError> {
    let cfg = state.config_store.load();
    let mut messages = Vec::new();

    let midi_inputs = list_midi_inputs()?;
    let midi_in_ok = cfg
        .midi
        .input_device
        .as_ref()
        .map(|name| {
            if name == RTP_VIRTUAL_INPUT {
                true
            } else {
                midi_inputs.iter().any(|p| p.contains(name))
            }
        })
        .unwrap_or(false);
    if !midi_in_ok {
        messages.push("MIDI IN manquant ou non configure".to_string());
    }

    let midi_outputs = list_midi_outputs()?;
    let midi_out_ok = cfg
        .midi
        .output_device
        .as_ref()
        .map(|name| {
            if name == VST_INTERNAL_OUTPUT || name == VST_INTERNAL_OUTPUT_LEGACY {
                true
            } else {
                midi_outputs.iter().any(|p| p.contains(name))
            }
        })
        .unwrap_or(false);
    if !midi_out_ok {
        messages.push("MIDI OUT manquant ou non configure".to_string());
    }

    let backends = state.audio.list_backends();
    let audio_backend_ok = if !cfg.audio.enabled {
        true
    } else if let Some(backend) = cfg.audio.backend.as_ref() {
        if backend.to_lowercase() == "auto" {
            !backends.is_empty()
        } else {
            backends.iter().any(|b| b.eq_ignore_ascii_case(backend))
        }
    } else {
        !backends.is_empty()
    };
    if !audio_backend_ok {
        messages.push(format!(
            "Backend audio indisponible ({})",
            cfg.audio.backend.clone().unwrap_or_default()
        ));
    }

    let rtp_needed = cfg.rtp.enabled
        || cfg.rtp.remote_enabled
        || cfg
            .midi
            .input_device
            .as_ref()
            .map(|s| s == RTP_VIRTUAL_INPUT)
            .unwrap_or(false);
    let rtp_port_ok = if rtp_needed {
        if state.bridge.owns_rtp_port(cfg.rtp.port) {
            true
        } else {
            rtp::ports_available(cfg.rtp.port)
                .map_err(|err| runtime_error("rtp.port-check-failed", err))?
        }
    } else {
        true
    };
    if !rtp_port_ok {
        messages.push(format!("Port RTP {} deja utilise", cfg.rtp.port));
    }

    Ok(PreflightReport {
        midi_in_ok,
        midi_out_ok,
        audio_backend_ok,
        rtp_port_ok,
        messages,
    })
}

#[derive(Debug, Clone, Copy)]
enum StressTestMode {
    AudioVstOnly,
    BridgePipeline,
}

const MAX_STRESS_RATE_MESSAGES_PER_SECOND: u32 = 10_000;
const MAX_STRESS_DURATION_SECONDS: u32 = 30;

impl StressTestMode {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "audio-vst" => Some(Self::AudioVstOnly),
            "bridge" => Some(Self::BridgePipeline),
            _ => None,
        }
    }
}

/// Run stress test with selected mode to isolate pipeline segment losses.
pub fn run_stress_test(
    mode: &str,
    rate: u32,
    duration: u32,
    state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    if rate == 0 || rate > MAX_STRESS_RATE_MESSAGES_PER_SECOND {
        return Err(runtime_error(
            "stress.invalid-rate",
            format!(
                "Stress-test rate must be between 1 and {MAX_STRESS_RATE_MESSAGES_PER_SECOND} MIDI messages/s"
            ),
        ));
    }
    if duration == 0 || duration > MAX_STRESS_DURATION_SECONDS {
        return Err(runtime_error(
            "stress.invalid-duration",
            format!(
                "Stress-test duration must be between 1 and {MAX_STRESS_DURATION_SECONDS} seconds"
            ),
        ));
    }

    let mode = StressTestMode::from_str(mode)
        .ok_or_else(|| runtime_error("stress.invalid-mode", format!("Invalid mode: {}", mode)))?;

    match mode {
        StressTestMode::AudioVstOnly => run_stress_audio_only(rate, duration, state),
        StressTestMode::BridgePipeline => run_stress_bridge_pipeline(rate, duration, state),
    }
}

/// Mode Audio/VST Only: Direct injection to audio engine (bypass bridge).
fn run_stress_audio_only(
    rate: u32,
    duration: u32,
    state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    if !state.audio.is_running() {
        return Err(runtime_error(
            "stress.audio-not-running",
            "Le moteur audio doit être démarré pour le stress test.",
        ));
    }

    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let elapsed_ms = duration as u64 * 1000;

    // Reset and read initial counters
    let initial_drops = state.audio.midi_drop_count().unwrap_or(0);
    let initial_xruns = state.audio.xrun_count().unwrap_or(0);

    let start = std::time::Instant::now();
    let d = std::time::Duration::from_secs(duration as u64);
    let interval = std::time::Duration::from_nanos(1_000_000_000 / rate.max(1) as u64);

    let mut next_tick = start;

    while start.elapsed() < d {
        let sequence = sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let data = stress_midi_message(sequence);
        state.audio.send_midi(&data);

        next_tick += interval;
        let now = std::time::Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
    }

    let sent_notes = sent.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(note_off) = stress_trailing_note_off(sent_notes) {
        state.audio.send_midi(&note_off);
    }
    wait_for_stress_queues(&state.audio, false);

    let final_drops = state.audio.midi_drop_count().unwrap_or(0);
    let final_xruns = state.audio.xrun_count().unwrap_or(0);

    let audio_drops = final_drops.saturating_sub(initial_drops);
    let audio_xruns = final_xruns.saturating_sub(initial_xruns);

    Ok(crate::types::StressTestResult {
        sent_notes,
        elapsed_ms,
        dropped_notes: audio_drops,
        xruns: audio_xruns,
        segment_rtp: None,
        segment_bridge: None,
        segment_audio: Some(crate::types::SegmentMetrics {
            dropped: audio_drops,
            xruns: audio_xruns,
            latency_ms: state.audio.current_latency_ms(),
        }),
        received_notes: None,
    })
}

/// Mode Bridge Pipeline: Inject via bridge and measure pipeline metrics.
fn run_stress_bridge_pipeline(
    rate: u32,
    duration: u32,
    state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    use crate::bridge::{pipeline_stats, reset_pipeline_stats};

    if !state.audio.is_running() {
        return Err(runtime_error(
            "stress.audio-not-running",
            "Le moteur audio doit être démarré pour le stress test.",
        ));
    }

    // Reset pipeline counters
    reset_pipeline_stats();

    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let elapsed_ms = duration as u64 * 1000;

    // Read initial audio counters
    let initial_audio_drops = state.audio.midi_drop_count().unwrap_or(0);
    let initial_xruns = state.audio.xrun_count().unwrap_or(0);

    let start = std::time::Instant::now();
    let d = std::time::Duration::from_secs(duration as u64);
    let interval = std::time::Duration::from_nanos(1_000_000_000 / rate.max(1) as u64);

    let mut next_tick = start;

    while start.elapsed() < d {
        let sequence = sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let data = stress_midi_message(sequence);
        let _ = state.bridge.inject_frame(data, "StressTest".to_string());

        next_tick += interval;
        let now = std::time::Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
    }

    let sent_notes = sent.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(note_off) = stress_trailing_note_off(sent_notes) {
        let _ = state
            .bridge
            .inject_frame(note_off, "StressTest:cleanup".to_string());
    }
    wait_for_stress_queues(&state.audio, true);

    let final_audio_drops = state.audio.midi_drop_count().unwrap_or(0);
    let final_xruns = state.audio.xrun_count().unwrap_or(0);
    let (_pipeline_in, pipeline_out, _pipeline_filtered) = pipeline_stats();

    let audio_drops = final_audio_drops.saturating_sub(initial_audio_drops);
    let audio_xruns = final_xruns.saturating_sub(initial_xruns);

    // Bridge drops = messages that did not reach the audio segment plus audio drops.
    let bridge_drops = sent_notes.saturating_sub(pipeline_out as u32) + audio_drops;

    Ok(crate::types::StressTestResult {
        sent_notes,
        elapsed_ms,
        dropped_notes: bridge_drops,
        xruns: audio_xruns,
        segment_rtp: None,
        segment_bridge: Some(crate::types::SegmentMetrics {
            dropped: bridge_drops,
            xruns: 0, // Bridge doesn't generate xruns
            latency_ms: None,
        }),
        segment_audio: Some(crate::types::SegmentMetrics {
            dropped: audio_drops,
            xruns: audio_xruns,
            latency_ms: state.audio.current_latency_ms(),
        }),
        received_notes: Some(pipeline_out as u32),
    })
}

fn stress_midi_message(sequence: u32) -> smallvec::SmallVec<[u8; 32]> {
    let note = 36 + ((sequence / 2) % 48) as u8;
    if sequence.is_multiple_of(2) {
        smallvec::SmallVec::from_slice(&[0x90, note, 100])
    } else {
        smallvec::SmallVec::from_slice(&[0x80, note, 0])
    }
}

fn stress_trailing_note_off(sent_messages: u32) -> Option<smallvec::SmallVec<[u8; 32]>> {
    if sent_messages.is_multiple_of(2) {
        return None;
    }
    let note = 36 + (((sent_messages - 1) / 2) % 48) as u8;
    Some(smallvec::SmallVec::from_slice(&[0x80, note, 0]))
}

fn wait_for_stress_queues(audio: &crate::audio::AudioEngine, include_bridge: bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let audio_depth = audio.audio_midi_queue_depth().unwrap_or(0);
        let bridge_depth = if include_bridge {
            crate::bridge::pipeline::pipeline_metrics_snapshot().queue_depth
        } else {
            0
        };
        if audio_depth == 0 && bridge_depth == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Give the callback one more period to publish final deadline/drop metrics.
    std::thread::sleep(std::time::Duration::from_millis(20));
}

#[cfg(test)]
mod tests {
    use super::{stress_midi_message, stress_trailing_note_off};

    #[test]
    fn stress_messages_are_balanced_note_pairs() {
        for pair in 0..96 {
            let note_on = stress_midi_message(pair * 2);
            let note_off = stress_midi_message(pair * 2 + 1);
            assert_eq!(note_on[0], 0x90);
            assert_eq!(note_off[0], 0x80);
            assert_eq!(note_on[1], note_off[1]);
            assert!((36..84).contains(&note_on[1]));
        }
        assert!(stress_trailing_note_off(100).is_none());
        assert_eq!(stress_trailing_note_off(101).unwrap()[0], 0x80);
    }

    #[test]
    fn bridge_stress_does_not_double_count_audio_drops() {
        let source = include_str!("service_runtime.rs");

        let forbidden = ["dropped_notes: bridge_drops", " + audio_drops"].concat();
        assert!(
            !source.contains(&forbidden),
            "bridge stress totals must not add audio drops twice"
        );
        assert!(
            source.contains("dropped_notes: bridge_drops"),
            "bridge stress total should be the bridge segment total"
        );
    }
}
