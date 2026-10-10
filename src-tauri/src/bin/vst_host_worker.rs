// The isolated worker owns the VST DLL, audio driver, and native editor.
// It is always a background process: neither debug nor release launches may
// allocate a console window when the supervisor starts it.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

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
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, AtomicU32, Ordering},
            Arc,
        },
        time::Duration,
    };

    use osc_midi_bridge::{
        logger::{background_log, FrontendLogger},
        vst_worker::protocol::{
            monotonic_qpc, read_control_frame, read_midi_frame, write_control_frame,
            ControlMessage, WorkerAudioStatus, CONTROL_PROTOCOL_VERSION, HOST_ABI_VERSION,
        },
        AudioEngine,
    };
    use tauri::{Listener, Manager};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    const PIPE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    const PIPE_RETRY_DELAY: Duration = Duration::from_millis(25);
    const HEARTBEAT_PERIOD: Duration = Duration::from_millis(500);
    static WORKER_PROCESS_ID: AtomicU32 = AtomicU32::new(0);

    fn process_is_worker_descendant(process_id: u32, worker_id: u32) -> bool {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
        };

        if process_id == 0 || worker_id == 0 || process_id == worker_id {
            return process_id == worker_id;
        }
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return false;
        };
        let mut parents = HashMap::new();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
            loop {
                parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
                if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                    break;
                }
            }
        }
        let _ = unsafe { CloseHandle(snapshot) };

        let mut current = process_id;
        for _ in 0..64 {
            let Some(parent) = parents.get(&current).copied() else {
                return false;
            };
            if parent == worker_id {
                return true;
            }
            if parent == 0 || parent == current {
                return false;
            }
            current = parent;
        }
        false
    }

    unsafe extern "system" fn hide_descendant_console(
        _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
        _event: u32,
        window: windows::Win32::Foundation::HWND,
        object_id: i32,
        child_id: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetWindowThreadProcessId, ShowWindowAsync, OBJID_WINDOW, SW_HIDE,
        };

        if window.0.is_null() || object_id != OBJID_WINDOW.0 || child_id != 0 {
            return;
        }
        let mut class_name = [0u16; 64];
        let length = unsafe { GetClassNameW(window, &mut class_name) };
        if length <= 0 {
            return;
        }
        let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
        if class_name != "ConsoleWindowClass" && class_name != "CASCADIA_HOSTING_WINDOW_CLASS" {
            return;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_is_worker_descendant(process_id, WORKER_PROCESS_ID.load(Ordering::Relaxed)) {
            let _ = unsafe { ShowWindowAsync(window, SW_HIDE) };
        }
    }

    /// Some third-party plug-ins start console-subsystem helper processes while
    /// loading or shutting down. Their creation flags are outside our control,
    /// so hide only console windows belonging to descendants of this worker.
    pub fn install_descendant_console_guard() {
        WORKER_PROCESS_ID.store(std::process::id(), Ordering::Relaxed);
        let _ = std::thread::Builder::new()
            .name("vst-console-guard".into())
            .spawn(|| {
                use windows::Win32::UI::{
                    Accessibility::{SetWinEventHook, UnhookWinEvent},
                    WindowsAndMessaging::{
                        GetMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_SHOW, MSG,
                        WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
                    },
                };

                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_CREATE,
                        EVENT_OBJECT_SHOW,
                        None,
                        Some(hide_descendant_console),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if hook.0.is_null() {
                    return;
                }
                let mut message = MSG::default();
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
                let _ = unsafe { UnhookWinEvent(hook) };
            });
    }

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
            device_id: audio.current_device_id(),
            sample_rate: audio
                .current_sample_rate()
                .ok_or_else(|| "worker sample rate unavailable".to_string())?,
            requested_buffer_size: audio.requested_buffer_size().unwrap_or(0),
            stream_buffer_size: audio.current_buffer_size().unwrap_or(0),
            plugin_latency_samples: audio.plugin_latency_samples().unwrap_or(0),
            bridge_latency_samples: audio.bridge_latency_samples().unwrap_or(0),
            hosting_mode: audio
                .vst_hosting_mode()
                .unwrap_or_else(|| "directX64".to_string()),
            vst_midi_compatible: audio.vst_midi_compatible().unwrap_or(false),
            limiter_enabled: audio.limiter_enabled().unwrap_or(false),
            mmcss_enabled: audio.mmcss_enabled().unwrap_or(false),
            power_throttling_disabled: audio.power_throttling_disabled().unwrap_or(false),
            class_uid,
        })
    }

    async fn run_midi_channel(pipe_name: String, audio: AudioEngine) -> Result<(), String> {
        let mut pipe = connect_pipe(&pipe_name).await?;
        let mut received = 0usize;
        loop {
            let frame = read_midi_frame(&mut pipe)
                .await
                .map_err(|error| format!("MIDI pipe failed: {error}"))?;
            if frame.bytes() == [0xFF] {
                audio.request_emergency_midi_reset();
                continue;
            }
            audio.deliver_midi_wire(frame);
            received += 1;
            if received == 256 {
                received = 0;
                tokio::task::yield_now().await;
            }
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

    type ControlResponse = (
        ControlMessage,
        tokio::sync::oneshot::Sender<Result<(), String>>,
    );
    async fn send_response(
        tx: &tokio::sync::mpsc::Sender<ControlResponse>,
        message: ControlMessage,
    ) -> Result<(), String> {
        let (ack, received) = tokio::sync::oneshot::channel();
        tx.send((message, ack)).await.map_err(|e| e.to_string())?;
        received.await.map_err(|e| e.to_string())?
    }

    async fn worker_session(
        args: WorkerArgs,
        app: tauri::AppHandle,
        logger: FrontendLogger,
        audio: AudioEngine,
        mut editor_hidden: tokio::sync::mpsc::Receiver<()>,
    ) -> Result<(), String> {
        let control_runtime = osc_midi_bridge::realtime::MidiRuntime::new("vst-control-health")?;
        let control_name = args.control_pipe.clone();
        let mut control = control_runtime
            .spawn(async move { connect_pipe(&control_name).await })
            .await
            .map_err(|e| e.to_string())??;
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

        // read_exact-based framed reads are not cancellation-safe. Keeping one
        // dedicated reader task prevents heartbeat ticks from cancelling a
        // partially consumed frame and permanently desynchronizing the pipe.
        let (mut control_reader, mut control_writer) = tokio::io::split(control);
        let (control_event_tx, mut control_event_rx) = tokio::sync::mpsc::channel(64);
        control_runtime.spawn(async move {
            loop {
                let event = read_control_frame(&mut control_reader)
                    .await
                    .map_err(|error| format!("Control pipe failed: {error}"));
                let terminal = event.is_err();
                if control_event_tx.send(event).await.is_err() || terminal {
                    break;
                }
            }
        });

        let (control_tx, mut responses) = tokio::sync::mpsc::channel::<ControlResponse>(64);
        let (heartbeat_tx, mut heartbeats) = tokio::sync::mpsc::channel::<ControlMessage>(2);
        control_runtime.spawn(async move {
            loop {
                let frame = tokio::select! {
                    biased;
                    heartbeat = heartbeats.recv() => heartbeat.map(|frame| (frame, None)),
                    response = responses.recv() => response.map(|(frame, ack)| (frame, Some(ack))),
                };
                let Some((frame, ack)) = frame else {
                    break;
                };
                let result = match tokio::time::timeout(
                    Duration::from_secs(1),
                    write_control_frame(&mut control_writer, &frame),
                )
                .await
                {
                    Ok(result) => result.map_err(|e| e.to_string()),
                    Err(_) => Err("Control writer stalled".into()),
                };
                let failed = result.is_err();
                if let Some(ack) = ack {
                    let _ = ack.send(result);
                }
                if failed {
                    break;
                }
            }
        });
        // Publish only applied control state. Heartbeats must remain independent
        // of runtime/plugin locks, including while Load or maintenance runs.
        let applied_limiter = Arc::new(AtomicBool::new(false));
        let health_limiter = applied_limiter.clone();
        let health_audio = audio.clone();
        control_runtime.spawn(async move {
            let mut tick = tokio::time::interval(HEARTBEAT_PERIOD);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut sequence = 0u64;
            loop {
                tick.tick().await;
                sequence = sequence.wrapping_add(1);
                let mut metrics = health_audio.realtime_metrics();
                metrics.limiter_enabled = Some(health_limiter.load(Ordering::Acquire));
                let message = ControlMessage::Heartbeat {
                    sequence,
                    monotonic_qpc: monotonic_qpc(),
                    metrics,
                };
                match heartbeat_tx.try_send(message) {
                    Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        });
        let midi_runtime = osc_midi_bridge::realtime::MidiRuntime::new("vst-midi-read")?;
        let midi_audio = audio.clone();
        let midi_pipe = args.midi_pipe;
        midi_runtime.spawn(async move {
            if let Err(error) = run_midi_channel(midi_pipe, midi_audio.clone()).await {
                midi_audio.report_midi_transport_failure();
                if error.contains("early eof") || error.contains("broken pipe") {
                    background_log("debug", "VST worker MIDI pipe closed");
                } else {
                    background_log("error", error);
                }
            }
        });

        let mut heartbeat = tokio::time::interval(HEARTBEAT_PERIOD);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    let heartbeat_metrics = audio.realtime_metrics();
                    if let Some(reason) = audio.stream_fault_reason() {
                        let audio_for_save = audio.clone();
                        let app_for_save = app.clone();
                        if let Err(error) = execute_blocking(move || {
                            audio_for_save
                                .checkpoint_vst_state(app_for_save)
                                .map_err(|error| error.to_string())
                        }).await {
                            background_log(
                                "warn",
                                format!("State checkpoint before audio recovery failed: {error}"),
                            );
                        }
                        return Err(format!("Audio stream recovery requested: {reason}"));
                    }
                    if audio.vst_hosting_mode().as_deref() == Some("bridgedX86")
                        && !heartbeat_metrics.x86_worker_alive
                    {
                        #[cfg(target_pointer_width = "64")]
                        let detail = audio
                            .x86_bridge_fault_reason()
                            .unwrap_or_else(|| "health check failed without a bridge diagnostic".to_string());
                        #[cfg(target_pointer_width = "32")]
                        let detail = "health check failed without a bridge diagnostic".to_string();
                        return Err(format!("Nested x86 DSP worker failed: {detail}"));
                    }
                }
                incoming = control_event_rx.recv() => {
                    let message = incoming
                        .ok_or_else(|| "Control pipe reader exited".to_string())??;
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
                                        Ok(status) => {
                                            applied_limiter.store(status.limiter_enabled, Ordering::Release);
                                            ControlMessage::Ready { request_id, status }
                                        },
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
                            match audio.set_gain(gain_db) {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.gain-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::SetLimiter { request_id, enabled } => {
                            match audio.set_limiter_enabled(enabled) {
                                Ok(()) => {
                                    applied_limiter.store(enabled, Ordering::Release);
                                    ControlMessage::Ack { request_id }
                                },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.limiter-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::SetEq { request_id, settings } => {
                            match audio.set_vst_eq(settings) {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.eq-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::Panic { request_id } => {
                            match audio.panic_all_notes() {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(error) => ControlMessage::Error { request_id: Some(request_id), code: "worker.panic-failed".into(), message: error.to_string() },
                            }
                        }
                        ControlMessage::Ping { sequence } => ControlMessage::Pong { sequence },
                        ControlMessage::SetGameMode { request_id, enabled } => {
                            let audio = audio.clone(); let app = app.clone();
                            match execute_blocking(move || audio.set_game_mode(enabled, app).map_err(|error| error.to_string())).await {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(message) => ControlMessage::Error { request_id: Some(request_id), code: "worker.game-mode-failed".into(), message },
                            }
                        },
                        ControlMessage::SaveState { request_id } => {
                            let audio = audio.clone();
                            let app = app.clone();
                            match execute_blocking(move || audio.checkpoint_vst_state(app).map_err(|error| error.to_string())).await {
                                Ok(()) => ControlMessage::Ack { request_id },
                                Err(message) => ControlMessage::Error { request_id: Some(request_id), code: "worker.state-save-failed".into(), message },
                            }
                        },
                        ControlMessage::Stop { request_id } => {
                            let audio = audio.clone();
                            let app_for_stop = app.clone();
                            execute_blocking(move || {
                                audio.stop(Some(app_for_stop));
                                Ok(())
                            }).await?;
                            send_response(&control_tx, ControlMessage::Stopped { request_id })
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
                    send_response(&control_tx, response)
                        .await
                        .map_err(|error| format!("Failed to send worker response: {error}"))?;
                }
                hidden = editor_hidden.recv() => {
                    if hidden.is_none() {
                        continue;
                    }
                    let audio_for_save = audio.clone();
                    let app_for_save = app.clone();
                    if let Err(error) = execute_blocking(move || {
                        audio_for_save
                            .checkpoint_vst_state(app_for_save)
                            .map_err(|error| error.to_string())
                    }).await {
                        background_log("warn", format!("VST editor-close state save failed: {error}"));
                    }
                    send_response(&control_tx, ControlMessage::EditorHidden)
                        .await
                        .map_err(|error| format!("Failed to report hidden editor: {error}"))?;
                }
            }
        }
    }

    pub fn run() -> Result<(), String> {
        let dsp_mode = std::env::args().any(|arg| arg == "--dsp-bridge");
        let args = if dsp_mode {
            None
        } else {
            Some(WorkerArgs::parse()?)
        };
        env_logger::init();

        // The shared Tauri configuration declares OSCMidi's main window as
        // visible. A worker using that context would briefly paint the same
        // window before `setup` could hide it on every VST switch. Override the
        // worker context before Tauri creates any native window.
        let mut context = tauri::generate_context!();
        for window in &mut context.config_mut().app.windows {
            window.visible = false;
        }

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
                if dsp_mode {
                    tauri::async_runtime::spawn_blocking(move || {
                        let result = osc_midi_bridge::audio::run_dsp_bridge_from_env(logger)
                            .unwrap_or_else(|| Err("Missing DSP bridge arguments".to_string()));
                        if let Err(error) = result {
                            background_log("error", format!("VST x86 DSP bridge stopped: {error}"));
                            exit_handle.exit(1);
                        } else {
                            exit_handle.exit(0);
                        }
                    });
                    return Ok(());
                }
                let args = args.expect("normal worker arguments");
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
            .run(context)
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "windows")]
fn main() {
    osc_midi_bridge::audio::windows_tuning::reapply_process_priority_on_focus_loss();
    windows_worker::install_descendant_console_guard();
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
