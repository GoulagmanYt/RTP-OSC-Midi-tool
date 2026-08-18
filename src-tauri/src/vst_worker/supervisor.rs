use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::{mpsc, oneshot},
};
use uuid::Uuid;

use crate::{audio::AudioSettings, logger::background_log};

use super::protocol::{
    monotonic_qpc, read_control_frame, write_control_frame, write_midi_frame, ControlMessage,
    MidiWireFrame, WorkerAudioStatus, WorkerMetrics, CONTROL_PROTOCOL_VERSION, HOST_ABI_VERSION,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const LOAD_TIMEOUT: Duration = Duration::from_secs(45);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_millis(1_500);
const PROCESS_POLL_PERIOD: Duration = Duration::from_millis(100);
const MIDI_PIPE_CAPACITY: usize = 16_384;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const MAX_RESTARTS_PER_WINDOW: usize = 3;
const CONTROL_PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const MIDI_PIPE_BUFFER_BYTES: u32 = 256 * 1024;
const STATE_AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum VstWorkerState {
    #[default]
    Disabled,
    Starting,
    Loading,
    Ready,
    Stopping,
    Faulted,
}

impl VstWorkerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Starting => "starting",
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Faulted => "faulted",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SupervisorSnapshot {
    pub state: VstWorkerState,
    pub restarts: u32,
    pub last_exit: Option<String>,
    pub status: Option<WorkerAudioStatus>,
    pub metrics: WorkerMetrics,
    pub midi_drops: u32,
    pub editor_open: bool,
}

#[derive(Default)]
struct SupervisorStatus {
    snapshot: SupervisorSnapshot,
    last_heartbeat: Option<Instant>,
}

enum SessionCommand {
    Request {
        message: ControlMessage,
        response: oneshot::Sender<Result<ControlMessage, String>>,
    },
    Notify(ControlMessage),
    Shutdown,
}

struct ActiveSession {
    commands: mpsc::Sender<SessionCommand>,
    midi: mpsc::Sender<MidiWireFrame>,
}

struct SupervisorInner {
    status: Mutex<SupervisorStatus>,
    session: Mutex<Option<ActiveSession>>,
    request_sequence: AtomicU64,
    midi_sequence: AtomicU64,
    state_generation: AtomicU64,
    last_load: Mutex<Option<(AudioSettings, Option<String>)>>,
    restart_history: Mutex<VecDeque<Instant>>,
}

#[derive(Clone)]
pub struct VstWorkerSupervisor {
    inner: Arc<SupervisorInner>,
}

impl Default for VstWorkerSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl VstWorkerSupervisor {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SupervisorInner {
                status: Mutex::new(SupervisorStatus::default()),
                session: Mutex::new(None),
                request_sequence: AtomicU64::new(1),
                midi_sequence: AtomicU64::new(1),
                state_generation: AtomicU64::new(0),
                last_load: Mutex::new(None),
                restart_history: Mutex::new(VecDeque::new()),
            }),
        }
    }

    pub fn snapshot(&self) -> SupervisorSnapshot {
        self.inner.status.lock().snapshot.clone()
    }

    pub fn is_ready(&self) -> bool {
        self.snapshot().state == VstWorkerState::Ready
    }

    pub async fn start(
        &self,
        settings: AudioSettings,
        class_uid: Option<String>,
    ) -> Result<WorkerAudioStatus, String> {
        *self.inner.last_load.lock() = Some((settings.clone(), class_uid.clone()));
        self.stop().await.ok();
        self.set_state(VstWorkerState::Starting);

        let credentials = SessionCredentials::new();
        let control_server =
            create_pipe_server(&credentials.control_pipe, CONTROL_PIPE_BUFFER_BYTES)?;
        let midi_server = create_pipe_server(&credentials.midi_pipe, MIDI_PIPE_BUFFER_BYTES)?;
        let child = match spawn_worker(&credentials) {
            Ok(child) => child,
            Err(error) => {
                self.fault(error.clone());
                return Err(error);
            }
        };

        let (command_tx, command_rx) = mpsc::channel(64);
        let (midi_tx, midi_rx) = mpsc::channel(MIDI_PIPE_CAPACITY);
        let (connected_tx, connected_rx) = oneshot::channel();
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            run_session(SessionRuntime {
                credentials,
                control: control_server,
                midi_pipe: midi_server,
                child,
                commands: command_rx,
                midi: midi_rx,
                connected: connected_tx,
                inner,
            })
            .await;
        });

        match tokio::time::timeout(CONNECT_TIMEOUT, connected_rx).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                self.fault(error.clone());
                return Err(error);
            }
            Ok(Err(_)) => {
                let error = "worker connection task exited before authentication".to_string();
                self.fault(error.clone());
                return Err(error);
            }
            Err(_) => {
                let error = "timed out authenticating VST worker".to_string();
                self.fault(error.clone());
                return Err(error);
            }
        }

        *self.inner.session.lock() = Some(ActiveSession {
            commands: command_tx,
            midi: midi_tx,
        });
        self.set_state(VstWorkerState::Loading);

        let request_id = self.next_request_id();
        let response = match self
            .request_with_timeout(
                ControlMessage::Load {
                    request_id,
                    settings,
                    class_uid,
                    warmup_ms: 500,
                },
                LOAD_TIMEOUT,
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                self.fault(error.clone());
                return Err(error);
            }
        };
        match response {
            ControlMessage::Ready { status, .. } => {
                let mut guard = self.inner.status.lock();
                guard.snapshot.status = Some(status.clone());
                guard.snapshot.state = VstWorkerState::Ready;
                Ok(status)
            }
            ControlMessage::Error { message, .. } => {
                self.fault(message.clone());
                Err(message)
            }
            other => {
                let error = format!("unexpected worker load response: {other:?}");
                self.fault(error.clone());
                Err(error)
            }
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        let state = self.snapshot().state;
        if matches!(state, VstWorkerState::Disabled) && self.inner.session.lock().is_none() {
            return Ok(());
        }
        self.set_state(VstWorkerState::Stopping);
        let session = self.inner.session.lock().take();
        if let Some(session) = session {
            let request_id = self.next_request_id();
            let (response_tx, response_rx) = oneshot::channel();
            session
                .commands
                .send(SessionCommand::Request {
                    message: ControlMessage::Stop { request_id },
                    response: response_tx,
                })
                .await
                .map_err(|_| "worker control channel is closed".to_string())?;
            match tokio::time::timeout(COMMAND_TIMEOUT, response_rx).await {
                Ok(Ok(Ok(ControlMessage::Stopped { .. }))) => {}
                Ok(Ok(Ok(other))) => {
                    background_log(
                        "warn",
                        format!("Unexpected worker stop response: {other:?}"),
                    );
                }
                Ok(Ok(Err(error))) => return Err(error),
                Ok(Err(_)) => return Err("worker stop response channel closed".into()),
                Err(_) => return Err("worker did not stop within 10 seconds".into()),
            }
            let _ = session.commands.send(SessionCommand::Shutdown).await;
        }
        self.set_state(VstWorkerState::Disabled);
        Ok(())
    }

    pub fn try_send_midi(&self, bytes: &[u8]) -> bool {
        let frame = MidiWireFrame::new(
            self.inner.midi_sequence.fetch_add(1, Ordering::Relaxed),
            monotonic_qpc(),
            bytes,
        );
        let Some(frame) = frame else {
            return false;
        };
        let sent = self
            .inner
            .session
            .lock()
            .as_ref()
            .is_some_and(|session| session.midi.try_send(frame).is_ok());
        if !sent {
            let mut guard = self.inner.status.lock();
            guard.snapshot.midi_drops = guard.snapshot.midi_drops.saturating_add(1);
        }
        sent
    }

    pub async fn open_editor(&self) -> Result<(), String> {
        self.expect_ack(ControlMessage::OpenEditor {
            request_id: self.next_request_id(),
        })
        .await?;
        self.inner.status.lock().snapshot.editor_open = true;
        Ok(())
    }

    pub async fn close_editor(&self) -> Result<(), String> {
        self.expect_ack(ControlMessage::CloseEditor {
            request_id: self.next_request_id(),
        })
        .await?;
        self.inner.status.lock().snapshot.editor_open = false;
        self.save_state().await?;
        Ok(())
    }

    pub async fn set_parameter(&self, index: usize, value: f32) -> Result<(), String> {
        self.expect_ack(ControlMessage::SetParameter {
            request_id: self.next_request_id(),
            index,
            value,
        })
        .await?;
        self.schedule_state_autosave();
        Ok(())
    }

    fn schedule_state_autosave(&self) {
        let generation = self
            .inner
            .state_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let supervisor = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(STATE_AUTOSAVE_DEBOUNCE).await;
            if supervisor.inner.state_generation.load(Ordering::Acquire) != generation
                || !supervisor.is_ready()
            {
                return;
            }
            if let Err(error) = supervisor.save_state().await {
                background_log("warn", format!("VST parameter autosave failed: {error}"));
            }
        });
    }

    pub fn try_set_gain(&self, gain_db: f32) -> bool {
        self.try_notify(ControlMessage::SetGain {
            request_id: self.next_request_id(),
            gain_db,
        })
    }

    pub fn try_set_limiter_enabled(&self, enabled: bool) -> bool {
        self.try_notify(ControlMessage::SetLimiter {
            request_id: self.next_request_id(),
            enabled,
        })
    }

    pub async fn list_parameters(&self) -> Result<Vec<crate::types::VstParameter>, String> {
        let response = self
            .request_with_timeout(
                ControlMessage::ListParameters {
                    request_id: self.next_request_id(),
                },
                COMMAND_TIMEOUT,
            )
            .await?;
        match response {
            ControlMessage::Parameters { parameters, .. } => Ok(parameters),
            ControlMessage::Error { message, .. } => Err(message),
            other => Err(format!("unexpected parameter response: {other:?}")),
        }
    }

    pub async fn panic_all_notes(&self) -> Result<(), String> {
        self.expect_ack(ControlMessage::Panic {
            request_id: self.next_request_id(),
        })
        .await
    }

    pub async fn save_state(&self) -> Result<(), String> {
        self.expect_ack(ControlMessage::SaveState {
            request_id: self.next_request_id(),
        })
        .await
    }

    async fn expect_ack(&self, message: ControlMessage) -> Result<(), String> {
        match self.request_with_timeout(message, COMMAND_TIMEOUT).await? {
            ControlMessage::Ack { .. } => Ok(()),
            ControlMessage::Error { message, .. } => Err(message),
            other => Err(format!("unexpected worker acknowledgement: {other:?}")),
        }
    }

    async fn request_with_timeout(
        &self,
        message: ControlMessage,
        timeout: Duration,
    ) -> Result<ControlMessage, String> {
        let commands = self
            .inner
            .session
            .lock()
            .as_ref()
            .map(|session| session.commands.clone())
            .ok_or_else(|| "VST worker is not connected".to_string())?;
        let (response_tx, response_rx) = oneshot::channel();
        commands
            .send(SessionCommand::Request {
                message,
                response: response_tx,
            })
            .await
            .map_err(|_| "VST worker control channel is closed".to_string())?;
        tokio::time::timeout(timeout, response_rx)
            .await
            .map_err(|_| "VST worker command timed out".to_string())?
            .map_err(|_| "VST worker response channel closed".to_string())?
    }

    fn try_notify(&self, message: ControlMessage) -> bool {
        self.inner.session.lock().as_ref().is_some_and(|session| {
            session
                .commands
                .try_send(SessionCommand::Notify(message))
                .is_ok()
        })
    }

    fn next_request_id(&self) -> u64 {
        self.inner.request_sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn set_state(&self, state: VstWorkerState) {
        self.inner.status.lock().snapshot.state = state;
    }

    fn fault(&self, error: String) {
        let mut guard = self.inner.status.lock();
        guard.snapshot.state = VstWorkerState::Faulted;
        guard.snapshot.last_exit = Some(error);
    }

    fn schedule_restart_after_fault(&self) {
        let now = Instant::now();
        let allowed = {
            let mut history = self.inner.restart_history.lock();
            while history
                .front()
                .is_some_and(|started| now.duration_since(*started) > RESTART_WINDOW)
            {
                history.pop_front();
            }
            if history.len() >= MAX_RESTARTS_PER_WINDOW {
                false
            } else {
                history.push_back(now);
                true
            }
        };
        if !allowed {
            let mut guard = self.inner.status.lock();
            guard.snapshot.state = VstWorkerState::Faulted;
            guard.snapshot.last_exit =
                Some("VST worker restart budget exhausted (3 restarts in 60 seconds)".into());
            return;
        }

        let Some((settings, class_uid)) = self.inner.last_load.lock().clone() else {
            return;
        };
        {
            let mut guard = self.inner.status.lock();
            guard.snapshot.restarts = guard.snapshot.restarts.saturating_add(1);
            background_log(
                "warn",
                format!(
                    "Restarting VST worker (attempt {}/{MAX_RESTARTS_PER_WINDOW})",
                    guard.snapshot.restarts
                ),
            );
        }
        let supervisor = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Err(error) = supervisor.start(settings, class_uid).await {
                supervisor.fault(format!("VST worker restart failed: {error}"));
            }
        });
    }
}

struct SessionCredentials {
    session_id: String,
    token: String,
    control_pipe: String,
    midi_pipe: String,
}

impl SessionCredentials {
    fn new() -> Self {
        let session_id = Uuid::new_v4().simple().to_string();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let pipe_nonce = Uuid::new_v4().simple().to_string();
        Self {
            control_pipe: format!(r"\\.\pipe\oscmidi-{session_id}-{pipe_nonce}-control"),
            midi_pipe: format!(r"\\.\pipe\oscmidi-{session_id}-{pipe_nonce}-midi"),
            session_id,
            token,
        }
    }
}

fn create_pipe_server(name: &str, buffer_bytes: u32) -> Result<NamedPipeServer, String> {
    ServerOptions::new()
        .first_pipe_instance(true)
        .in_buffer_size(buffer_bytes)
        .out_buffer_size(buffer_bytes)
        .create(name)
        .map_err(|error| format!("Failed to create named pipe {name}: {error}"))
}

fn worker_executable_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("OSCMIDI_VST_WORKER_X64_PATH")
        .or_else(|| std::env::var_os("OSCMIDI_VST_WORKER_PATH"))
    {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "OSCMIDI_VST_WORKER_PATH does not point to a file: {}",
            path.display()
        ));
    }
    let current = std::env::current_exe()
        .map_err(|error| format!("Cannot resolve OSCMidi executable: {error}"))?;
    let directory = current
        .parent()
        .ok_or_else(|| "OSCMidi executable has no parent directory".to_string())?;
    for name in ["vst-host-worker-x64.exe", "vst-host-worker.exe"] {
        let path = directory.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }

    let path = directory.join("vst-host-worker-x64.exe");
    Err(format!(
        "VST x64 worker executable not found at {}. Install the MSI or keep both VST workers beside OSCMidi.exe. Developers can rebuild them with `npm run prepare:vst-worker:release`.",
        path.display()
    ))
}

struct WorkerProcess {
    child: Child,
    job: Option<windows::Win32::Foundation::HANDLE>,
}

// Windows job/process handles are kernel object references and may be waited,
// queried, or closed from the supervisor runtime thread.
unsafe impl Send for WorkerProcess {}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            // SAFETY: this process owns the job handle returned by CreateJobObjectW.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
        }
    }
}

fn create_kill_on_close_job(child: &Child) -> Result<windows::Win32::Foundation::HANDLE, String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    // SAFETY: all handles and buffers are valid for the duration of these calls.
    unsafe {
        let job = CreateJobObjectW(None, None).map_err(|error| error.to_string())?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Err(error) = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) {
            let _ = windows::Win32::Foundation::CloseHandle(job);
            return Err(error.to_string());
        }
        let process = HANDLE(child.as_raw_handle());
        if let Err(error) = AssignProcessToJobObject(job, process) {
            let _ = windows::Win32::Foundation::CloseHandle(job);
            return Err(error.to_string());
        }
        Ok(job)
    }
}

fn spawn_worker(credentials: &SessionCredentials) -> Result<WorkerProcess, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let executable = worker_executable_path()?;
    let inherit_stdio = std::env::var_os("OSCMIDI_VST_WORKER_INHERIT_STDIO").is_some();
    let mut command = Command::new(executable);
    command
        .arg("--control-pipe")
        .arg(&credentials.control_pipe)
        .arg("--midi-pipe")
        .arg(&credentials.midi_pipe)
        .arg("--session-id")
        .arg(&credentials.session_id)
        .arg("--token")
        .arg(&credentials.token)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if inherit_stdio {
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let child = command
        .spawn()
        .map_err(|error| format!("Failed to launch VST worker: {error}"))?;
    let job = match create_kill_on_close_job(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            background_log(
                "warn",
                format!("Could not assign VST worker to kill-on-close job: {error}"),
            );
            None
        }
    };
    Ok(WorkerProcess { child, job })
}

async fn authenticate_worker(
    credentials: &SessionCredentials,
    control: &mut NamedPipeServer,
) -> Result<(), String> {
    let hello = read_control_frame(control)
        .await
        .map_err(|error| format!("Failed to read worker hello: {error}"))?;
    match hello {
        ControlMessage::Hello {
            protocol_version,
            host_abi_version,
            session_id,
            token,
            ..
        } if protocol_version == CONTROL_PROTOCOL_VERSION
            && host_abi_version == HOST_ABI_VERSION
            && session_id == credentials.session_id
            && token == credentials.token => Ok(()),
        ControlMessage::Hello {
            protocol_version,
            host_abi_version,
            ..
        } => Err(format!(
            "worker authentication/protocol mismatch (protocol={protocol_version}, ABI={host_abi_version})"
        )),
        _ => Err("worker did not send an authenticated hello".into()),
    }
}

async fn control_reader(
    mut reader: ReadHalf<NamedPipeServer>,
    events: mpsc::Sender<Result<ControlMessage, String>>,
) {
    loop {
        let event = read_control_frame(&mut reader)
            .await
            .map_err(|error| format!("worker control read failed: {error}"));
        let terminal = event.is_err();
        if events.send(event).await.is_err() || terminal {
            break;
        }
    }
}

async fn midi_writer(mut pipe: NamedPipeServer, mut midi: mpsc::Receiver<MidiWireFrame>) {
    while let Some(frame) = midi.recv().await {
        if let Err(error) = write_midi_frame(&mut pipe, frame).await {
            background_log("error", format!("VST worker MIDI pipe failed: {error}"));
            break;
        }
    }
}

fn message_request_id(message: &ControlMessage) -> Option<u64> {
    match message {
        ControlMessage::Load { request_id, .. }
        | ControlMessage::OpenEditor { request_id }
        | ControlMessage::CloseEditor { request_id }
        | ControlMessage::ListParameters { request_id }
        | ControlMessage::SetParameter { request_id, .. }
        | ControlMessage::SetGain { request_id, .. }
        | ControlMessage::SetLimiter { request_id, .. }
        | ControlMessage::SaveState { request_id }
        | ControlMessage::Panic { request_id }
        | ControlMessage::Stop { request_id }
        | ControlMessage::Ready { request_id, .. }
        | ControlMessage::Parameters { request_id, .. }
        | ControlMessage::Ack { request_id }
        | ControlMessage::Stopped { request_id } => Some(*request_id),
        ControlMessage::Error { request_id, .. } => *request_id,
        _ => None,
    }
}

struct SessionRuntime {
    credentials: SessionCredentials,
    control: NamedPipeServer,
    midi_pipe: NamedPipeServer,
    child: WorkerProcess,
    commands: mpsc::Receiver<SessionCommand>,
    midi: mpsc::Receiver<MidiWireFrame>,
    connected: oneshot::Sender<Result<(), String>>,
    inner: Arc<SupervisorInner>,
}

async fn run_session(runtime: SessionRuntime) {
    let SessionRuntime {
        credentials,
        mut control,
        midi_pipe,
        mut child,
        mut commands,
        midi,
        connected,
        inner,
    } = runtime;
    let connection = async {
        tokio::try_join!(control.connect(), midi_pipe.connect())
            .map_err(|error| format!("worker pipe connection failed: {error}"))?;
        authenticate_worker(&credentials, &mut control).await
    };
    match tokio::time::timeout(CONNECT_TIMEOUT, connection).await {
        Ok(Ok(())) => {
            inner.status.lock().last_heartbeat = Some(Instant::now());
            let _ = connected.send(Ok(()));
        }
        Ok(Err(error)) => {
            let _ = connected.send(Err(error.clone()));
            terminate_child(&mut child.child);
            set_session_fault(&inner, error);
            return;
        }
        Err(_) => {
            let error = "worker pipes did not connect within 10 seconds".to_string();
            let _ = connected.send(Err(error.clone()));
            terminate_child(&mut child.child);
            set_session_fault(&inner, error);
            return;
        }
    }

    tauri::async_runtime::spawn(midi_writer(midi_pipe, midi));
    let (reader, mut writer): (ReadHalf<_>, WriteHalf<_>) = tokio::io::split(control);
    let (event_tx, mut event_rx) = mpsc::channel(64);
    tauri::async_runtime::spawn(control_reader(reader, event_tx));

    let mut pending: HashMap<u64, oneshot::Sender<Result<ControlMessage, String>>> = HashMap::new();
    let mut poll = tokio::time::interval(PROCESS_POLL_PERIOD);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut terminal_error = None;

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(SessionCommand::Request { message, response }) => {
                    let Some(request_id) = message_request_id(&message) else {
                        let _ = response.send(Err("request has no request ID".into()));
                        continue;
                    };
                    if let Err(error) = write_control_frame(&mut writer, &message).await {
                        let message = format!("worker control write failed: {error}");
                        let _ = response.send(Err(message.clone()));
                        terminal_error = Some(message);
                        break;
                    }
                    pending.insert(request_id, response);
                }
                Some(SessionCommand::Notify(message)) => {
                    if let Err(error) = write_control_frame(&mut writer, &message).await {
                        terminal_error = Some(format!("worker control write failed: {error}"));
                        break;
                    }
                }
                Some(SessionCommand::Shutdown) | None => break,
            },
            event = event_rx.recv() => match event {
                Some(Ok(ControlMessage::Heartbeat { metrics, .. })) => {
                    let mut guard = inner.status.lock();
                    guard.last_heartbeat = Some(Instant::now());
                    guard.snapshot.metrics = metrics;
                }
                Some(Ok(ControlMessage::EditorHidden)) => {
                    inner.status.lock().snapshot.editor_open = false;
                }
                Some(Ok(message)) => {
                    if let Some(request_id) = message_request_id(&message) {
                        if let Some(response) = pending.remove(&request_id) {
                            let _ = response.send(Ok(message));
                        }
                    }
                }
                Some(Err(error)) => {
                    terminal_error = Some(error);
                    break;
                }
                None => {
                    terminal_error = Some("worker control event channel closed".into());
                    break;
                }
            },
            _ = poll.tick() => {
                match child.child.try_wait() {
                    Ok(Some(status)) => {
                        terminal_error = Some(format!("worker exited with {status}"));
                        break;
                    }
                    Err(error) => {
                        terminal_error = Some(format!("failed to query worker process: {error}"));
                        break;
                    }
                    Ok(None) => {}
                }
                let heartbeat_expired = {
                    let guard = inner.status.lock();
                    guard.snapshot.state == VstWorkerState::Ready
                        && guard
                            .last_heartbeat
                            .is_some_and(|last| last.elapsed() >= HEARTBEAT_TIMEOUT)
                };
                if heartbeat_expired {
                    terminal_error = Some("worker missed three consecutive heartbeats".into());
                    break;
                }
            }
        }
    }

    if let Some(error) = terminal_error {
        for (_, response) in pending.drain() {
            let _ = response.send(Err(error.clone()));
        }
        terminate_child(&mut child.child);
        *inner.session.lock() = None;
        let intentional_stop = matches!(
            inner.status.lock().snapshot.state,
            VstWorkerState::Stopping | VstWorkerState::Disabled
        );
        if !intentional_stop {
            set_session_fault(&inner, error);
            VstWorkerSupervisor {
                inner: inner.clone(),
            }
            .schedule_restart_after_fault();
        }
    }
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn set_session_fault(inner: &Arc<SupervisorInner>, error: String) {
    let mut guard = inner.status.lock();
    guard.snapshot.state = VstWorkerState::Faulted;
    guard.snapshot.last_exit = Some(error.clone());
    background_log("error", format!("VST worker faulted: {error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_names_and_tokens_are_unpredictable_per_session() {
        let first = SessionCredentials::new();
        let second = SessionCredentials::new();
        assert_ne!(first.session_id, second.session_id);
        assert_ne!(first.token, second.token);
        assert_ne!(first.control_pipe, second.control_pipe);
        assert!(first.control_pipe.starts_with(r"\\.\pipe\oscmidi-"));
        assert!(first.token.len() >= 64);
    }

    #[test]
    fn response_ids_cover_every_request_response_pair() {
        assert_eq!(
            message_request_id(&ControlMessage::Ack { request_id: 7 }),
            Some(7)
        );
        assert_eq!(
            message_request_id(&ControlMessage::Error {
                request_id: Some(9),
                code: "test".into(),
                message: "test".into(),
            }),
            Some(9)
        );
        assert_eq!(
            message_request_id(&ControlMessage::Heartbeat {
                sequence: 1,
                monotonic_qpc: 2,
                metrics: WorkerMetrics::default(),
            }),
            None
        );
    }
}
