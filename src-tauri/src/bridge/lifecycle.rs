use super::{
    activity::MidiActivityTracker,
    metrics::spawn_activity_emitter,
    midi_io::{initial_connected_input, initial_connected_output, open_output, watch_midi_input},
    processing::{spawn_midi_event_emitter, spawn_processing_loop},
    rtp_config::resolve_remote_targets,
    runtime::{BridgeRuntime, RtpConfigSnapshot},
    status::build_initial_status,
};
use crate::{
    audio::AudioEngine,
    config::{Config, RTP_VIRTUAL_INPUT},
    logger::FrontendLogger,
    midi::MidiFrame,
    osc::OscClient,
    reliable_playback::{ReliablePlaybackServer, DEFAULT_RELIABLE_PLAYBACK_ADDR},
    rtp::RtpServer,
    types::{MidiNoteEvent, RtpParticipantInfo},
};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    thread,
};
use tauri::{Emitter, Window};

pub(super) const BRIDGE_MIDI_QUEUE_CAPACITY: usize = 8192;

pub(super) fn rtp_requested(config: &Config) -> bool {
    config.rtp.enabled
        || config.rtp.remote_enabled
        || config
            .midi
            .input_device
            .as_ref()
            .map(|device| device == RTP_VIRTUAL_INPUT)
            .unwrap_or(false)
}

pub(super) fn sync_rtp(
    rtp_server: &Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: &Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
    rtp_config: &Arc<Mutex<Option<RtpConfigSnapshot>>>,
    config: &Config,
    logger: &FrontendLogger,
    force_restart: bool,
) -> Result<(), String> {
    let should_run = rtp_requested(config);
    let mut server_guard = rtp_server.lock();
    let mut config_guard = rtp_config.lock();

    if should_run {
        let remote_targets = resolve_remote_targets(config, logger);
        let needs_restart = force_restart
            || server_guard.is_none()
            || config_guard
                .as_ref()
                .map(|current| {
                    current.name != config.rtp.session_name
                        || current.requested_port != config.rtp.port
                        || current.remote_enabled != config.rtp.remote_enabled
                        || current.remote_targets != remote_targets
                        || current.log_rtp != config.rtp.log_messages
                })
                .unwrap_or(true);

        if needs_restart {
            if let Some(existing) = server_guard.take() {
                tauri::async_runtime::block_on(existing.stop());
            }
            let server = RtpServer::start(
                config.rtp.session_name.clone(),
                config.rtp.port,
                remote_targets.clone(),
                config.rtp.log_messages,
                rtp_sink.clone(),
                logger.clone(),
            )?;
            let bound_port = server.bound_port();
            let advertisement = server.advertisement_status();
            *config_guard = Some(RtpConfigSnapshot {
                name: config.rtp.session_name.clone(),
                requested_port: config.rtp.port,
                bound_port,
                remote_enabled: config.rtp.remote_enabled,
                remote_targets,
                log_rtp: config.rtp.log_messages,
                advertisement,
            });
            *server_guard = Some(server);
        }
    } else {
        if let Some(existing) = server_guard.take() {
            tauri::async_runtime::block_on(existing.stop());
        }
        *config_guard = None;
        let _ = logger
            .app_handle()
            .emit("rtp:participants", Vec::<RtpParticipantInfo>::new());
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_runtime(
    window: Window,
    config: Config,
    dev_logging: Arc<AtomicBool>,
    audio: AudioEngine,
    rtp_server: &Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: &Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
    rtp_config: &Arc<Mutex<Option<RtpConfigSnapshot>>>,
) -> Result<BridgeRuntime, String> {
    let logger = FrontendLogger::new(window.clone(), dev_logging);
    let osc = OscClient::new(&config.osc.target_ip, config.osc.target_port)?;
    let wants_rtp = rtp_requested(&config);
    let mut status = build_initial_status(&config, &audio, wants_rtp);

    use crossbeam_channel::bounded;
    let stop = Arc::new(AtomicBool::new(false));
    let (midi_tx, midi_rx) = bounded::<MidiFrame>(BRIDGE_MIDI_QUEUE_CAPACITY);
    let (midi_event_tx, midi_event_rx) = bounded::<MidiNoteEvent>(1024);
    let shared_config = Arc::new(Mutex::new(config.clone()));
    let config_rev = Arc::new(AtomicU64::new(1));
    let activity_tracker = Arc::new(Mutex::new(MidiActivityTracker::default()));
    let osc_counter = Arc::new(AtomicU32::new(0));
    let actual_midi_in = Arc::new(Mutex::new(initial_connected_input(&config)));
    let actual_midi_out = Arc::new(Mutex::new(initial_connected_output(&config)));

    status.midi_input = actual_midi_in.lock().clone();
    status.midi_output = actual_midi_out.lock().clone();

    *rtp_sink.write() = Some(midi_tx.clone());
    if let Err(error) = sync_rtp(rtp_server, rtp_sink, rtp_config, &config, &logger, false) {
        *rtp_sink.write() = None;
        return Err(error);
    }
    let reliable_playback =
        match ReliablePlaybackServer::start(DEFAULT_RELIABLE_PLAYBACK_ADDR, midi_tx.clone()) {
            Ok(server) => Some(server),
            Err(error) => {
                cleanup_startup_rtp(rtp_server, rtp_sink, rtp_config);
                return Err(error);
            }
        };

    status.rtp_active = rtp_server.lock().is_some();
    status.rtp_bound_port = rtp_config.lock().as_ref().map(|current| current.bound_port);

    let midi_watcher = Some(thread::spawn({
        let midi_stop = stop.clone();
        let midi_logger = logger.clone();
        let midi_config = shared_config.clone();
        let midi_rev = config_rev.clone();
        let midi_actual_in = actual_midi_in.clone();
        let midi_tx_for_watcher = midi_tx.clone();
        move || {
            watch_midi_input(
                midi_config,
                midi_rev,
                midi_tx_for_watcher,
                midi_stop,
                midi_logger,
                midi_actual_in,
            );
        }
    }));

    let (midi_out_conn, connected_output) = open_output(&config, &logger);
    *actual_midi_out.lock() = connected_output;

    let processing = Some(spawn_processing_loop(
        shared_config.clone(),
        config_rev.clone(),
        osc,
        midi_rx,
        stop.clone(),
        midi_out_conn,
        logger.clone(),
        audio.clone(),
        activity_tracker.clone(),
        osc_counter.clone(),
        actual_midi_out.clone(),
        midi_event_tx,
    ));

    let midi_event_emitter = Some(spawn_midi_event_emitter(
        stop.clone(),
        window.clone(),
        midi_event_rx,
    ));

    let activity_emitter = Some(spawn_activity_emitter(
        stop.clone(),
        window,
        activity_tracker,
        osc_counter,
        audio,
    ));

    Ok(BridgeRuntime {
        status,
        stop,
        processing,
        midi_watcher,
        activity_emitter,
        midi_event_emitter,
        reliable_playback,
        config: shared_config,
        config_rev,
        actual_midi_in,
        actual_midi_out,
    })
}

pub(super) fn stop_runtime(
    runtime: BridgeRuntime,
    rtp_server: &Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: &Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
    rtp_config: &Arc<Mutex<Option<RtpConfigSnapshot>>>,
) {
    let config_for_reset = runtime.config.lock().clone();
    runtime.stop.store(true, Ordering::Relaxed);
    if let Some(handle) = runtime.processing {
        let _ = handle.join();
    }
    if let Some(handle) = runtime.midi_watcher {
        let _ = handle.join();
    }
    if let Some(handle) = runtime.activity_emitter {
        let _ = handle.join();
    }
    if let Some(handle) = runtime.midi_event_emitter {
        let _ = handle.join();
    }
    if let Some(server) = runtime.reliable_playback {
        server.stop();
    }
    if config_for_reset.osc.enabled {
        if let Ok(osc) = OscClient::new(
            &config_for_reset.osc.target_ip,
            config_for_reset.osc.target_port,
        ) {
            let _ = osc.send_reset_all();
        }
    }

    *rtp_sink.write() = None;
    if let Some(server) = rtp_server.lock().take() {
        tauri::async_runtime::block_on(server.stop());
    }
    *rtp_config.lock() = None;
}

fn cleanup_startup_rtp(
    rtp_server: &Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: &Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
    rtp_config: &Arc<Mutex<Option<RtpConfigSnapshot>>>,
) {
    *rtp_sink.write() = None;
    if let Some(server) = rtp_server.lock().take() {
        tauri::async_runtime::block_on(server.stop());
    }
    *rtp_config.lock() = None;
}

#[cfg(test)]
mod tests {
    #[test]
    fn runtime_uses_bounded_midi_channel() {
        let source = include_str!("lifecycle.rs");
        let required = ["bounded", "::<MidiFrame>(BRIDGE_MIDI_QUEUE_CAPACITY)"].concat();
        let forbidden = ["unbounded", "::<MidiFrame>()"].concat();

        assert!(source.contains(&required));
        assert!(!source.contains(&forbidden));
    }

    #[test]
    fn start_runtime_cleans_up_rtp_state_when_reliable_start_fails() {
        let source = include_str!("lifecycle.rs");

        assert!(source.contains("cleanup_startup_rtp"));
        assert!(source.contains("cleanup_startup_rtp(rtp_server, rtp_sink, rtp_config);"));
    }
}
