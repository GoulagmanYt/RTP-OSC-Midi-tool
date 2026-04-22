use super::{
    activity::{record_activity, MidiActivityTracker},
    midi_io::{open_output, reset_midi_output},
    pipeline::{handle_midi_frame, ConfigSnapshot},
};
use crate::{
    audio::AudioEngine, config::Config, logger::FrontendLogger, midi::MidiFrame, osc::OscClient,
};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use midir::MidiOutputConnection;
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_processing_loop(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    osc: OscClient,
    midi_rx: Receiver<MidiFrame>,
    stop: Arc<AtomicBool>,
    midi_out: Option<MidiOutputConnection>,
    logger: FrontendLogger,
    audio: AudioEngine,
    activity: Arc<Mutex<MidiActivityTracker>>,
    osc_counter: Arc<AtomicU32>,
    actual_midi_out: Arc<Mutex<Option<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        processing_loop(
            shared_config,
            config_rev,
            osc,
            midi_rx,
            stop,
            midi_out,
            logger,
            audio,
            activity,
            osc_counter,
            actual_midi_out,
        );
    })
}

#[allow(clippy::too_many_arguments)]
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
        initial_cfg.osc.target_ip, initial_cfg.osc.target_port
    ));

    if let Some(out) = midi_out.as_mut() {
        reset_midi_output(out, &logger);
    }

    let mut osc_client = Some(osc);
    let mut current_target = (
        initial_cfg.osc.target_ip.clone(),
        initial_cfg.osc.target_port,
    );
    let mut snapshot = ConfigSnapshot::from(&initial_cfg);
    let mut cached_rev = config_rev.load(Ordering::Relaxed);
    let mut midi_out_name = initial_cfg.midi.output_device.clone();
    let mut sustain_pressed_state = [None; 16];

    while !stop.load(Ordering::Relaxed) {
        let rev_now = config_rev.load(Ordering::Relaxed);
        if rev_now != cached_rev {
            let cfg = shared_config.lock().clone();
            snapshot = ConfigSnapshot::from(&cfg);
            cached_rev = rev_now;
            if cfg.midi.output_device != midi_out_name {
                let (conn, connected_output) = open_output(&cfg, &logger);
                midi_out = conn;
                *actual_midi_out.lock() = connected_output;
                if let Some(out) = midi_out.as_mut() {
                    reset_midi_output(out, &logger);
                }
                midi_out_name = cfg.midi.output_device.clone();
            }
        }

        update_osc_client(&snapshot, &logger, &mut osc_client, &mut current_target);

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

fn update_osc_client(
    snapshot: &ConfigSnapshot,
    logger: &FrontendLogger,
    osc_client: &mut Option<OscClient>,
    current_target: &mut (String, u16),
) {
    if snapshot.osc_enabled {
        let target_tuple = (snapshot.osc_target_ip.clone(), snapshot.osc_target_port);
        if target_tuple != *current_target || osc_client.is_none() {
            match OscClient::new(&snapshot.osc_target_ip, snapshot.osc_target_port) {
                Ok(new_osc) => {
                    *current_target = target_tuple.clone();
                    *osc_client = Some(new_osc);
                    logger.info(format!(
                        "Cible OSC mise a jour -> {}:{}",
                        target_tuple.0, target_tuple.1
                    ));
                }
                Err(err) => {
                    logger.error(format!("Impossible de creer le client OSC: {err}"));
                    *osc_client = None;
                }
            }
        }
    } else {
        if osc_client.is_some() {
            logger.debug("OSC desactive (client suspendu)");
        }
        *osc_client = None;
        *current_target = (snapshot.osc_target_ip.clone(), snapshot.osc_target_port);
    }
}
