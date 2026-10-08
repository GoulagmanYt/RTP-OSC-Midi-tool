use super::{
    activity::{record_activity, MidiActivityTracker},
    midi_io::{open_output, reset_midi_output},
    pipeline::{
        handle_midi_frame, midi_reset_generation, record_fanout_drop, request_critical_midi_reset,
        take_critical_midi_reset_request, update_pipeline_queue_depth, ConfigSnapshot,
    },
};
use crate::midi_queue::{bounded, generation_action, GenerationAction, Receiver, Sender};
use crate::{
    audio::AudioEngine,
    config::Config,
    logger::FrontendLogger,
    midi::{is_critical_release_message, MidiFrame},
    osc::OscClient,
    types::MidiNoteEvent,
};
use crossbeam_channel::RecvTimeoutError;
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
// Absorb the same bounded burst as the input after a scheduling interruption.
const FANOUT_QUEUE_CAPACITY: usize = super::lifecycle::BRIDGE_MIDI_QUEUE_CAPACITY;
const PROCESSING_BATCH_LIMIT: usize = 256;

fn spawn_named(name: &str, run: impl FnOnce() + Send + 'static) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name(name.into())
        .spawn(run)
        .expect("routing thread creation")
}

struct FanoutFrame {
    frame: MidiFrame,
    generation: u64,
    route_generation: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_processing_loop(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    panic_revision: Arc<AtomicU64>,
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
    rtp_output: crate::rtp::rtp_output::RtpOutputRoute,
) -> thread::JoinHandle<()> {
    spawn_named("midi-routing", move || {
        processing_loop(
            shared_config,
            config_rev,
            panic_revision,
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
            rtp_output,
        );
    })
}

#[allow(clippy::too_many_arguments)]
fn processing_loop(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    panic_revision: Arc<AtomicU64>,
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
    rtp_output: crate::rtp::rtp_output::RtpOutputRoute,
) {
    let _priority = crate::audio::windows_tuning::prioritize_midi_thread();
    let initial_cfg = shared_config.lock().clone();
    logger.info(format!(
        "Bridge actif vers {}:{}",
        initial_cfg.osc.target_ip, initial_cfg.osc.target_port
    ));

    let (midi_thru_tx, midi_thru_rx) = bounded::<FanoutFrame>(FANOUT_QUEUE_CAPACITY);
    let (osc_tx, osc_rx) = bounded::<FanoutFrame>(FANOUT_QUEUE_CAPACITY);
    let midi_worker = spawn_midi_thru_worker(
        shared_config.clone(),
        config_rev.clone(),
        Arc::clone(&panic_revision),
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
    snapshot.rtp_output = Some(rtp_output.clone());
    let mut cached_rev = config_rev.load(Ordering::Relaxed);
    let mut sustain_pressed_state = [None; 16];
    let mut unused_midi_out = None;
    let mut unused_notes = crate::midi::ActiveNotes::default();
    let mut generation = midi_reset_generation();
    let mut pending = None;

    while !stop.load(Ordering::Relaxed) {
        let observed_generation = midi_reset_generation();
        let _ = take_critical_midi_reset_request();
        if generation != observed_generation {
            generation = observed_generation;
            // Invalidate queued work before allowing new notes after the reset.
            audio.request_emergency_midi_reset();
            panic_revision.fetch_add(1, Ordering::AcqRel);
        }
        let rev_now = config_rev.load(Ordering::Relaxed);
        if rev_now != cached_rev {
            let cfg = shared_config.lock().clone();
            snapshot = ConfigSnapshot::from(&cfg);
            snapshot.rtp_output = Some(rtp_output.clone());
            cached_rev = rev_now;
        }

        update_pipeline_queue_depth(midi_rx.len());

        // Batch-drain the bounded input queue. Audio injection happens first;
        // OSC and MIDI Thru are dispatched to independent bounded workers.
        let first = pending
            .take()
            .map_or_else(|| midi_rx.recv_timeout(Duration::from_millis(1)), Ok);
        match first {
            Ok(frame) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match generation_action(
                    (frame.generation, 0),
                    (generation, 0),
                    (midi_reset_generation(), 0),
                ) {
                    GenerationAction::Discard => continue,
                    GenerationAction::Defer => {
                        pending = Some(frame);
                        continue;
                    }
                    GenerationAction::Ready => {}
                }
                update_pipeline_queue_depth(midi_rx.len());
                record_activity(&activity, &frame);
                let Some((midi_thru_frame, osc_frame)) =
                    frame.clone_realtime().zip(frame.clone_realtime())
                else {
                    record_fanout_drop();
                    request_critical_midi_reset();
                    continue;
                };
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
                    &mut unused_notes,
                    None,
                );
                if snapshot.midi_thru {
                    send_fanout(&midi_thru_tx, midi_thru_frame, generation, true);
                }
                send_fanout(&osc_tx, osc_frame, generation, snapshot.osc_enabled);
                // Drain remaining buffered messages without waiting.
                for _ in 1..PROCESSING_BATCH_LIMIT {
                    if stop.load(Ordering::Relaxed) || midi_reset_generation() != generation {
                        break;
                    }
                    let Ok(frame) = midi_rx.try_recv() else {
                        break;
                    };
                    match generation_action(
                        (frame.generation, 0),
                        (generation, 0),
                        (midi_reset_generation(), 0),
                    ) {
                        GenerationAction::Discard => continue,
                        GenerationAction::Defer => {
                            pending = Some(frame);
                            break;
                        }
                        GenerationAction::Ready => {}
                    }
                    update_pipeline_queue_depth(midi_rx.len());
                    record_activity(&activity, &frame);
                    let Some((midi_thru_frame, osc_frame)) =
                        frame.clone_realtime().zip(frame.clone_realtime())
                    else {
                        record_fanout_drop();
                        request_critical_midi_reset();
                        continue;
                    };
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
                        &mut unused_notes,
                        None,
                    );
                    if snapshot.midi_thru {
                        send_fanout(&midi_thru_tx, midi_thru_frame, generation, true);
                    }
                    send_fanout(&osc_tx, osc_frame, generation, snapshot.osc_enabled);
                }
                update_pipeline_queue_depth(midi_rx.len());
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }
    }

    for ch in 0..16u8 {
        audio.send_midi(&[0xB0 | ch, 120, 0]);
        audio.send_midi(&[0xB0 | ch, 121, 0]);
        audio.send_midi(&[0xB0 | ch, 123, 0]);
    }
    drop(midi_thru_tx);
    drop(osc_tx);
    let _ = midi_worker.join();
    let _ = osc_worker.join();
}

fn send_fanout(tx: &Sender<FanoutFrame>, frame: MidiFrame, generation: u64, carries_output: bool) {
    if let Err(error) = tx.try_send(FanoutFrame {
        frame,
        generation,
        route_generation: tx.recovery().generation(),
    }) {
        let lost = error.into_inner();
        record_fanout_drop();
        // The OSC worker also feeds the UI when OSC is disabled. Losing a UI
        // update must never silence audio or release the sustain pedal.
        if carries_output && is_critical_release_message(lost.frame.data.as_slice()) {
            tx.recovery().request_reset();
        }
    }
}

fn flush_midi_output(
    out: &mut MidiOutputConnection,
    notes: &mut crate::midi::ActiveNotes,
    logger: &FrontendLogger,
) {
    notes.release_all(|message| {
        let _ = out.send(&message);
    });
    reset_midi_output(out, logger);
}

#[allow(clippy::too_many_arguments)]
fn spawn_midi_thru_worker(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    panic_revision: Arc<AtomicU64>,
    rx: Receiver<FanoutFrame>,
    stop: Arc<AtomicBool>,
    mut midi_out: Option<MidiOutputConnection>,
    logger: FrontendLogger,
    audio: AudioEngine,
    actual_midi_out: Arc<Mutex<Option<String>>>,
    osc_counter: Arc<AtomicU32>,
    midi_event_tx: Sender<MidiNoteEvent>,
) -> thread::JoinHandle<()> {
    spawn_named("midi-thru", move || {
        let _priority = crate::audio::windows_tuning::prioritize_midi_thread();
        let initial = shared_config.lock().clone();
        let mut snapshot = ConfigSnapshot::from(&initial);
        let mut cached_rev = 0;
        let mut cached_panic_revision = panic_revision.load(Ordering::Acquire);
        let mut midi_out_name = initial.midi.output_device;
        let mut next_output_reconnect = std::time::Instant::now();
        let mut output_reconnect_backoff = Duration::from_millis(500);
        let mut sustain = [None; 16];
        let mut active_notes = crate::midi::ActiveNotes::default();
        let mut cached_generation = midi_reset_generation();
        let mut route_generation = rx.recovery().generation();
        let mut pending = None;
        if let Some(out) = midi_out.as_mut() {
            flush_midi_output(out, &mut active_notes, &logger);
        }
        while !stop.load(Ordering::Relaxed) {
            let current_panic_revision = panic_revision.load(Ordering::Acquire);
            if current_panic_revision != cached_panic_revision
                || cached_generation != midi_reset_generation()
                || route_generation != rx.recovery().generation()
            {
                cached_generation = midi_reset_generation();
                route_generation = rx.recovery().generation();
                if let Some(out) = midi_out.as_mut() {
                    flush_midi_output(out, &mut active_notes, &logger);
                }
                sustain.fill(None);
                cached_panic_revision = current_panic_revision;
            }
            let observed_rev = config_rev.load(Ordering::Acquire);
            if observed_rev != cached_rev {
                let config = shared_config.lock().clone();
                snapshot = ConfigSnapshot::from(&config);
                cached_rev = observed_rev;
                if let Some(out) = midi_out.as_mut() {
                    flush_midi_output(out, &mut active_notes, &logger);
                }
                if config.midi.output_device != midi_out_name {
                    let (connection, connected) = open_output(&config, &logger);
                    midi_out = connection;
                    *actual_midi_out.lock() = connected;
                    midi_out_name = config.midi.output_device;
                    output_reconnect_backoff = Duration::from_millis(500);
                    next_output_reconnect = std::time::Instant::now() + Duration::from_millis(500);
                }
            }
            if midi_out.is_none()
                && snapshot.midi_thru
                && midi_out_name.as_deref().is_some_and(|name| {
                    name != crate::config::VST_INTERNAL_OUTPUT
                        && name != crate::config::VST_INTERNAL_OUTPUT_LEGACY
                })
                && std::time::Instant::now() >= next_output_reconnect
            {
                let config = shared_config.lock().clone();
                let (connection, connected) = open_output(&config, &logger);
                midi_out = connection;
                *actual_midi_out.lock() = connected;
                if let Some(out) = midi_out.as_mut() {
                    flush_midi_output(out, &mut active_notes, &logger);
                    output_reconnect_backoff = Duration::from_millis(500);
                } else {
                    output_reconnect_backoff =
                        (output_reconnect_backoff * 2).min(Duration::from_secs(10));
                }
                next_output_reconnect = std::time::Instant::now() + output_reconnect_backoff;
            }
            match pending
                .take()
                .map_or_else(|| rx.recv_timeout(Duration::from_millis(5)), Ok)
            {
                Ok(envelope) => {
                    match generation_action(
                        (envelope.generation, envelope.route_generation),
                        (cached_generation, route_generation),
                        (midi_reset_generation(), rx.recovery().generation()),
                    ) {
                        GenerationAction::Discard => continue,
                        GenerationAction::Defer => {
                            pending = Some(envelope);
                            continue;
                        }
                        GenerationAction::Ready => {}
                    }
                    let frame = envelope.frame;
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
                        &mut active_notes,
                        Some(rx.recovery()),
                    );
                    if midi_out.is_none() {
                        *actual_midi_out.lock() = None;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some(out) = midi_out.as_mut() {
            flush_midi_output(out, &mut active_notes, &logger);
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_osc_worker(
    shared_config: Arc<Mutex<Config>>,
    config_rev: Arc<AtomicU64>,
    rx: Receiver<FanoutFrame>,
    stop: Arc<AtomicBool>,
    osc: OscClient,
    logger: FrontendLogger,
    audio: AudioEngine,
    osc_counter: Arc<AtomicU32>,
    midi_event_tx: Sender<MidiNoteEvent>,
) -> thread::JoinHandle<()> {
    spawn_named("osc-send", move || {
        let _priority = crate::audio::windows_tuning::prioritize_midi_thread();
        let initial = shared_config.lock().clone();
        let mut snapshot = ConfigSnapshot::from(&initial);
        let mut cached_rev = 0;
        let mut osc_client = Some(osc);
        let mut current_target = (initial.osc.target_ip, initial.osc.target_port);
        let mut sustain = [None; 16];
        let mut active_notes = crate::midi::ActiveNotes::default();
        let mut cached_generation = midi_reset_generation();
        let mut route_generation = rx.recovery().generation();
        let mut pending = None;
        let mut no_midi_out = None;
        let mut reset_pending = false;
        while !stop.load(Ordering::Relaxed) {
            let generation = midi_reset_generation();
            if cached_generation != generation || route_generation != rx.recovery().generation() {
                route_generation = rx.recovery().generation();
                cached_generation = generation;
                reset_pending = true;
                sustain.fill(None);
            }
            if reset_pending {
                reset_pending = osc_client
                    .as_ref()
                    .is_some_and(|client| client.send_reset_all().is_err());
                if reset_pending {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            }
            let observed_rev = config_rev.load(Ordering::Acquire);
            if observed_rev != cached_rev {
                let config = shared_config.lock().clone();
                if let Some(client) = osc_client.as_ref() {
                    let _ = client.send_reset_all();
                }
                sustain.fill(None);
                snapshot = ConfigSnapshot::from(&config);
                cached_rev = observed_rev;
            }
            update_osc_client(&snapshot, &logger, &mut osc_client, &mut current_target);
            match pending
                .take()
                .map_or_else(|| rx.recv_timeout(Duration::from_millis(5)), Ok)
            {
                Ok(envelope) => {
                    match generation_action(
                        (envelope.generation, envelope.route_generation),
                        (cached_generation, route_generation),
                        (midi_reset_generation(), rx.recovery().generation()),
                    ) {
                        GenerationAction::Discard => continue,
                        GenerationAction::Defer => {
                            pending = Some(envelope);
                            continue;
                        }
                        GenerationAction::Ready => {}
                    }
                    let frame = envelope.frame;
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
                        &mut active_notes,
                        Some(rx.recovery()),
                    )
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some(client) = osc_client.as_ref() {
            let _ = client.send_reset_all();
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
    spawn_named("midi-ui-events", move || {
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

impl crate::midi_queue::Priority for FanoutFrame {
    fn critical(&self) -> bool {
        crate::midi::is_critical_release_message(self.frame.data.as_slice())
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::bridge::pipeline::{try_enqueue_midi_frame, EnqueueOutcome};

    fn note(status: u8, channel: u8, pitch: u8) -> MidiFrame {
        MidiFrame {
            generation: MidiFrame::current_generation(),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            data: smallvec::smallvec![
                status | channel,
                pitch,
                if status == 0x90 { 100 } else { 0 }
            ],
            source: Arc::from("regression"),
        }
    }

    #[test]
    fn ten_thousand_polyphonic_notes_cross_bounded_queues_and_all_release() {
        let (input, receiver) = bounded(256);
        let (output, sink) = bounded(256);
        let mut active = crate::midi::ActiveNotes::default();
        let mut on_count = 0;
        let mut off_count = 0;
        for batch in 0..100 {
            for status in [0x90, 0x80] {
                for n in 0..100 {
                    assert_eq!(
                        try_enqueue_midi_frame(&input, note(status, (batch % 16) as u8, n))
                            .unwrap(),
                        EnqueueOutcome::Enqueued
                    );
                }
                for frame in receiver.try_iter() {
                    send_fanout(&output, frame, 0, true);
                }
                for envelope in sink.try_iter() {
                    if envelope.frame.data[0] & 0xF0 == 0x90 {
                        on_count += 1;
                    } else {
                        off_count += 1;
                    }
                    active.observe(&envelope.frame.data);
                }
            }
        }
        assert_eq!((on_count, off_count), (10_000, 10_000));
        active.release_all(|_| panic!("Unmatched active note after burst"));
    }

    #[test]
    fn ui_only_overflow_does_not_reset_notes_or_sustain() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, _rx) = bounded(1);
        let generation = midi_reset_generation();
        send_fanout(&tx, note(0x90, 0, 60), generation, false);
        send_fanout(&tx, note(0x80, 0, 60), generation, false);
        send_fanout(
            &tx,
            MidiFrame {
                generation: MidiFrame::current_generation(),
                ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                data: smallvec::smallvec![0xB0, 64, 0],
                source: Arc::from("regression"),
            },
            generation,
            false,
        );
        assert_eq!(midi_reset_generation(), generation);
    }

    #[test]
    fn full_input_burst_preserves_sustain_and_release_order() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, rx) = bounded(FANOUT_QUEUE_CAPACITY);
        let generation = midi_reset_generation();
        for i in 0..super::super::lifecycle::BRIDGE_MIDI_QUEUE_CAPACITY {
            let data = match i % 4 {
                0 => smallvec::smallvec![0xB0, 64, 127],
                1 => smallvec::smallvec![0x90, 60, 100],
                2 => smallvec::smallvec![0x80, 60, 0],
                _ => smallvec::smallvec![0xB0, 64, 0],
            };
            send_fanout(
                &tx,
                MidiFrame {
                    generation: MidiFrame::current_generation(),
                    ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    data,
                    source: Arc::from("regression"),
                },
                generation,
                true,
            );
        }
        assert_eq!(midi_reset_generation(), generation);
        assert_eq!(
            rx.len(),
            super::super::lifecycle::BRIDGE_MIDI_QUEUE_CAPACITY
        );
        for (i, event) in rx.try_iter().enumerate() {
            let expected: &[u8] = match i % 4 {
                0 => &[0xB0, 64, 127],
                1 => &[0x90, 60, 100],
                2 => &[0x80, 60, 0],
                _ => &[0xB0, 64, 0],
            };
            assert_eq!(event.frame.data.as_slice(), expected);
        }
    }

    #[test]
    fn release_overflow_invalidates_queued_note_on_without_waiting() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let (tx, rx) = bounded(1);
        let generation = midi_reset_generation();
        send_fanout(&tx, note(0x90, 0, 60), generation, true);
        send_fanout(&tx, note(0x80, 0, 60), generation, true);
        let old = rx.recv().unwrap();
        assert_eq!(old.generation, midi_reset_generation());
        assert!(old.route_generation < rx.recovery().generation());
    }
}
