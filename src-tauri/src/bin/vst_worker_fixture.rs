#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
            sample_rate: settings.sample_rate,
            requested_buffer_size: settings.buffer_size,
            stream_buffer_size: settings.buffer_size,
            plugin_latency_samples: 0,
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
                protocol_version: CONTROL_PROTOCOL_VERSION,
                host_abi_version: HOST_ABI_VERSION,
                session_id: args.session_id,
                token: args.token,
                worker_pid: std::process::id(),
            },
        )
        .await
        .map_err(|error| error.to_string())?;

        tokio::spawn(async move { while read_midi_frame(&mut midi).await.is_ok() {} });

        let mut heartbeat = tokio::time::interval(Duration::from_millis(100));
        let mut sequence = 0u64;
        let mut hang_deadline = None;
        loop {
            if hang_deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                std::future::pending::<()>().await;
            }
            tokio::select! {
                _ = heartbeat.tick() => {
                    sequence = sequence.wrapping_add(1);
                    write_control_frame(&mut control, &ControlMessage::Heartbeat {
                        sequence,
                        monotonic_qpc: monotonic_qpc(),
                        metrics: WorkerMetrics::default(),
                    }).await.map_err(|error| error.to_string())?;
                }
                message = read_control_frame(&mut control) => {
                    let message = message.map_err(|error| error.to_string())?;
                    let response = match message {
                        ControlMessage::Load { request_id, settings, class_uid, .. } => {
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
                            write_control_frame(&mut control, &ControlMessage::Stopped { request_id })
                                .await.map_err(|error| error.to_string())?;
                            return Ok(());
                        }
                        ControlMessage::ListParameters { request_id } => ControlMessage::Parameters { request_id, parameters: Vec::new() },
                        ControlMessage::Ping { sequence } => ControlMessage::Pong { sequence },
                        ControlMessage::OpenEditor { request_id }
                        | ControlMessage::CloseEditor { request_id }
                        | ControlMessage::SetParameter { request_id, .. }
                        | ControlMessage::SetGain { request_id, .. }
                        | ControlMessage::SetLimiter { request_id, .. }
                        | ControlMessage::SaveState { request_id }
                        | ControlMessage::Panic { request_id } => ControlMessage::Ack { request_id },
                        _ => ControlMessage::Error { request_id: None, code: "fixture.invalid-command".into(), message: "invalid fixture command".into() },
                    };
                    write_control_frame(&mut control, &response).await.map_err(|error| error.to_string())?;
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() {
    if fixture::run().await.is_err() {
        std::process::exit(1);
    }
}
