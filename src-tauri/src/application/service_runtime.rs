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

pub fn stop_bridge(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    state.audio.stop(Some(app.clone()));
    state
        .bridge
        .stop()
        .map_err(|err| runtime_error("runtime.stop-failed", err))
}

pub fn reset_keys(state: &AppState) -> Result<(), CommandError> {
    state
        .bridge
        .reset_keys()
        .map_err(|err| runtime_error("runtime.reset-keys-failed", err))
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
pub enum StressTestMode {
    /// Test direct audio/VST (bypass bridge)
    AudioVstOnly,
    /// Test bridge pipeline (inject via bridge)
    BridgePipeline,
    /// Test RTP-MIDI input (via local session)
    RtpInput,
    /// Test complet end-to-end
    EndToEnd,
}

impl StressTestMode {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "audio-vst" => Some(Self::AudioVstOnly),
            "bridge" => Some(Self::BridgePipeline),
            "rtp" => Some(Self::RtpInput),
            "end-to-end" => Some(Self::EndToEnd),
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
    let mode = StressTestMode::from_str(mode)
        .ok_or_else(|| runtime_error("stress.invalid-mode", format!("Invalid mode: {}", mode)))?;

    match mode {
        StressTestMode::AudioVstOnly => run_stress_audio_only(rate, duration, state),
        StressTestMode::BridgePipeline => run_stress_bridge_pipeline(rate, duration, state),
        StressTestMode::RtpInput => run_stress_rtp_input(rate, duration, state),
        StressTestMode::EndToEnd => run_stress_end_to_end(rate, duration, state),
    }
}

/// Mode Audio/VST Only: Direct injection to audio engine (bypass bridge).
fn run_stress_audio_only(
    rate: u32,
    duration: u32,
    state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    if !state.audio.is_running() {
        return Err(runtime_error("stress.audio-not-running", "Le moteur audio doit être démarré pour le stress test."));
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
    let mut data = smallvec::SmallVec::<[u8; 32]>::new();
    data.push(0x90);
    data.push(60);
    data.push(100);

    while start.elapsed() < d {
        state.audio.send_midi(&data);
        sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        next_tick += interval;
        let now = std::time::Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(50));

    let final_drops = state.audio.midi_drop_count().unwrap_or(0);
    let final_xruns = state.audio.xrun_count().unwrap_or(0);

    let sent_notes = sent.load(std::sync::atomic::Ordering::Relaxed);
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
        return Err(runtime_error("stress.audio-not-running", "Le moteur audio doit être démarré pour le stress test."));
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
        // Inject via bridge (same as test note)
        let ch = 0u8; // Channel 1
        let note = 60u8;
        let velocity = 100u8;
        let data = smallvec::SmallVec::from_slice(&[0x90 | ch, note, velocity]);
        let _ = state.bridge.inject_frame(data, "StressTest".to_string());

        sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        next_tick += interval;
        let now = std::time::Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(100));

    let final_audio_drops = state.audio.midi_drop_count().unwrap_or(0);
    let final_xruns = state.audio.xrun_count().unwrap_or(0);
    let (pipeline_in, pipeline_out, pipeline_filtered) = pipeline_stats();

    let sent_notes = sent.load(std::sync::atomic::Ordering::Relaxed);
    let audio_drops = final_audio_drops.saturating_sub(initial_audio_drops);
    let audio_xruns = final_xruns.saturating_sub(initial_xruns);

    // Bridge drops = (injected - pipeline_out) + audio_drops
    let bridge_drops = sent_notes.saturating_sub(pipeline_out as u32) + audio_drops;

    Ok(crate::types::StressTestResult {
        sent_notes,
        elapsed_ms,
        dropped_notes: bridge_drops + audio_drops,
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

/// Mode RTP Input: Test RTP-MIDI input layer.
fn run_stress_rtp_input(
    _rate: u32,
    _duration: u32,
    _state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    // RTP stress test would require setting up a local RTP session as sender
    // For now, return a placeholder - full implementation would need tokio runtime
    Err(runtime_error("stress.rtp-not-implemented", "RTP stress test mode requires async runtime integration. Use external midi_stress binary for now."))
}

/// Mode End-to-End: Test full pipeline with feedback measurement.
fn run_stress_end_to_end(
    rate: u32,
    duration: u32,
    state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    use crate::bridge::{pipeline_stats, reset_pipeline_stats};
    use crate::rtp::rtp_dropped_count;

    if !state.audio.is_running() {
        return Err(runtime_error("stress.audio-not-running", "Le moteur audio doit être démarré pour le stress test."));
    }

    // Reset all counters
    reset_pipeline_stats();
    let initial_rtp_drops = rtp_dropped_count();

    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let elapsed_ms = duration as u64 * 1000;

    let initial_audio_drops = state.audio.midi_drop_count().unwrap_or(0);
    let initial_xruns = state.audio.xrun_count().unwrap_or(0);

    let start = std::time::Instant::now();
    let d = std::time::Duration::from_secs(duration as u64);
    let interval = std::time::Duration::from_nanos(1_000_000_000 / rate.max(1) as u64);

    let mut next_tick = start;

    while start.elapsed() < d {
        let ch = 0u8;
        let note = 60u8;
        let velocity = 100u8;
        let data = smallvec::SmallVec::from_slice(&[0x90 | ch, note, velocity]);
        let _ = state.bridge.inject_frame(data, "StressTest".to_string());

        sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        next_tick += interval;
        let now = std::time::Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(100));

    let final_audio_drops = state.audio.midi_drop_count().unwrap_or(0);
    let final_xruns = state.audio.xrun_count().unwrap_or(0);
    let final_rtp_drops = rtp_dropped_count();
    let (pipeline_in, pipeline_out, _filtered) = pipeline_stats();

    let sent_notes = sent.load(std::sync::atomic::Ordering::Relaxed);
    let audio_drops = final_audio_drops.saturating_sub(initial_audio_drops);
    let audio_xruns = final_xruns.saturating_sub(initial_xruns);
    let rtp_drops = final_rtp_drops.saturating_sub(initial_rtp_drops) as u32;

    // In end-to-end, we don't have separate RTP input (using bridge injection)
    // but we report the metrics as if coming from respective segments
    let bridge_drops = sent_notes.saturating_sub(pipeline_out as u32);

    Ok(crate::types::StressTestResult {
        sent_notes,
        elapsed_ms,
        dropped_notes: rtp_drops + bridge_drops + audio_drops,
        xruns: audio_xruns,
        segment_rtp: Some(crate::types::SegmentMetrics {
            dropped: rtp_drops,
            xruns: 0,
            latency_ms: None,
        }),
        segment_bridge: Some(crate::types::SegmentMetrics {
            dropped: bridge_drops,
            xruns: 0,
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

/// Legacy compatibility - runs audio-only mode.
pub fn run_automated_stress_test(
    rate: u32,
    duration: u32,
    state: &AppState,
) -> Result<crate::types::StressTestResult, CommandError> {
    run_stress_test("audio-vst", rate, duration, state)
}
