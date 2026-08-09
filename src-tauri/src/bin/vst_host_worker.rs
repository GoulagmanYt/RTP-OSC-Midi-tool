// The isolated worker owns the VST DLL, audio driver, and native editor.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "windows"))]
fn main() {
    if let Some(exit_code) = osc_midi_bridge::plugin_probe::probe_cli_exit_code() {
        std::process::exit(exit_code);
    }
    eprintln!("vst-host-worker is supported on Windows x64 only");
    std::process::exit(2);
}

#[cfg(target_os = "windows")]
mod windows_worker {
    use std::{collections::HashMap, time::Duration};

    use osc_midi_bridge::{
        logger::{background_log, FrontendLogger},
        vst_worker::protocol::{
            monotonic_qpc, qpc_elapsed_us, read_control_frame, read_midi_frame,
            write_control_frame, ControlMessage, WorkerAudioStatus, WorkerMetrics,
            CONTROL_PROTOCOL_VERSION, HOST_ABI_VERSION,
        },
        AudioEngine,
    };
    use tauri::{Listener, Manager};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    const PIPE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    const PIPE_RETRY_DELAY: Duration = Duration::from_millis(25);
    const HEARTBEAT_PERIOD: Duration = Duration::from_millis(500);

    #[derive(Clone)]
    struct WorkerArgs {
        control_pipe: String,
        midi_pipe: String,
        session_id: String,
        token: String,
    }

    impl WorkerArgs {
        fn parse() -> Result<Self, String> {
            let mut values = HashMap::new();
            let mut args = std::env::args().skip(1);
            while let Some(flag) = args.next() {
                let Some(value) = args.next() else {
                    return Err(format!("Missing value for {flag}"));
                };
                values.insert(flag, value);
            }
            let take = |name: &str| {
                values
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("Missing required argument {name}"))
            };
            Ok(Self {
                control_pipe: take("--control-pipe")?,
                midi_pipe: take("--midi-pipe")?,
                session_id: take("--session-id")?,
                token: take("--token")?,
            })
        }
    }

    async fn connect_pipe(name: &str) -> Result<NamedPipeClient, String> {
        let deadline = tokio::time::Instant::now() + PIPE_CONNECT_TIMEOUT;
        loop {
            match ClientOptions::new().open(name) {
                Ok(client) => return Ok(client),
                Err(error) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(PIPE_RETRY_DELAY).await;
                    if error.raw_os_error().is_none() {
                        return Err(format!("Failed to connect to pipe {name}: {error}"));
                    }
                }
                Err(error) => {
                    return Err(format!("Timed out connecting to pipe {name}: {error}"));
                }
            }
        }
    }

    fn metrics(audio: &AudioEngine) -> WorkerMetrics {
        let (audio_peak_l, audio_peak_r) = audio.peak_levels().unwrap_or((0.0, 0.0));
        WorkerMetrics {
            audio_peak_l,
            audio_peak_r,
            audio_xruns: audio.xrun_count().unwrap_or(0),
            audio_midi_drops: audio.midi_drop_count().unwrap_or(0),
            audio_lock_misses: audio.audio_lock_miss_count().unwrap_or(0),
            audio_emergency_resets: audio.emergency_reset_count().unwrap_or(0),
            callback_last_us: audio.callback_last_us().unwrap_or(0),
            callback_max_us: audio.callback_max_us().unwrap_or(0),
            callback_over_budget_count: audio.callback_over_budget_count().unwrap_or(0),
            consecutive_deadline_misses: audio.consecutive_deadline_misses().unwrap_or(0),
            dsp_process_last_us: audio.dsp_process_last_us().unwrap_or(0),
            dsp_process_p95_us: audio.dsp_process_percentile_us(95).unwrap_or(0),
            dsp_process_p99_us: audio.dsp_process_percentile_us(99).unwrap_or(0),
            dsp_process_max_us: audio.dsp_process_max_us().unwrap_or(0),
            midi_queue_depth: audio.audio_midi_queue_depth().unwrap_or(0),
            midi_queue_max_depth: audio.audio_midi_queue_max_depth().unwrap_or(0),
            midi_oldest_us: audio.audio_midi_oldest_us().unwrap_or(0),
        }
    }

    fn ready_status(
        audio: &AudioEngine,
        class_uid: Option<String>,
    ) -> Result<WorkerAudioStatus, String> {
        Ok(WorkerAudioStatus {
            backend: audio
                .current_backend()
                .ok_or_else(|| "worker audio backend unavailable".to_string())?,
            device: audio
                .current_device()
                .ok_or_else(|| "worker audio device unavailable".to_string())?,
            sample_rate: audio
                .current_sample_rate()
                .ok_or_else(|| "worker sample rate unavailable".to_string())?,
            requested_buffer_size: audio.requested_buffer_size().unwrap_or(0),
            stream_buffer_size: audio.current_buffer_size().unwrap_or(0),
            plugin_latency_samples: audio.plugin_latency_samples().unwrap_or(0),
            vst_midi_compatible: audio.vst_midi_compatible().unwrap_or(false),
            limiter_enabled: audio.limiter_enabled().unwrap_or(false),
            mmcss_enabled: audio.mmcss_enabled().unwrap_or(false),
            power_throttling_disabled: audio.power_throttling_disabled().unwrap_or(false),
            class_uid,
        })
    }

    async fn run_midi_channel(pipe_name: String, audio: AudioEngine) -> Result<(), String> {
        let mut pipe = connect_pipe(&pipe_name).await?;
        loop {
            let frame = read_midi_frame(&mut pipe)
                .await
                .map_err(|error| format!("MIDI pipe failed: {error}"))?;
            let age_us = qpc_elapsed_us(frame.monotonic_qpc, monotonic_qpc());
            audio.send_midi_with_age(frame.bytes(), age_us);
        }
    }

    async fn execute_blocking<T, F>(operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        tauri::async_runtime::spawn_blocking(operation)
            .await
            .map_err(|error| format!("worker task failed: {error}"))?
    }

    async fn worker_session(
        args: WorkerArgs,
        app: tauri::AppHandle,
        logger: FrontendLogger,
        audio: AudioEngine,
        mut editor_hidden: tokio::sync::mpsc::Receiver<()>,
    ) -> Result<(), String> {
        let mut control = connect_pipe(&args.control_pipe).await?;
        write_control_frame(
            &mut control,
            &ControlMessage::Hello {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                host_abi_version: HOST_ABI_VERSION,
                session_id: args.session_id,
                token: args.token,
                worker_pid: std::process::id(),
            },
        )
        .await
        .map_err(|error| format!("Failed to send worker hello: {error}"))?;

        let midi_audio = audio.clone();
        let midi_pipe = args.midi_pipe;
        tauri::async_runtime::spawn(async move {
            if let Err(error) = run_midi_channel(midi_pipe, midi_audio).await {
                if error.contains("early eof") || error.contains("broken pipe") {
                    background_log("debug", "VST worker MIDI pipe closed");
                } else {
                    background_log("error", error);
                }
            }
        });

        let mut heartbeat = tokio::time::interval(HEARTBEAT_PERIOD);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut heartbeat_sequence = 0u64;

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    heartbeat_sequence = heartbeat_sequence.wrapping_add(1);
                    write_control_frame(
                        &mut control,
                        &ControlMessage::Heartbeat {
                            sequence: heartbeat_sequence,
                            monotonic_qpc: monotonic_qpc(),
                            metrics: metrics(&audio),
                        },
                    )
                    .await
                    .map_err(|error| format!("Heartbeat failed: {error}"))?;
                }
                incoming = read_control_frame(&mut control) => {
                    let message = incoming.map_err(|error| format!("Control pipe failed: {error}"))?;
                    let response = match message {
                        ControlMessage::Load { request_id, settings, class_uid, warmup_ms } => {
                            let audio_for_load = audio.clone();
                            let logger_for_load = logger.clone();
                            let load = execute_blocking(move || {
                                audio_for_load
                                    .start_worker_runtime(settings, None, logger_for_load)
                                    .map_err(|error| error.to_string())
                            }).await;
                            match load {
                                Ok(()) => {
                                    tokio::time::sleep(Duration::from_millis(u64::from(warmup_ms.clamp(500, 2_000)))).await;
                                    match ready_status(&audio, class_uid) {
                                        Ok(status) => ControlMessage::Ready { request_id, status },
                                        Err(message) => ControlMessage::Error { request_id: Some(request_id), code: "worker.ready-failed".into(), message },
                                    }
                                }
                                Err(message) => ControlMessage::Error { request_id: Some(request_id), code: "worker.load-failed".into(), message },
                            }
                        }
                        ControlMessage::OpenEditor { request_id } => {
                            let audio = audio.clone();
                            let app = app.clone();
                            match execute_blocking(move || audio.open_vst_ui(app).map_err(|error| error.to_string())).await {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(message) => ControlMessage::Error { request_id: Some(request_id), code: "worker.editor-open-failed".into(), message },
                            }
                        }
                        ControlMessage::CloseEditor { request_id } => {
                            let audio = audio.clone();
                            let app = app.clone();
                            match execute_blocking(move || audio.close_vst_ui(app).map_err(|error| error.to_string())).await {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(message) => ControlMessage::Error { request_id: Some(request_id), code: "worker.editor-close-failed".into(), message },
                            }
                        }
                        ControlMessage::ListParameters { request_id } => {
                            match audio.list_vst_parameters() {
                                Ok(parameters) => ControlMessage::Parameters { request_id, parameters },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.parameters-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::SetParameter { request_id, index, value } => {
                            match audio.set_vst_parameter(index, value) {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.parameter-set-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::SetGain { request_id, gain_db } => {
                            audio.set_gain(gain_db);
                            ControlMessage::Ack { request_id }
                        }
                        ControlMessage::SetLimiter { request_id, enabled } => {
                            audio.set_limiter_enabled(enabled);
                            ControlMessage::Ack { request_id }
                        }
                        ControlMessage::Panic { request_id } => {
                            match audio.panic_all_notes() {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.panic-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::Ping { sequence } => ControlMessage::Pong { sequence },
                        ControlMessage::SaveState { request_id } => ControlMessage::Error {
                            request_id: Some(request_id),
                            code: "worker.save-requires-stop".into(),
                            message: "State capture is performed during the serialized worker stop/reload transition".into(),
                        },
                        ControlMessage::Stop { request_id } => {
                            let audio = audio.clone();
                            let app_for_stop = app.clone();
                            execute_blocking(move || {
                                audio.stop(Some(app_for_stop));
                                Ok(())
                            }).await?;
                            write_control_frame(&mut control, &ControlMessage::Stopped { request_id })
                                .await
                                .map_err(|error| format!("Failed to acknowledge stop: {error}"))?;
                            app.exit(0);
                            return Ok(());
                        }
                        _ => ControlMessage::Error {
                            request_id: None,
                            code: "worker.invalid-direction".into(),
                            message: "Message is not a supervisor command".into(),
                        },
                    };
                    write_control_frame(&mut control, &response)
                        .await
                        .map_err(|error| format!("Failed to send worker response: {error}"))?;
                }
                hidden = editor_hidden.recv() => {
                    if hidden.is_none() {
                        continue;
                    }
                    write_control_frame(&mut control, &ControlMessage::EditorHidden)
                        .await
                        .map_err(|error| format!("Failed to report hidden editor: {error}"))?;
                }
            }
        }
    }

    pub fn run() -> Result<(), String> {
        let args = WorkerArgs::parse()?;
        env_logger::init();

        tauri::Builder::default()
            .setup(move |app| {
                let window = app
                    .get_webview_window("main")
                    .ok_or_else(|| "worker host window unavailable".to_string())?;
                window.hide()?;
                let logger = FrontendLogger::new(
                    window.as_ref().window().clone(),
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                );
                let app_handle = app.handle().clone();
                let exit_handle = app_handle.clone();
                let audio = AudioEngine::new();
                let (editor_hidden_tx, editor_hidden_rx) = tokio::sync::mpsc::channel(4);
                app_handle.listen("audio:vst-editor-hidden", move |_| {
                    let _ = editor_hidden_tx.try_send(());
                });
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        worker_session(args, app_handle, logger, audio, editor_hidden_rx).await
                    {
                        background_log("error", format!("VST worker stopped: {error}"));
                        exit_handle.exit(1);
                    }
                });
                Ok(())
            })
            .run(tauri::generate_context!())
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "windows")]
fn main() {
    if let Some(exit_code) = osc_midi_bridge::plugin_probe::probe_cli_exit_code() {
        if exit_code == 0 {
            return;
        }
        std::process::exit(exit_code);
    }
    if let Err(error) = windows_worker::run() {
        eprintln!("vst-host-worker: {error}");
        std::process::exit(2);
    }
}
