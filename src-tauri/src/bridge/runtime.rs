//! Runtime principal du bridge.

use super::{
    lifecycle::{start_runtime, stop_runtime, sync_rtp as sync_rtp_runtime},
    pipeline::try_enqueue_midi_frame,
    status::{enrich_status_from_runtime, refresh_runtime_status},
};
use crate::midi_queue::Sender;
use crate::{
    audio::AudioEngine,
    config::Config,
    logger::FrontendLogger,
    midi::MidiFrame,
    reliable_playback::ReliablePlaybackServer,
    rtp::{RtpRemoteTarget, RtpServer},
    types::{BridgeStatus, RtpParticipantInfo},
};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::Window;

#[derive(Clone)]
pub struct BridgeHandle {
    rtp_output: crate::rtp::rtp_output::RtpOutputRoute,
    inner: Arc<Mutex<Option<BridgeRuntime>>>,
    rtp_server: Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: Arc<arc_swap::ArcSwapOption<Sender<MidiFrame>>>,
    rtp_config: Arc<Mutex<Option<RtpConfigSnapshot>>>,
}

#[derive(Clone)]
pub(super) struct RtpConfigSnapshot {
    pub(super) name: String,
    pub(super) requested_port: u16,
    pub(super) bound_port: u16,
    pub(super) remote_enabled: bool,
    pub(super) remote_targets: Vec<RtpRemoteTarget>,
    pub(super) log_rtp: bool,
}

pub(super) struct BridgeRuntime {
    pub(super) status: BridgeStatus,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) processing: Option<thread::JoinHandle<()>>,
    pub(super) midi_watcher: Option<thread::JoinHandle<()>>,
    pub(super) activity_emitter: Option<thread::JoinHandle<()>>,
    pub(super) midi_event_emitter: Option<thread::JoinHandle<()>>,
    pub(super) reliable_playback: Option<ReliablePlaybackServer>,
    pub(super) osc_input: Option<crate::osc_input::OscInputServer>,
    pub(super) config: Arc<Mutex<Config>>,
    pub(super) config_rev: Arc<AtomicU64>,
    pub(super) actual_midi_in: Arc<Mutex<Option<String>>>,
    pub(super) actual_midi_out: Arc<Mutex<Option<String>>>,
}

impl BridgeHandle {
    pub fn new() -> Self {
        crate::midi::initialize_midi_buffers();
        Self {
            rtp_output: crate::rtp::rtp_output::RtpOutputRoute::default(),
            inner: Arc::new(Mutex::new(None)),
            rtp_server: Arc::new(Mutex::new(None)),
            rtp_sink: Arc::new(arc_swap::ArcSwapOption::empty()),
            rtp_config: Arc::new(Mutex::new(None)),
        }
    }

    pub fn sync_rtp(
        &self,
        config: &Config,
        logger: &FrontendLogger,
        force_restart: bool,
    ) -> Result<(), String> {
        sync_rtp_runtime(
            &self.rtp_server,
            &self.rtp_sink,
            &self.rtp_config,
            config,
            logger,
            force_restart,
            &self.rtp_output,
        )
    }

    pub fn restart_rtp(&self, config: &Config, logger: &FrontendLogger) -> Result<(), String> {
        self.sync_rtp(config, logger, true)
    }

    pub fn start(
        &self,
        window: Window,
        config: Config,
        dev_logging: Arc<AtomicBool>,
        audio: AudioEngine,
    ) -> Result<BridgeStatus, String> {
        self.stop_internal();
        let runtime = start_runtime(
            window,
            config,
            dev_logging,
            audio,
            &self.rtp_server,
            &self.rtp_sink,
            &self.rtp_config,
            self.rtp_output.clone(),
        )?;
        let status = runtime.status.clone();
        *self.inner.lock() = Some(runtime);
        Ok(status)
    }

    pub fn stop(&self) -> Result<(), String> {
        self.stop_internal();
        self.rtp_output.set(None);
        // RTP discovery/session setup may have run before the bridge started.
        if let Some(server) = self.rtp_server.lock().take() {
            crate::tauri::utils::safe_block_on(server.stop());
        }
        self.rtp_sink.store(None);
        *self.rtp_config.lock() = None;
        Ok(())
    }

    pub fn reset_keys(&self) -> Result<(), String> {
        super::pipeline::request_critical_midi_reset();
        Ok(())
    }

    pub fn send_test_note(&self, note: u8, velocity: u8, channel: u8) -> Result<(), String> {
        let ch = channel.clamp(1, 16) - 1;
        let note = note.min(127);
        let velocity = velocity.min(127);
        let source = "Test MIDI".to_string();
        let data = SmallVec::from_slice(&[0x90 | ch, note, velocity]);
        let generation = MidiFrame::current_generation();

        let sink = self
            .rtp_sink
            .load_full()
            .map(|sender| (*sender).clone())
            .ok_or("Bridge MIDI input unavailable")?;
        try_enqueue_midi_frame(
            &sink,
            MidiFrame {
                generation,
                ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                data,
                source: std::sync::Arc::from(source.as_str()),
            },
        )
        .map_err(|e| format!("Failed to inject test MIDI: {e}"))?;

        let sink_ref = self.rtp_sink.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            if MidiFrame::current_generation() != generation {
                return;
            }
            let off = smallvec::SmallVec::from_slice(&[0x80 | ch, note, 0]);
            if let Some(tx) = sink_ref.load_full() {
                let _ = try_enqueue_midi_frame(
                    &tx,
                    MidiFrame {
                        generation,
                        ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                        data: off,
                        source: std::sync::Arc::from(source.as_str()),
                    },
                );
            }
        });

        Ok(())
    }

    pub fn send_test_cc(&self, cc: u8, value: u8, channel: u8) -> Result<(), String> {
        let ch = channel.clamp(1, 16) - 1;
        let cc = cc.min(127);
        let value = value.min(127);
        let data = SmallVec::from_slice(&[0xB0 | ch, cc, value]);
        self.inject_frame(data, "Test MIDI".to_string())
    }

    pub fn update_config(&self, config: Config, logger: &FrontendLogger) -> Result<(), String> {
        let mut guard = self.inner.lock();
        if let Some(runtime) = guard.as_mut() {
            let old = runtime.config.lock().osc.clone();
            if old.input_enabled != config.osc.input_enabled
                || old.listen_ip != config.osc.listen_ip
                || old.listen_port != config.osc.listen_port
            {
                let sink = self
                    .rtp_sink
                    .load_full()
                    .map(|sender| (*sender).clone())
                    .ok_or("Bridge MIDI input unavailable")?;
                if old.input_enabled && old.listen_port == config.osc.listen_port {
                    if let Some(previous) = runtime.osc_input.take() {
                        previous.stop();
                    }
                    match crate::osc_input::OscInputServer::start(
                        &config.osc,
                        sink.clone(),
                        Some(logger.clone()),
                    ) {
                        Ok(replacement) => {
                            runtime.osc_input = replacement;
                        }
                        Err(err) => {
                            if let Ok(recovered) = crate::osc_input::OscInputServer::start(
                                &old,
                                sink,
                                Some(logger.clone()),
                            ) {
                                runtime.osc_input = recovered;
                            }
                            return Err(err);
                        }
                    }
                } else {
                    let replacement = crate::osc_input::OscInputServer::start(
                        &config.osc,
                        sink,
                        Some(logger.clone()),
                    )?;
                    if let Some(previous) = runtime.osc_input.take() {
                        previous.stop();
                    }
                    runtime.osc_input = replacement;
                }
            }
            let old_config = runtime.config.lock().clone();
            self.rtp_output.set_thru_enabled(config.rtp.thru_enabled);
            let old_midi = &old_config.midi;
            let routing_changed = old_midi.channel_filter != config.midi.channel_filter
                || !super::pipeline::ConfigSnapshot::from(&old_config)
                    .same_profiles(&super::pipeline::ConfigSnapshot::from(&config));
            let sender = self.rtp_sink.load();
            if routing_changed {
                if let Some(sender) = sender.as_ref() {
                    sender.invalidate_preparation();
                }
            }
            if routing_changed
                || old_midi.input_device != config.midi.input_device
                || old_midi.hotplug != config.midi.hotplug
            {
                super::pipeline::request_critical_midi_reset();
            }
            if routing_changed {
                if let Some(sender) = sender.as_ref() {
                    super::pipeline::prepare_input_routing(sender, &config);
                }
            }
            *runtime.config.lock() = config.clone();
            refresh_runtime_status(
                runtime,
                &old_config,
                &config,
                self.rtp_server.lock().is_some(),
                self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port),
            );
            runtime.config_rev.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn status(&self, config: &Config) -> BridgeStatus {
        let live_advertisement = self
            .rtp_server
            .lock()
            .as_ref()
            .map(|server| server.advertisement_status());
        let rtp_active = live_advertisement.is_some();
        let bound_port = if rtp_active {
            self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port)
        } else {
            None
        };
        let guard = self.inner.lock();
        let mut status = guard
            .as_ref()
            .map(|r| r.status.clone())
            .unwrap_or_else(|| BridgeStatus::from(config));
        if let Some(runtime) = guard.as_ref() {
            enrich_status_from_runtime(&mut status, runtime);
        }
        status.rtp_active = rtp_active;
        status.rtp_bound_port = bound_port;
        if let Some(adv) = live_advertisement {
            status.rtp_advertised_host = adv.advertised_host;
            status.rtp_advertised_addresses = adv.advertised_addresses;
            status.rtp_network_warning = adv.warning;
        } else {
            status.rtp_advertised_host = None;
            status.rtp_advertised_addresses.clear();
            status.rtp_network_warning = None;
        }
        status
    }

    pub fn is_running(&self) -> bool {
        self.inner.lock().is_some()
    }

    pub fn rtp_participants(&self) -> Vec<RtpParticipantInfo> {
        self.rtp_server
            .lock()
            .as_ref()
            .map(|server| server.participants())
            .unwrap_or_default()
    }

    pub fn owns_rtp_port(&self, port: u16) -> bool {
        let server_present = self.rtp_server.lock().is_some();
        if !server_present {
            return false;
        }
        self.rtp_config
            .lock()
            .as_ref()
            .map(|cfg| cfg.bound_port == port || cfg.requested_port == port)
            .unwrap_or(false)
    }

    /// Inject a MIDI frame directly into the bridge pipeline (for testing).
    pub fn inject_frame(&self, data: SmallVec<[u8; 32]>, source: String) -> Result<(), String> {
        if let Some(tx) = self.rtp_sink.load_full() {
            try_enqueue_midi_frame(
                &tx,
                MidiFrame {
                    generation: MidiFrame::current_generation(),
                    ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    data,
                    source: std::sync::Arc::from(source.as_str()),
                },
            )
            .and_then(|outcome| match outcome {
                super::pipeline::EnqueueOutcome::Enqueued
                | super::pipeline::EnqueueOutcome::Filtered => Ok(()),
                super::pipeline::EnqueueOutcome::DroppedNonCritical => {
                    Err(super::pipeline::EnqueueError::QueueFull)
                }
            })
            .map_err(|error| error.to_string())
        } else {
            Err("Bridge not running".to_string())
        }
    }

    fn stop_internal(&self) {
        let mut guard = self.inner.lock();
        if let Some(runtime) = guard.take() {
            stop_runtime(runtime, &self.rtp_server, &self.rtp_sink, &self.rtp_config);
        }
    }
}

impl Default for BridgeHandle {
    fn default() -> Self {
        Self::new()
    }
}
