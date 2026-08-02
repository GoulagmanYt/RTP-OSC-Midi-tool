#![allow(deprecated)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc,
    },
};

use cpal::{
    traits::{DeviceTrait, HostTrait},
    HostId,
};
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

use crate::logger::{background_log, FrontendLogger};
use crate::plugin_probe::ensure_supported_plugin_in_app;

use super::{
    callback_midi::reset_all_notes,
    device_selection::{select_device, select_host},
    engine::{db_to_linear, is_sforzando_vst3, AudioEngine},
    runtime_state::{
        AudioControls, AudioError, AudioRuntime, AudioSettings, AudioTelemetry, EditorWindow,
    },
    state_codec::{load_vst_state, save_vst_state_blocking},
    stream_runtime::build_stream,
    windows_tuning::apply_audio_process_tuning,
};

impl AudioEngine {
    pub fn stop(&self, app_handle: Option<tauri::AppHandle>) {
        let app_handle_for_drop = app_handle.clone();
        let runtime = self.runtime.lock().take();
        *self.midi_tx.lock() = None;
        self.midi_emergency_reset_requested
            .store(false, Ordering::Relaxed);

        if let Some(runtime) = runtime {
            let plugin = runtime.plugin.clone();
            let vst_path = runtime.vst_path.clone();
            let editor_window_arc = runtime.editor_window.clone();
            let is_vst3 = matches!(
                *plugin.lock(),
                super::runtime_state::PluginBackend::Vst3 { .. }
            );
            let is_sforzando = is_sforzando_vst3(&vst_path);

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

            reset_all_notes(plugin.clone());
            drop(runtime);
            save_vst_state_blocking(&plugin, &vst_path);
            if is_vst3 && !is_sforzando {
                if let Some(handle) = app_handle_for_drop {
                    let (tx, rx) = mpsc::channel();
                    let plugin_for_main_drop = plugin.clone();
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
                                "Timed out waiting for VST3 drop on main thread",
                            );
                        }
                    } else {
                        background_log("warn", "Failed to schedule VST3 drop on main thread");
                    }
                }
            }
            if is_sforzando {
                background_log(
                    "warn",
                    "Leaking sforzando VST3 instance (~5-50MB) to avoid plugin_free crash. This is a known sforzando bug."
                );
                std::mem::forget(plugin);
            } else {
                drop(plugin);
            }
        }
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
        settings: AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        self.stop(Some(logger.app_handle()));

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
        let controls = Arc::new(AudioControls {
            gain_bits: AtomicU32::new(db_to_linear(settings.gain_db).to_bits()),
            limiter_enabled: AtomicBool::new(settings.limiter_enabled),
        });
        let telemetry = Arc::new(AudioTelemetry {
            xruns: AtomicU32::new(0),
            meter_left: AtomicU32::new(0.0f32.to_bits()),
            meter_right: AtomicU32::new(0.0f32.to_bits()),
            block_size_frames: AtomicU32::new(0),
            midi_drop_count: AtomicU32::new(0),
            audio_lock_miss_count: AtomicU32::new(0),
            emergency_reset_count: AtomicU32::new(0),
            callback_last_us: AtomicU32::new(0),
            callback_max_us: AtomicU32::new(0),
            callback_over_budget_count: AtomicU32::new(0),
        });

        let mut candidates: Vec<(Option<String>, Option<String>, u8, bool)> = Vec::new();
        let preferred_backend = settings.backend.as_deref().unwrap_or("auto").to_lowercase();
        let asio_available = cpal::available_hosts().contains(&HostId::Asio);

        let push_asio = |list: &mut Vec<_>| {
            if asio_available {
                for attempt in 0..2u8 {
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

        for (backend, device_pref, attempt, prefer_low_latency) in candidates {
            if attempt > 0 {
                let delay_ms = 400u64;
                logger.debug(format!(
                    "Retrying ASIO (attempt {}/2), waiting {}ms…",
                    attempt + 1,
                    delay_ms
                ));
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }

            let backend_name = backend.as_deref().unwrap_or("auto");
            logger.info(format!("Attempting audio backend: {}", backend_name));

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

            let device = select_device(&host, device_pref.as_deref())
                .or_else(|| host.default_output_device());
            let Some(device) = device else {
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
            ) {
                Ok(res) => {
                    let (
                        stream,
                        plugin,
                        midi_tx,
                        active_sample_rate,
                        stream_buffer_size,
                        vst_midi_compatible,
                    ) = res;
                    stream_opt = Some((
                        stream,
                        plugin,
                        midi_tx,
                        active_sample_rate,
                        stream_buffer_size,
                        vst_midi_compatible,
                    ));
                    logger.info(format!(
                        "Audio stream started on '{}' ({})",
                        dev_name, backend_name
                    ));
                    selected_backend = Some(backend_name.to_string());
                    selected_device = Some(dev_name);
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

        let (stream, plugin, midi_tx, active_sample_rate, stream_buffer_size, vst_midi_compatible) =
            stream_opt.ok_or_else(|| {
                let final_err = last_err.unwrap_or_else(|| {
                    AudioError::Message("All audio backends failed to initialise.".into())
                });
                logger.error(format!("Audio initialisation failed: {}", final_err));
                final_err
            })?;

        let active_backend = selected_backend.unwrap_or_else(|| "unknown".to_string());
        let active_device = selected_device.unwrap_or_else(|| "unknown".to_string());

        load_vst_state(&plugin, &vst_path, &logger);

        let runtime = AudioRuntime {
            _stream: stream,
            plugin: plugin.clone(),
            editor_window: Arc::new(parking_lot::Mutex::new(None)),
            controls,
            telemetry,
            sample_rate: active_sample_rate,
            requested_buffer_size: settings.buffer_size,
            stream_buffer_size,
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
