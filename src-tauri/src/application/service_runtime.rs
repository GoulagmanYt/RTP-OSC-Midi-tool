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
