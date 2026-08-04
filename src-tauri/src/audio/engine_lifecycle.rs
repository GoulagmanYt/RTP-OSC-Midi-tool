#![allow(deprecated)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc,
    },
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    HostId,
};
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

use crate::logger::{background_log, FrontendLogger};
use crate::plugin_probe::ensure_supported_plugin_in_app;

use super::{
    callback_midi::reset_all_notes,
    device_selection::{select_device, select_host},
    engine::{db_to_linear, requires_vst3_destructor_quarantine, AudioEngine},
    runtime_state::{
        AudioControls, AudioError, AudioLifecycleState, AudioRuntime, AudioSettings,
        AudioTelemetry, EditorWindow,
    },
    state_codec::{load_vst_state, save_vst_state_blocking},
    stream_runtime::build_stream,
    windows_tuning::apply_audio_process_tuning,
};

const ASIO_RETRY_DELAYS_MS: [u64; 6] = [0, 100, 250, 500, 1_000, 1_500];

struct RetainedAudioDevice {
    device: cpal::Device,
    backend: String,
    name: String,
}

// CPAL intentionally makes platform devices !Send for the most restrictive
// backend. This wrapper is used only to hand a Windows ASIO device from the UI
// thread that loads the COM driver to the serialized audio lifecycle caller.
// The resulting runtime is already kept behind the same lifecycle gate.
struct MainThreadAudioDevice(cpal::Device);
unsafe impl Send for MainThreadAudioDevice {}

fn acquire_asio_device_on_main_thread(
    backend: Option<String>,
    preferred: Option<String>,
    logger: &FrontendLogger,
) -> Result<Option<cpal::Device>, AudioError> {
    let acquire = move || {
        let host = select_host(backend.as_deref())
            .ok_or_else(|| AudioError::Message("ASIO host not available".into()))?;
        Ok(select_device(&host, preferred.as_deref()).or_else(|| host.default_output_device()))
    };

    if super::thread_affinity::is_main_ui_thread() {
        logger.debug("Enumerating ASIO devices directly on the main UI thread");
        return acquire();
    }

    let (tx, rx) = mpsc::sync_channel(1);
    logger.debug("Scheduling ASIO device enumeration on the main UI thread");
    logger
        .app_handle()
        .run_on_main_thread(move || {
            let result = acquire().map(|device| device.map(MainThreadAudioDevice));
            let _ = tx.send(result);
        })
        .map_err(|_| {
            AudioError::Message("Failed to schedule ASIO enumeration on main thread".into())
        })?;

    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| AudioError::Message("Timed out enumerating ASIO devices".into()))?
        .map(|device| device.map(|device| device.0))
}

fn restore_vst_state_thread_affine(
    plugin: &Arc<parking_lot::Mutex<super::runtime_state::PluginBackend>>,
    parameter_cache: &Arc<parking_lot::Mutex<Vec<crate::types::VstParameter>>>,
    vst_path: &std::path::Path,
    logger: &FrontendLogger,
) -> Result<(), AudioError> {
    if super::thread_affinity::is_main_ui_thread() {
        load_vst_state(plugin, vst_path, logger);
        refresh_vst3_parameter_cache(plugin, parameter_cache);
        return Ok(());
    }

    let plugin = plugin.clone();
    let parameter_cache = parameter_cache.clone();
    let path = vst_path.to_path_buf();
    let logger_for_main = logger.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    logger.debug("Scheduling VST state restoration on the main UI thread");
    logger
        .app_handle()
        .run_on_main_thread(move || {
            load_vst_state(&plugin, &path, &logger_for_main);
            refresh_vst3_parameter_cache(&plugin, &parameter_cache);
            let _ = tx.send(());
        })
        .map_err(|_| {
            AudioError::Message("Failed to schedule VST state restoration on main thread".into())
        })?;
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .map_err(|_| AudioError::Message("Timed out restoring VST state on main thread".into()))
}

fn refresh_vst3_parameter_cache(
    plugin: &Arc<parking_lot::Mutex<super::runtime_state::PluginBackend>>,
    parameter_cache: &Arc<parking_lot::Mutex<Vec<crate::types::VstParameter>>>,
) {
    use rack::PluginInstance as _;

    let mut plugin_guard = plugin.lock();
    if let super::runtime_state::PluginBackend::Vst3 { instance, .. } = &mut *plugin_guard {
        let mut cache = parameter_cache.lock();
        for parameter in cache.iter_mut() {
            if let Ok(value) = instance.get_parameter(parameter.index) {
                parameter.value = value;
            }
        }
    }
}

impl AudioEngine {
    pub fn stop(&self, app_handle: Option<tauri::AppHandle>) {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let _lifecycle_guard = self.lifecycle_gate.lock();
            self.lifecycle_state
                .store(AudioLifecycleState::Stopping as u8, Ordering::Release);
            if let Err(error) = crate::tauri::utils::safe_block_on(self.worker.stop()) {
                background_log("error", format!("Failed to stop VST worker: {error}"));
                self.lifecycle_state
                    .store(AudioLifecycleState::Faulted as u8, Ordering::Release);
                return;
            }
            self.worker_enabled.store(false, Ordering::Release);
            self.lifecycle_state
                .store(AudioLifecycleState::Stopped as u8, Ordering::Release);
            return;
        }
        let _lifecycle_guard = self.lifecycle_gate.lock();
        let _ = self.stop_locked(app_handle, false);
    }

    fn stop_locked(
        &self,
        app_handle: Option<tauri::AppHandle>,
        retain_audio_device: bool,
    ) -> Option<RetainedAudioDevice> {
        self.lifecycle_state
            .store(AudioLifecycleState::Stopping as u8, Ordering::Release);
        let app_handle_for_drop = app_handle.clone();
        let runtime = self.runtime.lock().take();
        let mut retained_device = None;
        *self.midi_tx.lock() = None;
        self.midi_emergency_reset_requested
            .store(false, Ordering::Relaxed);

        if let Some(runtime) = runtime {
            if retain_audio_device {
                retained_device = Some(RetainedAudioDevice {
                    device: runtime._device.clone(),
                    backend: runtime.backend.clone(),
                    name: runtime.device.clone(),
                });
            }
            let plugin = runtime.plugin.clone();
            let vst_path = runtime.vst_path.clone();
            let editor_window_arc = runtime.editor_window.clone();
            let quarantine_destructor = requires_vst3_destructor_quarantine(&vst_path);

            if let Some(handle) = app_handle {
                let (tx, rx) = mpsc::channel();
                let scheduled = handle.run_on_main_thread(move || {
                    if let Some(mut editor_win) = editor_window_arc.lock().take() {
                        match &mut editor_win {
                            EditorWindow::Vst2 { editor, hwnd } => {
                                editor.close();
                                unsafe {
                                    let _ = DestroyWindow(*hwnd);
                                }
                            }
                            EditorWindow::Vst3 { gui, hwnd } => {
                                let _ = gui.detach();
                                unsafe {
                                    let _ = DestroyWindow(*hwnd);
                                }
                            }
                        }
                    }
                    let _ = tx.send(());
                });

                if scheduled.is_ok() {
                    if rx
                        .recv_timeout(super::runtime_state::vst_shutdown_timeout())
                        .is_err()
                    {
                        background_log(
                            "warn",
                            "Timed out waiting for VST editor to close on main thread",
                        );
                    }
                } else if let Some(editor_win) = runtime.editor_window.lock().take() {
                    background_log(
                        "warn",
                        "Failed to schedule VST editor close on main thread; leaking thread-affine editor",
                    );
                    std::mem::forget(editor_win);
                }
            } else if let Some(editor_win) = editor_window_arc.lock().take() {
                background_log(
                    "warn",
                    "Stopping audio without AppHandle; leaking thread-affine VST editor",
                );
                std::mem::forget(editor_win);
            }

            if let Err(error) = runtime._stream.pause() {
                background_log(
                    "warn",
                    format!("Failed to pause audio stream during shutdown: {error}"),
                );
            }
            reset_all_notes(plugin.clone());
            drop(runtime);
            save_vst_state_blocking(&plugin, &vst_path);
            if quarantine_destructor {
                background_log(
                    "warn",
                    format!(
                        "Quarantining VST3 instance {:?}: native destructor is known to crash or block; memory will be reclaimed when the hosting process exits",
                        vst_path
                    ),
                );
                std::mem::forget(plugin);
                self.lifecycle_state
                    .store(AudioLifecycleState::Stopped as u8, Ordering::Release);
                return retained_device;
            }
            if let Some(handle) = app_handle_for_drop {
                let (tx, rx) = mpsc::channel();
                let plugin_for_main_drop = plugin;
                let scheduled = handle.run_on_main_thread(move || {
                    drop(plugin_for_main_drop);
                    let _ = tx.send(());
                });
                if scheduled.is_ok() {
                    if rx
                        .recv_timeout(super::runtime_state::vst_shutdown_timeout())
                        .is_err()
                    {
                        background_log(
                            "warn",
                            "Timed out waiting for VST plugin drop on main thread",
                        );
                    }
                } else {
                    background_log("warn", "Failed to schedule VST drop on main thread");
                }
                self.lifecycle_state
                    .store(AudioLifecycleState::Stopped as u8, Ordering::Release);
                return retained_device;
            }
            drop(plugin);
        }
        self.lifecycle_state
            .store(AudioLifecycleState::Stopped as u8, Ordering::Release);
        retained_device
    }

    pub fn list_backends(&self) -> Vec<String> {
        cpal::available_hosts()
            .into_iter()
            .map(|h| h.name().to_string())
            .collect()
    }

    pub fn list_devices(&self, backend: Option<String>) -> Vec<String> {
        let host = select_host(backend.as_deref());
        host.and_then(|h| h.output_devices().ok())
            .map(|iter| iter.filter_map(|d| d.name().ok()).collect::<Vec<String>>())
            .unwrap_or_default()
    }

    pub fn start(
        &self,
        mut settings: AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        #[cfg(target_os = "windows")]
        {
            if settings.vst_worker_enabled {
                return self.start_isolated_worker(settings, vst_fallback, logger);
            }
            if self.is_worker_enabled() {
                crate::tauri::utils::safe_block_on(self.worker.stop())
                    .map_err(AudioError::Message)?;
                self.worker_enabled.store(false, Ordering::Release);
            }
        }
        // The in-process backend must never inherit a supervisor routing flag.
        settings.vst_worker_enabled = false;
        let Some(_lifecycle_guard) = self.lifecycle_gate.try_lock() else {
            return Err(AudioError::Message(
                "An audio start/reload/stop operation is already in progress".into(),
            ));
        };
        let retained_device = self.stop_locked(Some(logger.app_handle()), true);
        self.lifecycle_state
            .store(AudioLifecycleState::Loading as u8, Ordering::Release);

        let enabled = settings.enabled;
        let result = self.start_locked_after_stop(settings, vst_fallback, logger, retained_device);
        let final_state = if result.is_ok() {
            if enabled {
                AudioLifecycleState::Running
            } else {
                AudioLifecycleState::Stopped
            }
        } else {
            AudioLifecycleState::Faulted
        };
        self.lifecycle_state
            .store(final_state as u8, Ordering::Release);
        result
    }

    #[cfg(target_os = "windows")]
    fn start_isolated_worker(
        &self,
        mut settings: AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        let Some(_lifecycle_guard) = self.lifecycle_gate.try_lock() else {
            return Err(AudioError::Message(
                "An audio start/reload/stop operation is already in progress".into(),
            ));
        };

        let _ = self.stop_locked(Some(logger.app_handle()), false);
        self.lifecycle_state
            .store(AudioLifecycleState::Loading as u8, Ordering::Release);
        self.worker_enabled.store(true, Ordering::Release);

        if !settings.enabled {
            crate::tauri::utils::safe_block_on(self.worker.stop()).map_err(AudioError::Message)?;
            self.worker_enabled.store(false, Ordering::Release);
            self.lifecycle_state
                .store(AudioLifecycleState::Stopped as u8, Ordering::Release);
            return Ok(());
        }

        let vst_path = match super::plugin_host::resolve_vst_path(&settings, vst_fallback, &logger)
        {
            Ok(path) => path,
            Err(error) => {
                self.lifecycle_state
                    .store(AudioLifecycleState::Faulted as u8, Ordering::Release);
                return Err(error);
            }
        };
        let vst_probe = match ensure_supported_plugin_in_app(&vst_path) {
            Ok(probe) => probe,
            Err(error) => {
                self.lifecycle_state
                    .store(AudioLifecycleState::Faulted as u8, Ordering::Release);
                return Err(AudioError::Message(error));
            }
        };
        settings.vst_path = Some(vst_path.to_string_lossy().to_string());
        logger.info(format!(
            "Launching isolated VST worker for '{}' ({}, {})",
            vst_probe.name, vst_probe.format, vst_probe.architecture
        ));

        match crate::tauri::utils::safe_block_on(
            self.worker.start(settings, vst_probe.class_uid.clone()),
        ) {
            Ok(status) => {
                *self.last_vst.lock() = Some(vst_path);
                self.lifecycle_state
                    .store(AudioLifecycleState::Running as u8, Ordering::Release);
                logger.info(format!(
                    "VST worker ready on '{}' ({} Hz / {} samples)",
                    status.device, status.sample_rate, status.stream_buffer_size
                ));
                Ok(())
            }
            Err(error) => {
                self.lifecycle_state
                    .store(AudioLifecycleState::Faulted as u8, Ordering::Release);
                logger.error(format!("VST worker failed: {error}"));
                Err(AudioError::Message(error))
            }
        }
    }

    fn start_locked_after_stop(
        &self,
        settings: AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
        retained_device: Option<RetainedAudioDevice>,
    ) -> Result<(), AudioError> {
        if !settings.enabled {
            return Ok(());
        }

        apply_audio_process_tuning(&logger);

        let vst_path = super::plugin_host::resolve_vst_path(&settings, vst_fallback, &logger)?;
        let vst_probe = ensure_supported_plugin_in_app(&vst_path).map_err(AudioError::Message)?;
        logger.info(format!(
            "Selected plugin '{}' ({}, {}, editor={})",
            vst_probe.name, vst_probe.format, vst_probe.architecture, vst_probe.has_editor
        ));

        let mut last_err: Option<AudioError> = None;
        let mut stream_opt = None;
        let mut selected_backend: Option<String> = None;
        let mut selected_device: Option<String> = None;
        let mut selected_device_handle: Option<cpal::Device> = None;
        let controls = Arc::new(AudioControls {
            gain_bits: AtomicU32::new(db_to_linear(settings.gain_db).to_bits()),
            limiter_enabled: AtomicBool::new(settings.limiter_enabled),
        });
        let telemetry = Arc::new(AudioTelemetry::new());

        let mut candidates: Vec<(Option<String>, Option<String>, u8, bool)> = Vec::new();
        let preferred_backend = settings.backend.as_deref().unwrap_or("auto").to_lowercase();
        let asio_available = cpal::available_hosts().contains(&HostId::Asio);

        let push_asio = |list: &mut Vec<_>| {
            if asio_available {
                for attempt in 0..ASIO_RETRY_DELAYS_MS.len() as u8 {
                    list.push((
                        Some("asio".to_string()),
                        settings.device.clone(),
                        attempt,
                        true,
                    ));
                }
            }
        };
        let push_wasapi = |list: &mut Vec<_>| {
            list.push((
                Some("wasapi".to_string()),
                settings.device.clone(),
                0,
                false,
            ));
        };

        match preferred_backend.as_str() {
            "auto" | "" => {
                push_asio(&mut candidates);
                push_wasapi(&mut candidates);
            }
            val if val.contains("asio") => {
                push_asio(&mut candidates);
                if !asio_available {
                    logger.warn(
                        "ASIO requested but no ASIO host found. Falling back to WASAPI."
                            .to_string(),
                    );
                    push_wasapi(&mut candidates);
                }
            }
            val if val.contains("wasapi") => push_wasapi(&mut candidates),
            _ => {
                let prefer_low_latency = preferred_backend.contains("asio");
                candidates.push((
                    settings.backend.clone(),
                    settings.device.clone(),
                    0,
                    prefer_low_latency,
                ));
            }
        }

        let mut retained_device = retained_device.filter(|retained| {
            let backend_matches = retained.backend.eq_ignore_ascii_case(&preferred_backend)
                || (preferred_backend == "auto" && retained.backend.eq_ignore_ascii_case("asio"));
            let device_matches = settings
                .device
                .as_deref()
                .map(|requested| requested == retained.name)
                .unwrap_or(true);
            backend_matches && device_matches
        });

        for (backend, device_pref, attempt, prefer_low_latency) in candidates {
            if attempt > 0 {
                let delay_ms = ASIO_RETRY_DELAYS_MS[attempt as usize];
                logger.debug(format!(
                    "Retrying ASIO device acquisition (attempt {}/{}), waiting {}ms…",
                    attempt + 1,
                    ASIO_RETRY_DELAYS_MS.len(),
                    delay_ms
                ));
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }

            let backend_name = backend.as_deref().unwrap_or("auto");
            logger.info(format!("Attempting audio backend: {}", backend_name));

            let retained_for_attempt = if attempt == 0 {
                retained_device.take()
            } else {
                None
            };
            let device = if let Some(retained) = retained_for_attempt {
                logger.debug(format!(
                    "Reusing active audio device handle '{}' for hot VST reload",
                    retained.name
                ));
                Some(retained.device)
            } else if backend_name.contains("asio") {
                match acquire_asio_device_on_main_thread(
                    backend.clone(),
                    device_pref.clone(),
                    &logger,
                ) {
                    Ok(device) => device,
                    Err(error) => {
                        logger.warn(format!(
                            "ASIO device enumeration failed on main thread: {error}"
                        ));
                        last_err = Some(error);
                        continue;
                    }
                }
            } else {
                let host = match select_host(backend.as_deref()) {
                    Some(h) => h,
                    None => {
                        if backend_name.contains("asio") {
                            logger.warn(
                                "ASIO host not available. Install ASIO4ALL or a vendor ASIO driver."
                                    .to_string(),
                            );
                        } else {
                            logger.warn(format!("Host '{}' not available", backend_name));
                        }
                        last_err = Some(AudioError::Message(format!(
                            "Host {} not available",
                            backend_name
                        )));
                        continue;
                    }
                };
                select_device(&host, device_pref.as_deref())
                    .or_else(|| host.default_output_device())
            };
            let Some(device) = device else {
                logger.warn(format!(
                    "Audio device '{}' is temporarily unavailable on host '{}' (attempt {}/{})",
                    device_pref.as_deref().unwrap_or("default"),
                    backend_name,
                    attempt + 1,
                    if backend_name.contains("asio") {
                        ASIO_RETRY_DELAYS_MS.len()
                    } else {
                        1
                    }
                ));
                last_err = Some(AudioError::Message(
                    "No audio output device found for this host.".into(),
                ));
                continue;
            };

            let dev_name = device.name().unwrap_or_else(|_| "unknown".into());

            let has_stereo = device
                .supported_output_configs()
                .map_err(|e| AudioError::Message(e.to_string()))?
                .any(|cfg| cfg.channels() >= 2);

            if !has_stereo {
                logger.info(format!(
                    "Device '{}' skipped — VST requires stereo output.",
                    dev_name
                ));
                continue;
            }

            match build_stream(
                &device,
                settings.sample_rate,
                settings.buffer_size,
                Arc::clone(&controls),
                Arc::clone(&telemetry),
                self.midi_emergency_reset_requested.clone(),
                vst_path.clone(),
                &logger,
                backend_name,
                &dev_name,
                prefer_low_latency,
                &vst_probe,
            ) {
                Ok(res) => {
                    let (
                        stream,
                        plugin,
                        midi_tx,
                        active_sample_rate,
                        stream_buffer_size,
                        vst_midi_compatible,
                        plugin_latency_samples,
                        parameter_tx,
                        parameter_cache,
                    ) = res;
                    stream_opt = Some((
                        stream,
                        plugin,
                        midi_tx,
                        active_sample_rate,
                        stream_buffer_size,
                        vst_midi_compatible,
                        plugin_latency_samples,
                        parameter_tx,
                        parameter_cache,
                    ));
                    logger.debug(format!(
                        "Audio stream built on '{}' ({}) and waiting for VST state restoration",
                        dev_name, backend_name
                    ));
                    selected_backend = Some(backend_name.to_string());
                    selected_device = Some(dev_name);
                    selected_device_handle = Some(device.clone());
                    if !vst_midi_compatible {
                        logger.warn(
                            "VST does not advertise MIDI input support; note routing may not work."
                                .to_string(),
                        );
                    }
                    break;
                }
                Err(err) => {
                    let err_str = err.to_string();

                    if err_str.contains("no longer available") || err_str.contains("unplugged") {
                        logger.warn(format!(
                            "Device '{}' reported unavailable (attempt {}). Retrying…",
                            dev_name,
                            attempt + 1
                        ));
                    } else if err_str.contains("0x8889000A") || err_str.contains("exclusive") {
                        logger.warn(format!(
                            "Device '{}' is in exclusive mode. Close other audio applications first.",
                            dev_name
                        ));
                    } else {
                        logger.warn(format!(
                            "Failed to build stream on '{}': {}",
                            dev_name, err_str
                        ));
                    }

                    last_err = Some(err);
                    continue;
                }
            }
        }

        let (
            stream,
            plugin,
            midi_tx,
            active_sample_rate,
            stream_buffer_size,
            vst_midi_compatible,
            plugin_latency_samples,
            parameter_tx,
            parameter_cache,
        ) = stream_opt.ok_or_else(|| {
            let final_err = last_err.unwrap_or_else(|| {
                AudioError::Message("All audio backends failed to initialise.".into())
            });
            logger.error(format!("Audio initialisation failed: {}", final_err));
            final_err
        })?;

        let active_backend = selected_backend.unwrap_or_else(|| "unknown".to_string());
        let active_device = selected_device.unwrap_or_else(|| "unknown".to_string());
        let active_device_handle = selected_device_handle.ok_or_else(|| {
            AudioError::Message("Selected audio device handle was lost during startup".into())
        })?;

        restore_vst_state_thread_affine(&plugin, &parameter_cache, &vst_path, &logger)?;
        stream.play().map_err(|error| {
            AudioError::Message(format!(
                "Failed to start stream on '{}': {}",
                active_device, error
            ))
        })?;
        logger.info(format!(
            "Audio stream started on '{}' ({}) after VST state restoration",
            active_device, active_backend
        ));

        let runtime = AudioRuntime {
            _stream: stream,
            _device: active_device_handle,
            plugin: plugin.clone(),
            editor_window: Arc::new(parking_lot::Mutex::new(None)),
            controls,
            telemetry,
            sample_rate: active_sample_rate,
            requested_buffer_size: settings.buffer_size,
            stream_buffer_size,
            plugin_latency_samples,
            parameter_tx,
            parameter_cache,
            vst_midi_compatible,
            backend: active_backend,
            device: active_device,
            vst_path: vst_path.clone(),
        };

        *self.last_vst.lock() = Some(vst_path);
        *self.midi_tx.lock() = Some(midi_tx);
        *self.runtime.lock() = Some(runtime);
        Ok(())
    }

    pub fn panic_all_notes(&self) -> Result<(), AudioError> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return crate::tauri::utils::safe_block_on(self.worker.panic_all_notes())
                .map_err(AudioError::Message);
        }
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            background_log("warn", "panic_all_notes: Audio runtime not started");
            return Err(AudioError::Message("Audio runtime not started".into()));
        };
        self.midi_emergency_reset_requested
            .store(true, Ordering::Relaxed);
        reset_all_notes(runtime.plugin.clone());
        Ok(())
    }
}
