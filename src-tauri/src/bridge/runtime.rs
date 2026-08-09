//! Runtime principal du bridge.

use super::{
    lifecycle::{start_runtime, stop_runtime, sync_rtp as sync_rtp_runtime},
    pipeline::try_enqueue_midi_frame,
    status::{enrich_status_from_runtime, refresh_runtime_status},
};
use crate::{
    audio::AudioEngine,
    config::Config,
    logger::FrontendLogger,
    midi::MidiFrame,
    osc::OscClient,
    reliable_playback::ReliablePlaybackServer,
    rtp::{rtp_advertisement::RtpAdvertisementStatus, RtpRemoteTarget, RtpServer},
    types::{BridgeStatus, RtpParticipantInfo},
};
use crossbeam_channel::Sender;
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
    inner: Arc<Mutex<Option<BridgeRuntime>>>,
    rtp_server: Arc<Mutex<Option<RtpServer>>>,
    rtp_sink: Arc<parking_lot::RwLock<Option<Sender<MidiFrame>>>>,
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
    pub(super) advertisement: RtpAdvertisementStatus,
}

pub(super) struct BridgeRuntime {
    pub(super) status: BridgeStatus,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) processing: Option<thread::JoinHandle<()>>,
    pub(super) midi_watcher: Option<thread::JoinHandle<()>>,
    pub(super) activity_emitter: Option<thread::JoinHandle<()>>,
    pub(super) midi_event_emitter: Option<thread::JoinHandle<()>>,
    pub(super) reliable_playback: Option<ReliablePlaybackServer>,
    pub(super) config: Arc<Mutex<Config>>,
    pub(super) config_rev: Arc<AtomicU64>,
    pub(super) panic_revision: Arc<AtomicU64>,
    pub(super) actual_midi_in: Arc<Mutex<Option<String>>>,
    pub(super) actual_midi_out: Arc<Mutex<Option<String>>>,
}

impl BridgeHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            rtp_server: Arc::new(Mutex::new(None)),
            rtp_sink: Arc::new(parking_lot::RwLock::new(None)),
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
        )?;
        let status = runtime.status.clone();
        *self.inner.lock() = Some(runtime);
        Ok(status)
    }

    pub fn stop(&self) -> Result<(), String> {
        self.stop_internal();
        Ok(())
    }

    pub fn reset_keys(&self) -> Result<(), String> {
        let guard = self.inner.lock();
        if let Some(runtime) = guard.as_ref() {
            runtime.panic_revision.fetch_add(1, Ordering::Release);
            let cfg = runtime.config.lock().clone();
            if cfg.osc.enabled {
                let osc = OscClient::new(&cfg.osc.target_ip, cfg.osc.target_port)?;
                osc.send_reset_all()?;
            }
        }
        Ok(())
    }

    pub fn send_test_note(&self, note: u8, velocity: u8, channel: u8) -> Result<(), String> {
        let ch = channel.clamp(1, 16) - 1;
        let note = note.min(127);
        let velocity = velocity.min(127);
        let source = "Test MIDI".to_string();
        let data = SmallVec::from_slice(&[0x90 | ch, note, velocity]);
        self.inject_frame(data, source.clone())?;

        let sink = self.rtp_sink.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            let off = smallvec::SmallVec::from_slice(&[0x80 | ch, note, 0]);
            if let Some(tx) = sink.read().as_ref().cloned() {
                let _ = try_enqueue_midi_frame(
                    &tx,
                    MidiFrame {
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

    pub fn update_config(&self, config: Config, _logger: &FrontendLogger) -> Result<(), String> {
        let mut guard = self.inner.lock();
        if let Some(runtime) = guard.as_mut() {
            *runtime.config.lock() = config.clone();
            refresh_runtime_status(
                runtime,
                &config,
                self.rtp_server.lock().is_some(),
                self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port),
            );
            runtime.config_rev.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn status(&self, config: &Config) -> BridgeStatus {
        let rtp_active = self.rtp_server.lock().is_some();
        let bound_port = self.rtp_config.lock().as_ref().map(|cfg| cfg.bound_port);
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
        if let Some(cfg) = self.rtp_config.lock().as_ref() {
            status.rtp_advertised_host = cfg.advertisement.advertised_host.clone();
            status.rtp_advertised_addresses = cfg.advertisement.advertised_addresses.clone();
            status.rtp_network_warning = cfg.advertisement.warning.clone();
        } else {
            status.rtp_advertised_host = None;
            status.rtp_advertised_addresses.clear();
            status.rtp_network_warning = None;
        }
        status
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
        if let Some(tx) = self.rtp_sink.read().as_ref().cloned() {
            try_enqueue_midi_frame(
                &tx,
                MidiFrame {
                    data,
                    source: std::sync::Arc::from(source.as_str()),
                },
            )
            .map(|_| ())
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
