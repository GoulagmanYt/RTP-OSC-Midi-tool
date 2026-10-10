#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(not(target_os = "windows"))]
fn main() {}

#[cfg(target_os = "windows")]
mod fixture {
    use std::{collections::HashMap, time::Duration};

    use osc_midi_bridge::{
        audio::AudioSettings,
        vst_worker::protocol::{
            monotonic_qpc, read_control_frame, read_midi_frame, write_control_frame,
            ControlMessage, WorkerAudioStatus, WorkerMetrics, CONTROL_PROTOCOL_VERSION,
            HOST_ABI_VERSION,
        },
    };
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    struct Args {
        control_pipe: String,
        midi_pipe: String,
        session_id: String,
        token: String,
    }

    impl Args {
        fn parse() -> Result<Self, String> {
            let mut values = HashMap::new();
            let mut args = std::env::args().skip(1);
            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                values.insert(flag, value);
            }
            let required = |name: &str| {
                values
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("missing {name}"))
            };
            Ok(Self {
                control_pipe: required("--control-pipe")?,
                midi_pipe: required("--midi-pipe")?,
                session_id: required("--session-id")?,
                token: required("--token")?,
            })
        }
    }

    async fn connect(name: &str) -> Result<NamedPipeClient, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match ClientOptions::new().open(name) {
                Ok(pipe) => return Ok(pipe),
                Err(error) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    if error.raw_os_error().is_none() {
                        return Err(error.to_string());
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    fn ready(settings: &AudioSettings, class_uid: Option<String>) -> WorkerAudioStatus {
        WorkerAudioStatus {
            backend: settings.backend.clone().unwrap_or_else(|| "asio".into()),
            device: settings.device.clone().unwrap_or_else(|| "fixture".into()),
            device_id: settings.device_id.clone(),
            sample_rate: settings.sample_rate,
            requested_buffer_size: settings.buffer_size,
            stream_buffer_size: settings.buffer_size,
            plugin_latency_samples: 0,
            bridge_latency_samples: 0,
            hosting_mode: "directX64".into(),
            vst_midi_compatible: true,
            limiter_enabled: settings.limiter_enabled,
            mmcss_enabled: true,
            power_throttling_disabled: true,
            class_uid,
        }
    }

    pub async fn run() -> Result<(), String> {
        let args = Args::parse()?;
        let mode = std::env::var("OSCMIDI_VST_FIXTURE_MODE").unwrap_or_else(|_| "ready".into());
        let mut control = connect(&args.control_pipe).await?;
        let mut midi = connect(&args.midi_pipe).await?;
        write_control_frame(
            &mut control,
            &ControlMessage::Hello {
                protocol_version: if mode == "legacy" {
                    8
                } else {
                    CONTROL_PROTOCOL_VERSION
                },
                host_abi_version: HOST_ABI_VERSION,
                session_id: args.session_id,
                token: args.token,
                worker_pid: std::process::id(),
            },
        )
        .await
        .map_err(|error| error.to_string())?;

        // A framed read must never be cancelled after it consumed a prefix.
        // Use a dedicated reader and deliver only complete frames to the timer
        // loop, mirroring the production worker.
        let (mut control_reader, mut control_writer) = tokio::io::split(control);
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                let event = read_control_frame(&mut control_reader)
                    .await
                    .map_err(|error| error.to_string());
                let terminal = event.is_err();
                if control_tx.send(event).await.is_err() || terminal {
                    break;
                }
            }
        });

        tokio::spawn(async move { while read_midi_frame(&mut midi).await.is_ok() {} });

        let mut heartbeat = tokio::time::interval(Duration::from_millis(100));
        let mut sequence = 0u64;
        let mut hang_deadline = None;
        let mut silence_until = None;
        let mut slow_ack = None;
        let mut loaded = false;
        let mut game_mode = false;
        let mut limiter_enabled = false;
        let mut editor_open = false;
        let mut deferred_editor = None;
        loop {
            if hang_deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                std::future::pending::<()>().await;
            }
            tokio::select! {
                _ = heartbeat.tick() => {
                    sequence = sequence.wrapping_add(1);
                    if let Some((deadline, request_id)) = slow_ack {
                        if tokio::time::Instant::now() >= deadline {
                            write_control_frame(&mut control_writer, &ControlMessage::Ack { request_id }).await.map_err(|e| e.to_string())?;
                            slow_ack = None;
                        }
                    }
                    if silence_until.is_some_and(|deadline| tokio::time::Instant::now() < deadline) { continue; }
                    write_control_frame(&mut control_writer, &ControlMessage::Heartbeat {
                        sequence,
                        monotonic_qpc: monotonic_qpc(),
                        metrics: WorkerMetrics { callback_count: if loaded && mode != "callback-blocked" { sequence } else { 0 }, game_mode, limiter_enabled: Some(limiter_enabled), editor_open, deferred_editor, ..WorkerMetrics::default() },
                    }).await.map_err(|error| error.to_string())?;
                }
                message = control_rx.recv() => {
                    let message = message
                        .ok_or_else(|| "control reader exited".to_string())??;
                    let response = match message {
                        ControlMessage::Load { request_id, settings, class_uid, .. } => {
                            if mode == "recover-controls" {
                                if let Ok(expected) = std::env::var("OSCMIDI_VST_FIXTURE_EXPECT_SETTINGS") {
                                    let expected: AudioSettings = serde_json::from_str(&expected).map_err(|e| e.to_string())?;
                                    if serde_json::to_value(&settings).unwrap() != serde_json::to_value(&expected).unwrap() {
                                        return Err("recovery settings differ from admitted live controls".into());
                                    }
                                }
                            }
                            loaded = true;
                            game_mode = settings.game_mode;
                            limiter_enabled = settings.limiter_enabled;
                            if mode == "silence-two-seconds" { silence_until = Some(tokio::time::Instant::now() + Duration::from_secs(2)); }
                            match mode.as_str() {
                                "crash" => std::process::abort(),
                                "hang" => {
                                    hang_deadline = Some(tokio::time::Instant::now() + Duration::from_millis(100));
                                    ControlMessage::Ready { request_id, status: ready(&settings, class_uid) }
                                }
                                _ => ControlMessage::Ready { request_id, status: ready(&settings, class_uid) },
                            }
                        }
                        ControlMessage::Stop { request_id } => {
                            write_control_frame(&mut control_writer, &ControlMessage::Stopped { request_id })
                                .await.map_err(|error| error.to_string())?;
                            return Ok(());
                        }
                        ControlMessage::ListParameters { request_id } => ControlMessage::Parameters { request_id, parameters: Vec::new() },
                        ControlMessage::Ping { sequence } => ControlMessage::Pong { sequence },
                        ControlMessage::SaveState { request_id } if mode == "slow-command" => {
                            slow_ack = Some((tokio::time::Instant::now() + Duration::from_secs(3), request_id));
                            continue;
                        }
                        ControlMessage::SaveState { .. } if mode == "recover-controls" => std::process::exit(23),
                        ControlMessage::SetGameMode { request_id, enabled } => {
                            game_mode = enabled;
                            if enabled { editor_open = false; }
                            else if let Some(open) = deferred_editor.take() { editor_open = open; }
                            ControlMessage::Ack { request_id }
                        }
                        ControlMessage::OpenEditor { request_id } => {
                            if game_mode { deferred_editor = Some(true); }
                            else { editor_open = true; }
                            ControlMessage::Ack { request_id }
                        }
                        ControlMessage::CloseEditor { request_id } => {
                            if game_mode { deferred_editor = Some(false); }
                            else { editor_open = false; }
                            ControlMessage::Ack { request_id }
                        }
                        ControlMessage::SetParameter { request_id, .. }
                        | ControlMessage::SetGain { request_id, .. }
                        | ControlMessage::SetEq { request_id, .. }
                        | ControlMessage::SaveState { request_id }
                        | ControlMessage::Panic { request_id } => ControlMessage::Ack { request_id },
                        ControlMessage::SetLimiter { request_id, enabled } => {
                            if mode == "reject-limiter" {
                                ControlMessage::Error {
                                    request_id: Some(request_id),
                                    code: "fixture.limiter-rejected".into(),
                                    message: "fixture rejected limiter change".into(),
                                }
                            } else {
                                limiter_enabled = enabled;
                                ControlMessage::Ack { request_id }
                            }
                        }
                        _ => ControlMessage::Error { request_id: None, code: "fixture.invalid-command".into(), message: "invalid fixture command".into() },
                    };
                    write_control_frame(&mut control_writer, &response).await.map_err(|error| error.to_string())?;
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() {
    if let Err(error) = fixture::run().await {
        eprintln!("VST worker fixture failed: {error}");
        std::process::exit(1);
    }
}
