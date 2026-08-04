use super::{
    activity::{record_activity, MidiActivityTracker},
    midi_io::{open_output, reset_midi_output},
    pipeline::{
        handle_midi_frame, record_fanout_drop, update_pipeline_queue_depth, ConfigSnapshot,
    },
};
use crate::{
    audio::AudioEngine,
    config::Config,
    logger::FrontendLogger,
    midi::{is_critical_release_message, MidiFrame},
    osc::OscClient,
    types::MidiNoteEvent,
};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
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
use tauri::{Emitter, Window};

const MIDI_NOTE_EVENT: &str = "midi:note";
const FANOUT_QUEUE_CAPACITY: usize = 2048;

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
    midi_event_tx: Sender<MidiNoteEvent>,
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
            midi_event_tx,
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
    midi_out: Option<MidiOutputConnection>,
    logger: FrontendLogger,
    audio: AudioEngine,
    activity: Arc<Mutex<MidiActivityTracker>>,
    osc_counter: Arc<AtomicU32>,
    actual_midi_out: Arc<Mutex<Option<String>>>,
    midi_event_tx: Sender<MidiNoteEvent>,
) {
    let initial_cfg = shared_config.lock().clone();
    logger.info(format!(
        "Bridge actif vers {}:{}",
        initial_cfg.osc.target_ip, initial_cfg.osc.target_port
    ));

    let (midi_thru_tx, midi_thru_rx) = bounded::<MidiFrame>(FANOUT_QUEUE_CAPACITY);
    let (osc_tx, osc_rx) = bounded::<MidiFrame>(FANOUT_QUEUE_CAPACITY);
    let midi_worker = spawn_midi_thru_worker(
        shared_config.clone(),
        config_rev.clone(),
        midi_thru_rx,
        stop.clone(),
        midi_out,
        logger.clone(),
        audio.clone(),
        actual_midi_out.clone(),
        osc_counter.clone(),
        midi_event_tx.clone(),
    );
    let osc_worker = spawn_osc_worker(
        shared_config.clone(),
        config_rev.clone(),
        osc_rx,
        stop.clone(),
        osc,
        logger.clone(),
        audio.clone(),
        osc_counter.clone(),
        midi_event_tx.clone(),
    );

    let mut snapshot = ConfigSnapshot::from(&initial_cfg);
    let mut cached_rev = config_rev.load(Ordering::Relaxed);
    let mut sustain_pressed_state = [None; 16];
    let mut unused_midi_out = None;

    while !stop.load(Ordering::Relaxed) {
        let rev_now = config_rev.load(Ordering::Relaxed);
        if rev_now != cached_rev {
            let cfg = shared_config.lock().clone();
            snapshot = ConfigSnapshot::from(&cfg);
            cached_rev = rev_now;
        }

        update_pipeline_queue_depth(midi_rx.len());

        // Batch-drain the bounded input queue. Audio injection happens first;
        // OSC and MIDI Thru are dispatched to independent bounded workers.
        let first = midi_rx.recv_timeout(Duration::from_millis(1));
        match first {
            Ok(frame) => {
                update_pipeline_queue_depth(midi_rx.len());
                record_activity(&activity, &frame);
                let midi_thru_frame = frame.clone();
                let osc_frame = frame.clone();
                handle_midi_frame(
                    &snapshot,
                    None,
                    &mut unused_midi_out,
                    &logger,
                    frame,
                    &audio,
                    &osc_counter,
                    &mut sustain_pressed_state,
                    &midi_event_tx,
                    true,
                    false,
                    false,
                );
                send_fanout(&midi_thru_tx, midi_thru_frame);
                send_fanout(&osc_tx, osc_frame);
                // Drain remaining buffered messages without waiting.
                while let Ok(frame) = midi_rx.try_recv() {
                    update_pipeline_queue_depth(midi_rx.len());
                    record_activity(&activity, &frame);
                    let midi_thru_frame = frame.clone();
                    let osc_frame = frame.clone();
                    handle_midi_frame(
                        &snapshot,
                        None,
                        &mut unused_midi_out,
                        &logger,
                        frame,
                        &audio,
                        &osc_counter,
                        &mut sustain_pressed_state,
                        &midi_event_tx,
                        true,
                        false,
                        false,
                    );
                    send_fanout(&midi_thru_tx, midi_thru_frame);
                    send_fanout(&osc_tx, osc_frame);
                }
                update_pipeline_queue_depth(midi_rx.len());
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }

    drop(midi_thru_tx);
    drop(osc_tx);
    let _ = midi_worker.join();
    let _ = osc_worker.join();
}

fn send_fanout(tx: &Sender<MidiFrame>, frame: MidiFrame) {
    match tx.try_send(frame) {
        Ok(()) => {}
        Err(crossbeam_channel::TrySendError::Full(frame))
            if is_critical_release_message(frame.data.as_slice()) =>
        {
            if tx.send_timeout(frame, Duration::from_millis(1)).is_err() {
                record_fanout_drop();
            }
        }
        Err(_) => record_fanout_drop(),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_midi_thru_worker(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    rx: Receiver<MidiFrame>,
    stop: Arc<AtomicBool>,
    mut midi_out: Option<MidiOutputConnection>,
    logger: FrontendLogger,
    audio: AudioEngine,
    actual_midi_out: Arc<Mutex<Option<String>>>,
    osc_counter: Arc<AtomicU32>,
    midi_event_tx: Sender<MidiNoteEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let initial = shared_config.lock().clone();
        let mut snapshot = ConfigSnapshot::from(&initial);
        let mut cached_rev = config_rev.load(Ordering::Relaxed);
        let mut midi_out_name = initial.midi.output_device;
        let mut sustain = [None; 16];
        if let Some(out) = midi_out.as_mut() {
            reset_midi_output(out, &logger);
        }
        while !stop.load(Ordering::Relaxed) {
            if config_rev.load(Ordering::Relaxed) != cached_rev {
                let config = shared_config.lock().clone();
                snapshot = ConfigSnapshot::from(&config);
                cached_rev = config_rev.load(Ordering::Relaxed);
                if config.midi.output_device != midi_out_name {
                    let (connection, connected) = open_output(&config, &logger);
                    midi_out = connection;
                    *actual_midi_out.lock() = connected;
                    midi_out_name = config.midi.output_device;
                }
            }
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(frame) => {
                    let deliver_external = !frame.source.starts_with("StressTest");
                    handle_midi_frame(
                        &snapshot,
                        None,
                        &mut midi_out,
                        &logger,
                        frame,
                        &audio,
                        &osc_counter,
                        &mut sustain,
                        &midi_event_tx,
                        false,
                        deliver_external,
                        false,
                    )
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some(out) = midi_out.as_mut() {
            reset_midi_output(out, &logger);
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_osc_worker(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    rx: Receiver<MidiFrame>,
    stop: Arc<AtomicBool>,
    osc: OscClient,
    logger: FrontendLogger,
    audio: AudioEngine,
    osc_counter: Arc<AtomicU32>,
    midi_event_tx: Sender<MidiNoteEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let initial = shared_config.lock().clone();
        let mut snapshot = ConfigSnapshot::from(&initial);
        let mut cached_rev = config_rev.load(Ordering::Relaxed);
        let mut osc_client = Some(osc);
        let mut current_target = (initial.osc.target_ip, initial.osc.target_port);
        let mut sustain = [None; 16];
        let mut no_midi_out = None;
        while !stop.load(Ordering::Relaxed) {
            if config_rev.load(Ordering::Relaxed) != cached_rev {
                let config = shared_config.lock().clone();
                snapshot = ConfigSnapshot::from(&config);
                cached_rev = config_rev.load(Ordering::Relaxed);
            }
            update_osc_client(&snapshot, &logger, &mut osc_client, &mut current_target);
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(frame) => {
                    let deliver_external = !frame.source.starts_with("StressTest");
                    handle_midi_frame(
                        &snapshot,
                        osc_client.as_ref(),
                        &mut no_midi_out,
                        &logger,
                        frame,
                        &audio,
                        &osc_counter,
                        &mut sustain,
                        &midi_event_tx,
                        false,
                        false,
                        deliver_external,
                    )
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    })
}

fn update_osc_client(
    snapshot: &ConfigSnapshot,
    logger: &FrontendLogger,
    osc_client: &mut Option<OscClient>,
    current_target: &mut (String, u16),
) {
    let target_changed = current_target.1 != snapshot.osc_target_port
        || current_target.0.as_str() != snapshot.osc_target_ip;

    if snapshot.osc_enabled {
        if target_changed || osc_client.is_none() {
            match OscClient::new(&snapshot.osc_target_ip, snapshot.osc_target_port) {
                Ok(new_osc) => {
                    current_target.0.clone_from(&snapshot.osc_target_ip);
                    current_target.1 = snapshot.osc_target_port;
                    *osc_client = Some(new_osc);
                    logger.info(format!(
                        "Cible OSC mise a jour -> {}:{}",
                        current_target.0, current_target.1
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
        if target_changed {
            current_target.0.clone_from(&snapshot.osc_target_ip);
            current_target.1 = snapshot.osc_target_port;
        }
    }
}

pub(super) fn spawn_midi_event_emitter(
    stop: Arc<AtomicBool>,
    window: Window,
    rx: Receiver<MidiNoteEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => {
                    let _ = window.emit(MIDI_NOTE_EVENT, event);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    })
}
