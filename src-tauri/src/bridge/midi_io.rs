use crate::{
    config::{Config, RTP_VIRTUAL_INPUT, VST_INTERNAL_OUTPUT, VST_INTERNAL_OUTPUT_LEGACY},
    logger::FrontendLogger,
    midi::MidiFrame,
};
use crossbeam_channel::Sender;
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub(super) fn watch_midi_input(
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
            let midi_changed = new_cfg.midi.input_device != cached_cfg.midi.input_device
                || new_cfg.midi.hotplug != cached_cfg.midi.hotplug;
            cached_cfg = new_cfg;
            seen_rev = rev_now;
            wait_for_config_change_only = false;
            if midi_changed && current_conn.is_some() {
                logger.info("Reconnexion MIDI IN (changement de peripherique)");
                current_conn = None;
            }
        }

        if cached_cfg
            .midi
            .input_device
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
                    if !cached_cfg.midi.hotplug {
                        wait_for_config_change_only = true;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}

pub(super) fn open_input(
    config: &Config,
    midi_tx: Sender<MidiFrame>,
    logger: &FrontendLogger,
) -> Result<(MidiInputConnection<Sender<MidiFrame>>, String), String> {
    let mut input = MidiInput::new("OSCMidi").map_err(|e| e.to_string())?;
    input.ignore(Ignore::TimeAndActiveSense);

    let ports = input.ports();
    let port = select_input_port(&input, &ports, config.midi.input_device.as_ref())
        .ok_or_else(|| "Aucune entree MIDI trouvee ou correspondante".to_string())?;

    let name = input
        .port_name(&port)
        .unwrap_or_else(|_| "MIDI IN".to_string());
    logger.info(format!("Entree MIDI connectee: {name}"));
    let source: Arc<str> = Arc::from(name.as_str());
    let tx = midi_tx.clone();
    input
        .connect(
            &port,
            "osc-midi-in",
            move |_timestamp, message, _| {
                let frame = MidiFrame {
                    data: smallvec::SmallVec::from_slice(message),
                    source: Arc::clone(&source),
                };
                let _ = tx.send(frame);
            },
            midi_tx,
        )
        .map(|conn| (conn, name))
        .map_err(|e| e.to_string())
}

pub(super) fn open_output(
    config: &Config,
    logger: &FrontendLogger,
) -> (Option<MidiOutputConnection>, Option<String>) {
    if let Some(name) = config.midi.output_device.as_ref() {
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
    let Some(port) = select_output_port(&output, &ports, config.midi.output_device.as_ref()) else {
        if let Some(requested) = config.midi.output_device.as_ref() {
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

fn select_input_port(
    input: &MidiInput,
    ports: &[MidiInputPort],
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

fn select_output_port(
    output: &MidiOutput,
    ports: &[MidiOutputPort],
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

pub(super) fn initial_connected_input(config: &Config) -> Option<String> {
    config
        .midi
        .input_device
        .as_ref()
        .filter(|name| name.as_str() == RTP_VIRTUAL_INPUT)
        .cloned()
}

pub(super) fn initial_connected_output(config: &Config) -> Option<String> {
    config.midi.output_device.as_ref().and_then(|name| {
        if name == VST_INTERNAL_OUTPUT || name == VST_INTERNAL_OUTPUT_LEGACY {
            Some(VST_INTERNAL_OUTPUT.to_string())
        } else {
            None
        }
    })
}

pub(super) fn reset_midi_output(out: &mut MidiOutputConnection, logger: &FrontendLogger) {
    for ch in 0u8..16 {
        let _ = out.send(&[0xB0 | ch, 64, 0]);
        let _ = out.send(&[0xB0 | ch, 120, 0]);
        let _ = out.send(&[0xB0 | ch, 121, 0]);
        let _ = out.send(&[0xB0 | ch, 123, 0]);
        let _ = out.send(&[0xE0 | ch, 0x00, 0x40]);
    }
    logger.debug("MIDI OUT: sustain/all-sound/controllers/all-notes reset + pitch bend");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_connected_input_detects_rtp_virtual_port() {
        let mut config = Config::default();
        config.midi.input_device = Some(RTP_VIRTUAL_INPUT.to_string());
        assert_eq!(
            initial_connected_input(&config),
            Some(RTP_VIRTUAL_INPUT.to_string())
        );
    }

    #[test]
    fn initial_connected_output_detects_internal_vst_port() {
        let mut config = Config::default();
        config.midi.output_device = Some(VST_INTERNAL_OUTPUT_LEGACY.to_string());
        assert_eq!(
            initial_connected_output(&config),
            Some(VST_INTERNAL_OUTPUT.to_string())
        );
    }

    #[test]
    fn midi_input_callback_does_not_use_drop_send_path() {
        let source = include_str!("midi_io.rs");
        let forbidden = ["tx", "try_send("].join(".");

        assert!(!source.contains(&forbidden));
        assert!(source.contains("tx.send(frame)"));
    }
}
