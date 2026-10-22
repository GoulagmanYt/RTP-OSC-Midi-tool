use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use arc_swap::ArcSwapOption;
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
const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const HEARTBEAT_WARNING: Duration = Duration::from_millis(1_500);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
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
    pub heartbeat_delayed: bool,
    pub midi_transport_failed: bool,
    pub callback_healthy: bool,
}

#[derive(Default)]
struct SupervisorStatus {
    snapshot: SupervisorSnapshot,
    last_heartbeat: Option<Instant>,
    last_callback: u64,
    callback_progress: Option<Instant>,
    maintenance_started: Option<Instant>,
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
    generation: u64,
    task: tauri::async_runtime::JoinHandle<()>,
    commands: mpsc::Sender<SessionCommand>,
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        // Also covers cancellation while stop_session owns the session locally.
        self.task.abort();
    }
}

struct SupervisorInner {
    lifecycle: tokio::sync::Mutex<()>,
    lifecycle_generation: AtomicU64,
    status: Mutex<SupervisorStatus>,
    session: Mutex<Option<ActiveSession>>,
    midi_route: ArcSwapOption<crate::midi_queue::AsyncSender<MidiWireFrame>>,
    midi_drops: AtomicU32,
    request_sequence: AtomicU64,
    midi_sequence: AtomicU64,
    state_generation: AtomicU64,
    emergency_reset_pending: AtomicBool,
    midi_generation: AtomicU64,
    midi_transport_failed: AtomicBool,
    emergency_reset_notify: tokio::sync::Notify,
    last_load: Mutex<Option<(AudioSettings, Option<String>)>>,
    restart_history: Mutex<VecDeque<Instant>>,
}

#[derive(Clone)]
pub struct VstWorkerSupervisor {
    inner: Arc<SupervisorInner>,
    // Background contexts do not retain the lifetime of public owners.
    _lifetime: Option<Arc<SupervisorLifetime>>,
}

struct SupervisorLifetime {
    inner: Arc<SupervisorInner>,
}
impl Drop for SupervisorLifetime {
    fn drop(&mut self) {
        self.inner
            .lifecycle_generation
            .fetch_add(1, Ordering::AcqRel);
        self.inner.midi_route.store(None);
        if let Some(session) = self.inner.session.lock().take() {
            session.task.abort();
        }
    }
}

struct PendingSessionTask(Option<tauri::async_runtime::JoinHandle<()>>);
impl Drop for PendingSessionTask {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

impl Default for VstWorkerSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl VstWorkerSupervisor {
    pub fn new() -> Self {
        let inner = Arc::new(SupervisorInner {
            lifecycle: tokio::sync::Mutex::new(()),
            lifecycle_generation: AtomicU64::new(0),
            status: Mutex::new(SupervisorStatus::default()),
            session: Mutex::new(None),
            midi_route: ArcSwapOption::empty(),
            midi_drops: AtomicU32::new(0),
            request_sequence: AtomicU64::new(1),
            midi_sequence: AtomicU64::new(1),
            state_generation: AtomicU64::new(0),
            emergency_reset_pending: AtomicBool::new(false),
            midi_generation: AtomicU64::new(0),
            midi_transport_failed: AtomicBool::new(false),
            emergency_reset_notify: tokio::sync::Notify::new(),
            last_load: Mutex::new(None),
            restart_history: Mutex::new(VecDeque::new()),
        });
        Self {
            inner: inner.clone(),
            _lifetime: Some(Arc::new(SupervisorLifetime { inner })),
        }
    }

    fn task_context(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _lifetime: None,
        }
    }

    pub fn snapshot(&self) -> SupervisorSnapshot {
        let status = self.inner.status.lock();
        let mut snapshot = status.snapshot.clone();
        snapshot.callback_healthy = status.maintenance_started.is_some()
            || (snapshot.metrics.callback_count > 0
                && status
                    .callback_progress
                    .is_some_and(|last| last.elapsed() < HEARTBEAT_WARNING));
        snapshot.midi_drops = self.inner.midi_drops.load(Ordering::Relaxed);
        snapshot.midi_transport_failed = self.inner.midi_transport_failed.load(Ordering::Acquire);
        snapshot
    }

    pub fn is_ready(&self) -> bool {
        self.snapshot().state == VstWorkerState::Ready
    }

    pub async fn start(
        &self,
        settings: AudioSettings,
        class_uid: Option<String>,
    ) -> Result<WorkerAudioStatus, String> {
        let generation = self
            .inner
            .lifecycle_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.start_generation(settings, class_uid, generation).await
    }

    async fn start_generation(
        &self,
        settings: AudioSettings,
        class_uid: Option<String>,
        generation: u64,
    ) -> Result<WorkerAudioStatus, String> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if generation != self.inner.lifecycle_generation.load(Ordering::Acquire) {
            return Err("Worker start superseded by another lifecycle request".into());
        }
        self.stop_session().await.ok();
        // Allow virtual ASIO drivers (e.g. Voicemeeter AUX Virtual ASIO) time to release
        // their single-client IPC handle and buffer locks after the previous worker exits.
        tokio::time::sleep(Duration::from_millis(150)).await;
        // Old-session controls cannot overwrite the new recovery snapshot.
        *self.inner.last_load.lock() = Some((settings.clone(), class_uid.clone()));
        self.inner
            .midi_transport_failed
            .store(false, Ordering::Release);
        self.set_state(VstWorkerState::Starting);

        let credentials = SessionCredentials::new();
        let control_server =
            create_pipe_server(&credentials.control_pipe, CONTROL_PIPE_BUFFER_BYTES)?;
        let midi_runtime = crate::realtime::MidiRuntime::new("vst-midi-write")?;
        let midi_name = credentials.midi_pipe.clone();
        let midi_server = midi_runtime
            .spawn(async move { create_pipe_server(&midi_name, MIDI_PIPE_BUFFER_BYTES) })
            .await
            .map_err(|error| error.to_string())??;
        let child = match spawn_worker(&credentials) {
            Ok(child) => child,
            Err(error) => {
                self.fault(error.clone());
                return Err(error);
            }
        };

        let (command_tx, command_rx) = mpsc::channel(64);
        let (midi_tx, midi_rx) = crate::midi_queue::async_channel(MIDI_PIPE_CAPACITY);
        let (connected_tx, connected_rx) = oneshot::channel();
        let inner = self.inner.clone();
        let mut pending_task = PendingSessionTask(Some(tauri::async_runtime::spawn(async move {
            run_session(SessionRuntime {
                generation,
                credentials,
                control: control_server,
                midi_pipe: midi_server,
                midi_runtime,
                child,
                commands: command_rx,
                midi: midi_rx,
                connected: connected_tx,
                inner,
            })
            .await;
        })));

        let connected = match tokio::time::timeout(CONNECT_TIMEOUT, connected_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("worker connection task exited before authentication".to_string()),
            Err(_) => Err("timed out authenticating VST worker".to_string()),
        };
        if let Err(error) = connected {
            let task = pending_task.0.take().expect("pending session task");
            task.abort();
            let _ = task.await;
            self.fault(error.clone());
            return Err(error);
        }

        *self.inner.session.lock() = Some(ActiveSession {
            generation,
            task: pending_task.0.take().expect("connected session task"),
            commands: command_tx,
        });
        self.inner.midi_route.store(Some(Arc::new(midi_tx)));
        self.set_state(VstWorkerState::Loading);

        let game_mode = settings.game_mode;
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
                self.stop_session().await.ok();
                self.fault(error.clone());
                return Err(error);
            }
        };
        match response {
            ControlMessage::Ready { status, .. } => {
                let mut guard = self.inner.status.lock();
                guard.snapshot.status = Some(status.clone());
                guard.snapshot.state = VstWorkerState::Ready;
                guard.snapshot.metrics.game_mode = game_mode;
                guard.callback_progress = Some(Instant::now());
                guard.last_callback = 0;
                guard.maintenance_started = None;
                Ok(status)
            }
            ControlMessage::Error { message, .. } => {
                self.stop_session().await.ok();
                self.fault(message.clone());
                Err(message)
            }
            other => {
                let error = format!("unexpected worker load response: {other:?}");
                self.stop_session().await.ok();
                self.fault(error.clone());
                Err(error)
            }
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        self.inner
            .lifecycle_generation
            .fetch_add(1, Ordering::AcqRel);
        self.inner.midi_route.store(None);
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.stop_session().await
    }

    async fn stop_session(&self) -> Result<(), String> {
        self.inner.midi_route.store(None);
        self.set_state(VstWorkerState::Stopping);
        let session = self.inner.session.lock().take();
        let mut result = Ok(());
        if let Some(mut session) = session {
            let request_id = self.next_request_id();
            let (response_tx, response_rx) = oneshot::channel();
            result = match tokio::time::timeout(COMMAND_TIMEOUT, async {
                session
                    .commands
                    .send(SessionCommand::Request {
                        message: ControlMessage::Stop { request_id },
                        response: response_tx,
                    })
                    .await
                    .map_err(|_| "worker control channel is closed".to_string())?;
                match response_rx.await {
                    Ok(Ok(ControlMessage::Stopped { .. })) => Ok(()),
                    Ok(Ok(other)) => Err(format!("Unexpected worker stop response: {other:?}")),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err("worker stop response channel closed".into()),
                }
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err("worker did not stop within 10 seconds".into()),
            };
            // Never leave the session alive when Stop failed or its reply was lost.
            if result.is_err() || session.commands.try_send(SessionCommand::Shutdown).is_err() {
                session.task.abort();
            }
            match tokio::time::timeout(COMMAND_TIMEOUT, &mut session.task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if result.is_ok() => {
                    result = Err(format!("worker session join failed: {error}"))
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    session.task.abort();
                    let _ = (&mut session.task).await;
                    result = Err("worker session did not terminate within 10 seconds".into());
                }
            }
        }
        self.set_state(if result.is_ok() {
            VstWorkerState::Disabled
        } else {
            VstWorkerState::Faulted
        });
        result
    }

    pub fn try_send_midi(&self, bytes: &[u8]) -> bool {
        self.try_send_midi_at(bytes, monotonic_qpc())
    }

    pub(crate) fn queue_ping(&self) -> Result<(), String> {
        let lifecycle = self.inner.lifecycle_generation.load(Ordering::Acquire);
        let sender = self
            .inner
            .midi_route
            .load_full()
            .ok_or("Worker MIDI route unavailable")?;
        let generation = self.inner.midi_generation.load(Ordering::Acquire);
        if self.inner.lifecycle_generation.load(Ordering::Acquire) != lifecycle {
            return Err("Audio test superseded by a lifecycle change".into());
        }
        let mut on = MidiWireFrame::new(
            self.inner.midi_sequence.fetch_add(1, Ordering::Relaxed),
            monotonic_qpc(),
            &[0x90, 60, 100],
        )
        .unwrap();
        on.generation = generation;
        sender
            .try_send(on)
            .map_err(|_| "Audio test note could not be queued")?;
        let supervisor = self.task_context();
        let release_supervisor = supervisor.task_context();
        let fallback_sender = sender.clone();
        std::thread::Builder::new()
            .name("audio-test-note-off".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(220));
                if release_supervisor
                    .inner
                    .midi_generation
                    .load(Ordering::Acquire)
                    != generation
                    || release_supervisor
                        .inner
                        .lifecycle_generation
                        .load(Ordering::Acquire)
                        != lifecycle
                {
                    return;
                }
                let mut off = MidiWireFrame::new(
                    release_supervisor
                        .inner
                        .midi_sequence
                        .fetch_add(1, Ordering::Relaxed),
                    monotonic_qpc(),
                    &[0x80, 60, 0],
                )
                .unwrap();
                off.generation = generation;
                // Captured route and epoch prevent a delayed release from reaching
                // a replacement worker or cutting a note admitted after Panic.
                if sender.try_send(off).is_err()
                    && release_supervisor
                        .inner
                        .midi_generation
                        .load(Ordering::Acquire)
                        == generation
                {
                    if let Ok(_gate) = release_supervisor.inner.lifecycle.try_lock() {
                        if release_supervisor
                            .inner
                            .lifecycle_generation
                            .load(Ordering::Acquire)
                            == lifecycle
                            && release_supervisor
                                .inner
                                .midi_generation
                                .load(Ordering::Acquire)
                                == generation
                            && release_supervisor
                                .inner
                                .midi_route
                                .load()
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &sender))
                        {
                            release_supervisor.request_emergency_reset();
                        }
                    }
                }
            })
            .map_err(|error| {
                if let Ok(_gate) = supervisor.inner.lifecycle.try_lock() {
                    if supervisor
                        .inner
                        .lifecycle_generation
                        .load(Ordering::Acquire)
                        == lifecycle
                        && supervisor
                            .inner
                            .midi_route
                            .load()
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &fallback_sender))
                    {
                        supervisor.request_emergency_reset();
                    }
                }
                format!("Failed to schedule audio test release: {error}")
            })?;
        Ok(())
    }

    pub fn try_send_midi_at(&self, bytes: &[u8], ingress_qpc: u64) -> bool {
        let frame = MidiWireFrame::new(
            self.inner.midi_sequence.fetch_add(1, Ordering::Relaxed),
            ingress_qpc,
            bytes,
        );
        let Some(mut frame) = frame else {
            return false;
        };
        frame.generation = self.inner.midi_generation.load(Ordering::Acquire);
        let sent = self
            .inner
            .midi_route
            .load()
            .as_ref()
            .is_some_and(|sender| sender.try_send(frame).is_ok());
        if !sent {
            let _ =
                self.inner
                    .midi_drops
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        Some(value.saturating_add(1))
                    });
        }
        sent
    }

    pub fn request_emergency_reset(&self) {
        self.inner.midi_generation.fetch_add(1, Ordering::AcqRel);
        self.inner
            .emergency_reset_pending
            .store(true, Ordering::Release);
        self.inner.emergency_reset_notify.notify_one();
    }

    pub async fn set_game_mode(&self, enabled: bool) -> Result<(), String> {
        let _lifecycle = self.control_lifecycle().await?;
        self.expect_ack(ControlMessage::SetGameMode {
            request_id: self.next_request_id(),
            enabled,
        })
        .await?;
        let mut status = self.inner.status.lock();
        status.snapshot.metrics.game_mode = enabled;
        if let Some((settings, _)) = self.inner.last_load.lock().as_mut() {
            settings.game_mode = enabled;
        }
        if enabled {
            status.snapshot.editor_open = false;
        }
        Ok(())
    }

    pub async fn open_editor(&self) -> Result<(), String> {
        let _lifecycle = self.control_lifecycle().await?;
        self.expect_ack(ControlMessage::OpenEditor {
            request_id: self.next_request_id(),
        })
        .await?;
        let mut status = self.inner.status.lock();
        status.snapshot.editor_open = !status.snapshot.metrics.game_mode;
        Ok(())
    }

    pub async fn close_editor(&self) -> Result<(), String> {
        let _lifecycle = self.control_lifecycle().await?;
        self.expect_ack(ControlMessage::CloseEditor {
            request_id: self.next_request_id(),
        })
        .await?;
        self.inner.status.lock().snapshot.editor_open = false;
        self.save_state().await?;
        Ok(())
    }

    pub async fn set_parameter(&self, index: usize, value: f32) -> Result<(), String> {
        let _lifecycle = self.control_lifecycle().await?;
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
        let supervisor = self.task_context();
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
        let mut load = self.inner.last_load.lock();
        let admitted = self.try_notify(ControlMessage::SetGain {
            request_id: self.next_request_id(),
            gain_db,
        });
        if admitted {
            if let Some((settings, _)) = load.as_mut() {
                settings.gain_db = gain_db;
            }
        }
        admitted
    }

    pub fn try_set_limiter_enabled(&self, enabled: bool) -> bool {
        let mut load = self.inner.last_load.lock();
        let admitted = self.try_notify(ControlMessage::SetLimiter {
            request_id: self.next_request_id(),
            enabled,
        });
        if admitted {
            if let Some((settings, _)) = load.as_mut() {
                settings.limiter_enabled = enabled;
            }
        }
        admitted
    }

    pub fn try_set_eq(&self, settings: crate::config::VstEqSettings) -> bool {
        let mut load = self.inner.last_load.lock();
        let admitted = self.try_notify(ControlMessage::SetEq {
            request_id: self.next_request_id(),
            settings: settings.clone(),
        });
        if admitted {
            if let Some((audio, _)) = load.as_mut() {
                audio.vst_eq = settings;
            }
        }
        admitted
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

    // Keep response publication in the same lifecycle as admission. Load already
    // owns this gate, so request_with_timeout itself must not acquire it.
    async fn control_lifecycle(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        let generation = self.inner.lifecycle_generation.load(Ordering::Acquire);
        let guard = tokio::time::timeout(COMMAND_TIMEOUT, self.inner.lifecycle.lock())
            .await
            .map_err(|_| "VST worker lifecycle operation timed out".to_string())?;
        if generation != self.inner.lifecycle_generation.load(Ordering::Acquire) {
            return Err("VST worker command superseded by a lifecycle change".into());
        }
        Ok(guard)
    }

    async fn request_with_timeout(
        &self,
        message: ControlMessage,
        timeout: Duration,
    ) -> Result<ControlMessage, String> {
        let (commands, generation) = self
            .inner
            .session
            .lock()
            .as_ref()
            .map(|session| (session.commands.clone(), session.generation))
            .ok_or_else(|| "VST worker is not connected".to_string())?;
        if generation != self.inner.lifecycle_generation.load(Ordering::Acquire) {
            return Err("VST worker command superseded by a lifecycle change".into());
        }
        let (response_tx, response_rx) = oneshot::channel();
        let response = tokio::time::timeout(timeout, async {
            commands
                .send(SessionCommand::Request {
                    message,
                    response: response_tx,
                })
                .await
                .map_err(|_| "VST worker control channel is closed".to_string())?;
            response_rx
                .await
                .map_err(|_| "VST worker response channel closed".to_string())?
        })
        .await
        .map_err(|_| "VST worker command timed out".to_string())?;
        if generation != self.inner.lifecycle_generation.load(Ordering::Acquire) {
            return Err("VST worker command superseded by a lifecycle change".into());
        }
        response
    }

    fn try_notify(&self, message: ControlMessage) -> bool {
        self.inner.session.lock().as_ref().is_some_and(|session| {
            session.generation == self.inner.lifecycle_generation.load(Ordering::Acquire)
                && session
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

    fn schedule_restart_after_fault(&self, generation: u64) {
        if generation != self.inner.lifecycle_generation.load(Ordering::Acquire) {
            return;
        }
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
        let supervisor = self.task_context();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Err(error) = supervisor
                .start_generation(settings, class_uid, generation)
                .await
            {
                if generation
                    == supervisor
                        .inner
                        .lifecycle_generation
                        .load(Ordering::Acquire)
                {
                    supervisor.fault(format!("VST worker restart failed: {error}"));
                }
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
    use windows::{
        core::w,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
            },
        },
    };

    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // Protected DACL: only LocalSystem and the object owner (the interactive
    // user that launched OSCMidi) may open a worker pipe.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w!("D:P(A;;GA;;;SY)(A;;GA;;;OW)"),
            1,
            &mut descriptor,
            None,
        )
        .map_err(|error| format!("Failed to create worker pipe security descriptor: {error}"))?;
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .in_buffer_size(buffer_bytes)
        .out_buffer_size(buffer_bytes);
    let result = unsafe {
        options
            .create_with_security_attributes_raw(name, std::ptr::from_mut(&mut attributes).cast())
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    result.map_err(|error| format!("Failed to create named pipe {name}: {error}"))
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
        // Cancellation must also reap the child if Windows job assignment failed.
        terminate_child(&mut self.child);
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
    use windows::Win32::System::Threading::HIGH_PRIORITY_CLASS;
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
        .creation_flags(CREATE_NO_WINDOW | HIGH_PRIORITY_CLASS.0);
    if inherit_stdio {
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    #[cfg(feature = "latency-diagnostics")]
    command.env("OSCMIDI_LATENCY_TRACE", "1");
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

async fn control_writer<W: tokio::io::AsyncWrite + Unpin>(
    mut writer: W,
    mut commands: mpsc::Receiver<ControlMessage>,
    timeout: Duration,
) -> Result<(), String> {
    while let Some(message) = commands.recv().await {
        // A timed-out write may have emitted part of a frame. Terminate the
        // session instead of reusing a stream whose framing is now uncertain.
        tokio::time::timeout(timeout, write_control_frame(&mut writer, &message))
            .await
            .map_err(|_| "worker control write timed out".to_string())?
            .map_err(|error| format!("worker control write failed: {error}"))?;
    }
    Ok(())
}

async fn midi_writer<W: tokio::io::AsyncWrite + Unpin>(
    mut pipe: W,
    mut midi: crate::midi_queue::AsyncReceiver<MidiWireFrame>,
    inner: Arc<SupervisorInner>,
    lifecycle_generation: u64,
) {
    let mut generation = 0;
    let mut saved = None;
    let mut batch = [0u8; crate::vst_worker::protocol::MIDI_FRAME_BYTES * 256];
    loop {
        if inner.emergency_reset_pending.swap(false, Ordering::AcqRel)
            || generation != inner.midi_generation.load(Ordering::Acquire)
        {
            generation = inner.midi_generation.load(Ordering::Acquire);
            // An in-band reset cannot overtake older bytes already in the MIDI pipe.
            let reset = MidiWireFrame::new(
                inner.midi_sequence.fetch_add(1, Ordering::Relaxed),
                monotonic_qpc(),
                &[0xFF],
            )
            .expect("one-byte MIDI reset");
            if !matches!(
                tokio::time::timeout(
                    Duration::from_millis(500),
                    write_midi_frame(&mut pipe, reset),
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
        }
        tokio::select! {
            biased;
            _ = inner.emergency_reset_notify.notified() => continue,
            frame = async { if let Some(frame) = saved.take() { Some(frame) } else { midi.recv().await } } => {
                let Some(mut frame) = frame else { break; };
                frame.pipe_qpc = monotonic_qpc();
                #[cfg(feature = "latency-diagnostics")]
                crate::latency_trace::record(
                    crate::latency_trace::Stage::PipeWriteStarted,
                    frame.bytes(), frame.monotonic_qpc, frame.pipe_qpc,
                );
                if frame.generation < generation { tokio::task::yield_now().await; continue; }
                if frame.generation > generation || inner.midi_generation.load(Ordering::Acquire) != generation {
                    saved = Some(frame); continue;
                }
                let width = crate::vst_worker::protocol::MIDI_FRAME_BYTES;
                batch[..width].copy_from_slice(&frame.encode());
                let mut count = 1;
                let mut consumed = 1;
                while consumed < 256 && !inner.emergency_reset_pending.load(Ordering::Acquire) {
                    let Ok(mut frame) = midi.try_recv() else { break; };
                    consumed += 1;
                    if frame.generation < generation { continue; }
                    if frame.generation > generation { saved = Some(frame); break; }
                    frame.pipe_qpc = monotonic_qpc();
                    #[cfg(feature = "latency-diagnostics")]
                    crate::latency_trace::record(
                        crate::latency_trace::Stage::PipeWriteStarted,
                        frame.bytes(), frame.monotonic_qpc, frame.pipe_qpc,
                    );
                    batch[count * width..(count + 1) * width].copy_from_slice(&frame.encode());
                    count += 1;
                }
                if inner.emergency_reset_pending.load(Ordering::Acquire) || generation != inner.midi_generation.load(Ordering::Acquire) { continue; }
                let write_fut = tokio::io::AsyncWriteExt::write_all(&mut pipe, &batch[..count * width]);
                match tokio::time::timeout(Duration::from_millis(500), write_fut).await {
                    Ok(Ok(())) => {},
                    Ok(Err(error)) => {
                        background_log("error", format!("VST worker MIDI pipe failed: {error}"));
                        break;
                    }
                    Err(_) => {
                        background_log("error", "VST worker MIDI pipe write timed out (worker blocked)");
                        break;
                    }
                }
                if consumed >= 256 {
                    tokio::task::yield_now().await;
                }
            }
        }
    }
    if lifecycle_generation == inner.lifecycle_generation.load(Ordering::Acquire) {
        inner.midi_transport_failed.store(true, Ordering::Release);
    }
}

fn message_request_id(message: &ControlMessage) -> Option<u64> {
    match message {
        ControlMessage::Load { request_id, .. }
        | ControlMessage::SetGameMode { request_id, .. }
        | ControlMessage::OpenEditor { request_id }
        | ControlMessage::CloseEditor { request_id }
        | ControlMessage::ListParameters { request_id }
        | ControlMessage::SetParameter { request_id, .. }
        | ControlMessage::SetGain { request_id, .. }
        | ControlMessage::SetLimiter { request_id, .. }
        | ControlMessage::SetEq { request_id, .. }
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

struct SessionIoTasks {
    midi: tokio::task::JoinHandle<()>,
    _runtime: crate::realtime::MidiRuntime,
    control: tauri::async_runtime::JoinHandle<()>,
    writer: tauri::async_runtime::JoinHandle<Result<(), String>>,
    writer_finished: bool,
}
impl SessionIoTasks {
    async fn stop(&mut self) {
        self.midi.abort();
        self.control.abort();
        self.writer.abort();
        let _ = (&mut self.midi).await;
        let _ = (&mut self.control).await;
        if !self.writer_finished {
            let _ = (&mut self.writer).await;
        }
    }
}
impl Drop for SessionIoTasks {
    fn drop(&mut self) {
        self.midi.abort();
        self.control.abort();
        self.writer.abort();
    }
}

struct SessionRuntime {
    generation: u64,
    credentials: SessionCredentials,
    control: NamedPipeServer,
    midi_pipe: NamedPipeServer,
    midi_runtime: crate::realtime::MidiRuntime,
    child: WorkerProcess,
    commands: mpsc::Receiver<SessionCommand>,
    midi: crate::midi_queue::AsyncReceiver<MidiWireFrame>,
    connected: oneshot::Sender<Result<(), String>>,
    inner: Arc<SupervisorInner>,
}

async fn run_session(runtime: SessionRuntime) {
    let SessionRuntime {
        generation,
        credentials,
        mut control,
        midi_pipe,
        midi_runtime,
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

    let midi_task = midi_runtime.spawn(midi_writer(midi_pipe, midi, inner.clone(), generation));
    let (reader, writer): (ReadHalf<_>, WriteHalf<_>) = tokio::io::split(control);
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let control_task = tauri::async_runtime::spawn(control_reader(reader, event_tx));
    let (write_tx, write_rx) = mpsc::channel(64);
    let writer_task =
        tauri::async_runtime::spawn(control_writer(writer, write_rx, CONTROL_WRITE_TIMEOUT));
    let mut io_tasks = SessionIoTasks {
        midi: midi_task,
        _runtime: midi_runtime,
        control: control_task,
        writer: writer_task,
        writer_finished: false,
    };

    let mut pending: HashMap<u64, oneshot::Sender<Result<ControlMessage, String>>> = HashMap::new();
    let mut poll = tokio::time::interval(PROCESS_POLL_PERIOD);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut terminal_error = None;

    loop {
        tokio::select! {
            command = async {
                let permit = write_tx.reserve().await.map_err(|error| error.to_string())?;
                Ok::<_, String>((commands.recv().await, permit))
            } => match command {
                Ok((Some(SessionCommand::Request { message, response }), permit)) => {
                    if response.is_closed() { continue; }
                    let Some(request_id) = message_request_id(&message) else {
                        let _ = response.send(Err("request has no request ID".into()));
                        continue;
                    };
                    pending.insert(request_id, response);
                    permit.send(message);
                }
                Ok((Some(SessionCommand::Notify(message)), permit)) => {
                    permit.send(message);
                }
                Ok((Some(SessionCommand::Shutdown) | None, _)) => break,
                Err(error) => {
                    terminal_error = Some(format!("worker control writer unavailable: {error}"));
                    break;
                }
            },
            result = &mut io_tasks.writer => {
                io_tasks.writer_finished = true;
                terminal_error = Some(match result {
                    Ok(Err(error)) => error,
                    Ok(Ok(())) => "worker control writer exited".into(),
                    Err(error) => format!("worker control writer failed: {error}"),
                });
                break;
            },
            event = event_rx.recv() => match event {
                Some(Ok(ControlMessage::Heartbeat { metrics, .. })) => {
                    #[cfg(feature = "latency-diagnostics")]
                    crate::latency_trace::import(&metrics.diagnostic_trace);
                    let mut guard = inner.status.lock();
                    let now = Instant::now();
                    guard.last_heartbeat = Some(now);
                    guard.snapshot.heartbeat_delayed = false;
                    if metrics.callback_count != guard.last_callback {
                        guard.last_callback = metrics.callback_count;
                        guard.callback_progress = Some(now);
                    }
                    if metrics.maintenance {
                        guard.maintenance_started.get_or_insert(now);
                    } else if guard.maintenance_started.take().is_some() { guard.callback_progress = Some(now); }
                    guard.snapshot.editor_open = metrics.editor_open;
                    if let (Some(status), Some(enabled)) =
                        (guard.snapshot.status.as_mut(), metrics.limiter_enabled)
                    {
                        status.limiter_enabled = enabled;
                    }
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
                pending.retain(|_, response| !response.is_closed());
                let transport_failed = { let status = inner.status.lock(); status.snapshot.state == VstWorkerState::Ready && (inner.midi_transport_failed.load(Ordering::Acquire) || status.snapshot.metrics.midi_transport_failed) };
                if transport_failed {
                    terminal_error = Some("worker MIDI transport failed".into()); break;
                }
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
                    let mut guard = inner.status.lock();
                    guard.snapshot.heartbeat_delayed = guard.last_heartbeat.is_some_and(|last| last.elapsed() >= HEARTBEAT_WARNING);
                    guard.snapshot.state == VstWorkerState::Ready
                        && guard
                            .last_heartbeat
                            .is_some_and(|last| last.elapsed() >= HEARTBEAT_TIMEOUT)
                };
                let callback_stalled = {
                    let guard = inner.status.lock();
                    guard.snapshot.state == VstWorkerState::Ready
                        && !guard.snapshot.metrics.maintenance
                        && guard.callback_progress.is_some_and(|last| last.elapsed() >= HEARTBEAT_TIMEOUT)
                };
                let maintenance_expired = inner.status.lock().maintenance_started
                    .is_some_and(|start| start.elapsed() >= COMMAND_TIMEOUT);
                if heartbeat_expired || callback_stalled || maintenance_expired {
                    terminal_error = Some(if heartbeat_expired { "worker heartbeat timed out after 5 seconds" }
                        else if maintenance_expired { "worker maintenance exceeded 10 seconds" }
                        else { "worker audio callback stalled for 5 seconds" }.into());
                    break;
                }
            }
        }
    }

    io_tasks.stop().await;
    terminate_child(&mut child.child);
    if let Some(error) = terminal_error {
        // Finish state cleanup before waking failed requests, whose callers may
        // immediately stop/restart the supervisor.
        if generation == inner.lifecycle_generation.load(Ordering::Acquire) {
            inner.midi_route.store(None);
            *inner.session.lock() = None;
            let intentional_stop = matches!(
                inner.status.lock().snapshot.state,
                VstWorkerState::Stopping | VstWorkerState::Disabled
            );
            if !intentional_stop {
                set_session_fault(&inner, error.clone());
                VstWorkerSupervisor {
                    inner: inner.clone(),
                    _lifetime: None,
                }
                .schedule_restart_after_fault(generation);
            }
        }
        for (_, response) in pending.drain() {
            let _ = response.send(Err(error.clone()));
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

    #[tokio::test]
    async fn command_started_during_stop_cannot_adopt_the_old_session() {
        use std::{future::Future, task::Poll};
        let supervisor = VstWorkerSupervisor::new();
        *supervisor.inner.last_load.lock() = Some((
            AudioSettings {
                game_mode: false,
                ..Default::default()
            },
            None,
        ));
        supervisor.set_state(VstWorkerState::Ready);
        let (commands, mut rx) = mpsc::channel(8);
        let (delayed_tx, mut delayed_rx) = oneshot::channel();
        let task = tauri::async_runtime::spawn(async move {
            let mut delayed_tx = Some(delayed_tx);
            while let Some(command) = rx.recv().await {
                match command {
                    SessionCommand::Request {
                        message: ControlMessage::SetGameMode { request_id, .. },
                        response,
                    } => {
                        let _ = delayed_tx.take().unwrap().send((request_id, response));
                    }
                    SessionCommand::Request {
                        message: ControlMessage::Stop { request_id },
                        response,
                    } => {
                        let _ = response.send(Ok(ControlMessage::Stopped { request_id }));
                    }
                    SessionCommand::Shutdown => break,
                    _ => panic!("unexpected fixture command"),
                }
            }
        });
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands,
            task,
        });
        let gate = supervisor.inner.lifecycle.lock().await;
        let stopping_supervisor = supervisor.clone();
        let stopping = tokio::spawn(async move { stopping_supervisor.stop().await });
        while supervisor
            .inner
            .lifecycle_generation
            .load(Ordering::Acquire)
            == 0
        {
            tokio::task::yield_now().await;
        }
        let mut pending = Box::pin(supervisor.set_game_mode(true));
        // Poll admission once before letting Stop acquire the lifecycle gate.
        let immediate = std::future::poll_fn(|cx| {
            Poll::Ready(match pending.as_mut().poll(cx) {
                Poll::Ready(result) => Some(result),
                Poll::Pending => None,
            })
        })
        .await;
        drop(gate);
        stopping.await.unwrap().unwrap();
        if let Ok((request_id, response)) = delayed_rx.try_recv() {
            response
                .send(Ok(ControlMessage::Ack { request_id }))
                .unwrap();
        }
        let result = match immediate {
            Some(result) => result,
            None => pending.await,
        };
        assert!(
            result.is_err(),
            "the stopped worker must not accept a command from the next lifecycle"
        );
        assert!(!supervisor.snapshot().metrics.game_mode);
        assert!(
            !supervisor
                .inner
                .last_load
                .lock()
                .as_ref()
                .unwrap()
                .0
                .game_mode
        );
    }

    #[tokio::test]
    async fn a_late_command_ack_cannot_publish_state_after_stop_is_requested() {
        let supervisor = VstWorkerSupervisor::new();
        let settings = AudioSettings {
            game_mode: false,
            ..Default::default()
        };
        *supervisor.inner.last_load.lock() = Some((settings, None));
        supervisor.set_state(VstWorkerState::Ready);
        let (commands, mut rx) = mpsc::channel(8);
        let (delayed_tx, delayed_rx) = oneshot::channel();
        let task = tauri::async_runtime::spawn(async move {
            let mut delayed_tx = Some(delayed_tx);
            while let Some(command) = rx.recv().await {
                match command {
                    SessionCommand::Request {
                        message: ControlMessage::SetGameMode { request_id, .. },
                        response,
                    } => {
                        let _ = delayed_tx.take().unwrap().send((request_id, response));
                    }
                    SessionCommand::Request {
                        message: ControlMessage::Stop { request_id },
                        response,
                    } => {
                        let _ = response.send(Ok(ControlMessage::Stopped { request_id }));
                    }
                    SessionCommand::Shutdown => break,
                    _ => panic!("unexpected fixture command"),
                }
            }
        });
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands,
            task,
        });
        let pending_supervisor = supervisor.clone();
        let pending = tokio::spawn(async move { pending_supervisor.set_game_mode(true).await });
        let (request_id, response) = delayed_rx.await.unwrap();
        let stopping_supervisor = supervisor.clone();
        let stopping = tokio::spawn(async move { stopping_supervisor.stop().await });
        while supervisor
            .inner
            .lifecycle_generation
            .load(Ordering::Acquire)
            == 0
        {
            tokio::task::yield_now().await;
        }
        response
            .send(Ok(ControlMessage::Ack { request_id }))
            .unwrap();
        let result = pending.await.unwrap();
        stopping.await.unwrap().unwrap();
        assert!(
            result.is_err(),
            "an acknowledgement from the stopped session must be obsolete"
        );
        assert!(!supervisor.snapshot().metrics.game_mode);
        assert!(
            !supervisor
                .inner
                .last_load
                .lock()
                .as_ref()
                .unwrap()
                .0
                .game_mode
        );
    }

    #[test]
    fn ping_release_stays_on_its_original_worker_route_and_epoch() {
        let supervisor = VstWorkerSupervisor::new();
        let (tx, mut rx) = crate::midi_queue::async_channel(16);
        supervisor.inner.midi_route.store(Some(Arc::new(tx)));
        supervisor.queue_ping().unwrap();
        assert_eq!(rx.try_recv().unwrap().bytes(), [0x90, 60, 100]);
        supervisor.request_emergency_reset();
        let (new_tx, mut new_rx) = crate::midi_queue::async_channel(16);
        supervisor.inner.midi_route.store(Some(Arc::new(new_tx)));
        std::thread::sleep(Duration::from_millis(300));
        assert!(rx.try_recv().is_err());
        assert!(new_rx.try_recv().is_err());
    }

    #[test]
    fn delayed_ping_after_reload_cannot_panic_the_replacement_worker() {
        let supervisor = VstWorkerSupervisor::new();
        let (tx, mut rx) = crate::midi_queue::async_channel(16);
        supervisor.inner.midi_route.store(Some(Arc::new(tx)));
        supervisor.queue_ping().unwrap();
        assert_eq!(rx.try_recv().unwrap().bytes(), [0x90, 60, 100]);
        drop(rx);
        supervisor
            .inner
            .lifecycle_generation
            .fetch_add(1, Ordering::AcqRel);
        let (new_tx, mut new_rx) = crate::midi_queue::async_channel(16);
        supervisor.inner.midi_route.store(Some(Arc::new(new_tx)));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(supervisor.inner.midi_generation.load(Ordering::Acquire), 0);
        assert!(!supervisor
            .inner
            .emergency_reset_pending
            .load(Ordering::Acquire));
        assert!(new_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stalled_control_write_does_not_suspend_surveillance() {
        let (writer, _reader) = tokio::io::duplex(1);
        let (tx, rx) = mpsc::channel(2);
        tx.send(ControlMessage::SaveState { request_id: 1 })
            .await
            .unwrap();
        let mut writer = tokio::spawn(control_writer(writer, rx, Duration::from_millis(100)));
        let mut poll = tokio::time::interval(Duration::from_millis(5));
        let mut polls = 0;
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                tokio::select! {
                    result = &mut writer => break result.unwrap(),
                    _ = poll.tick() => polls += 1,
                }
            }
        })
        .await
        .unwrap();
        assert!(
            polls > 1,
            "control write suspended process/heartbeat polling"
        );
        assert!(result.unwrap_err().contains("control write timed out"));
    }

    #[tokio::test]
    async fn control_writer_preserves_order_across_partial_writes() {
        let (writer, mut reader) = tokio::io::duplex(7);
        let (tx, rx) = mpsc::channel(2);
        let task = tokio::spawn(control_writer(writer, rx, Duration::from_secs(2)));
        for request_id in [1, 2] {
            tx.send(ControlMessage::SaveState { request_id })
                .await
                .unwrap();
        }
        drop(tx);
        for request_id in [1, 2] {
            assert_eq!(
                read_control_frame(&mut reader).await.unwrap(),
                ControlMessage::SaveState { request_id }
            );
        }
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn rejected_control_admission_does_not_change_recovery_settings() {
        let supervisor = VstWorkerSupervisor::new();
        *supervisor.inner.last_load.lock() = Some((AudioSettings::default(), None));
        let (commands, mut rx) = mpsc::channel(1);
        let task = tauri::async_runtime::spawn(std::future::pending());
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands,
            task,
        });
        assert!(supervisor.try_set_gain(-60.0));
        assert!(!supervisor.try_set_gain(-12.0));
        assert_eq!(
            supervisor
                .inner
                .last_load
                .lock()
                .as_ref()
                .unwrap()
                .0
                .gain_db,
            -60.0
        );
        rx.recv().await.unwrap();
        assert!(supervisor.try_set_limiter_enabled(true));
        rx.recv().await.unwrap();
        let eq = crate::config::VstEqSettings {
            enabled: true,
            ..Default::default()
        };
        assert!(supervisor.try_set_eq(eq.clone()));
        let load = supervisor.inner.last_load.lock();
        assert!(load.as_ref().unwrap().0.limiter_enabled);
        assert_eq!(load.as_ref().unwrap().0.vst_eq, eq);
    }

    #[tokio::test]
    async fn command_timeout_includes_waiting_for_admission() {
        let supervisor = VstWorkerSupervisor::new();
        let (commands, _rx) = mpsc::channel(1);
        commands
            .try_send(SessionCommand::Notify(ControlMessage::Ping { sequence: 1 }))
            .unwrap();
        let task = tauri::async_runtime::spawn(std::future::pending());
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands,
            task,
        });
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            supervisor.request_with_timeout(
                ControlMessage::SaveState { request_id: 1 },
                Duration::from_millis(20),
            ),
        )
        .await
        .expect("admission wait escaped the command deadline")
        .unwrap_err();
        assert!(error.contains("command timed out"));
    }

    #[tokio::test]
    async fn stalled_emergency_reset_expires_and_marks_midi_transport_failed() {
        let supervisor = VstWorkerSupervisor::new();
        let (writer, _reader) = tokio::io::duplex(1);
        let (_tx, rx) = crate::midi_queue::async_channel(8);
        supervisor.request_emergency_reset();
        let mut task = tokio::spawn(midi_writer(writer, rx, supervisor.inner.clone(), 0));
        let result = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
        task.abort();
        assert!(
            result.is_ok(),
            "emergency MIDI reset escaped the write deadline"
        );
        assert!(supervisor.snapshot().midi_transport_failed);
    }

    #[tokio::test]
    async fn emergency_reset_discards_backlog_and_precedes_new_midi() {
        tokio::time::timeout(Duration::from_secs(2), async {
            let supervisor = VstWorkerSupervisor::new();
            let (writer, mut reader) = tokio::io::duplex(256);
            let (tx, rx) = crate::midi_queue::async_channel(8);
            tx.send(MidiWireFrame::new(1, 1, &[0x90, 60, 100]).unwrap())
                .await
                .unwrap();
            supervisor.request_emergency_reset();
            let mut fresh = MidiWireFrame::new(2, 2, &[0x90, 62, 100]).unwrap();
            fresh.generation = supervisor.inner.midi_generation.load(Ordering::Acquire);
            tx.send(fresh).await.unwrap();
            let task = tokio::spawn(midi_writer(writer, rx, supervisor.inner.clone(), 0));
            let reset = super::super::protocol::read_midi_frame(&mut reader)
                .await
                .unwrap();
            assert_eq!(reset.bytes(), &[0xFF]);
            let fresh = super::super::protocol::read_midi_frame(&mut reader)
                .await
                .unwrap();
            assert_eq!(fresh.bytes(), &[0x90, 62, 100]);
            for bytes in [[0x90, 61, 100], [0x80, 61, 0]] {
                let mut frame = MidiWireFrame::new(2, 2, &bytes).unwrap();
                frame.generation = supervisor.inner.midi_generation.load(Ordering::Acquire);
                tx.send(frame).await.unwrap();
                let received = super::super::protocol::read_midi_frame(&mut reader)
                    .await
                    .unwrap();
                assert_eq!(received.bytes(), bytes);
            }
            drop(tx);
            task.await.unwrap();
        })
        .await
        .expect("MIDI reset writer timed out");
    }

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

    #[tokio::test]
    async fn stop_invalidates_a_delayed_automatic_restart() {
        let supervisor = VstWorkerSupervisor::new();
        *supervisor.inner.last_load.lock() = Some((AudioSettings::default(), None));
        supervisor.schedule_restart_after_fault(0);
        supervisor.stop().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(supervisor.snapshot().state, VstWorkerState::Disabled);
        assert!(supervisor.inner.session.lock().is_none());
        assert!(supervisor.inner.midi_route.load().is_none());
    }

    #[tokio::test]
    async fn only_the_last_public_owner_cancels_the_worker_session() {
        struct OnDrop(Option<oneshot::Sender<()>>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let supervisor = VstWorkerSupervisor::new();
        let survivor = supervisor.clone();
        let internal = supervisor.task_context();
        let (commands, _rx) = mpsc::channel(1);
        let (started_tx, started_rx) = oneshot::channel();
        let (ended_tx, mut ended_rx) = oneshot::channel();
        let task = tauri::async_runtime::spawn(async move {
            let _on_drop = OnDrop(Some(ended_tx));
            started_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands,
            task,
        });
        started_rx.await.unwrap();
        drop(supervisor);
        assert!(matches!(
            ended_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(survivor);
        tokio::time::timeout(Duration::from_secs(2), ended_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(internal.inner.session.lock().is_none());
    }

    #[tokio::test]
    async fn cancelling_stop_cancels_the_owned_session_task() {
        let supervisor = VstWorkerSupervisor::new();
        let (commands, mut requests) = mpsc::channel(1);
        let (received_tx, received_rx) = oneshot::channel();
        let (ended_tx, ended_rx) = oneshot::channel::<()>();
        let task = tauri::async_runtime::spawn(async move {
            let _ended = ended_tx;
            let _request = requests.recv().await.unwrap();
            received_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands,
            task,
        });
        let stopping = supervisor.clone();
        let stop = tokio::spawn(async move { stopping.stop().await });
        tokio::time::timeout(Duration::from_secs(2), received_rx)
            .await
            .unwrap()
            .unwrap();
        stop.abort();
        let _ = stop.await;
        assert!(tokio::time::timeout(Duration::from_secs(2), ended_rx)
            .await
            .expect("cancelled Stop detached its worker session")
            .is_err());
        assert!(supervisor.inner.session.lock().is_none());
        assert!(supervisor.inner.midi_route.load().is_none());
    }

    #[tokio::test]
    async fn stop_waits_for_session_completion_after_audio_acknowledgement() {
        let supervisor = VstWorkerSupervisor::new();
        let (tx, mut rx) = mpsc::channel(2);
        let release = Arc::new(tokio::sync::Notify::new());
        let task_release = release.clone();
        let (ack_tx, ack_rx) = oneshot::channel();
        let task = tauri::async_runtime::spawn(async move {
            if let Some(SessionCommand::Request {
                message: ControlMessage::Stop { request_id },
                response,
            }) = rx.recv().await
            {
                response
                    .send(Ok(ControlMessage::Stopped { request_id }))
                    .unwrap();
            } else {
                panic!("expected Stop request");
            }
            assert!(matches!(rx.recv().await, Some(SessionCommand::Shutdown)));
            ack_tx.send(()).unwrap();
            task_release.notified().await;
        });
        *supervisor.inner.session.lock() = Some(ActiveSession {
            generation: 0,
            commands: tx,
            task,
        });
        let stopping = supervisor.clone();
        let mut stop = tokio::spawn(async move { stopping.stop().await });
        ack_rx.await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut stop)
            .await
            .is_err());
        release.notify_one();
        stop.await.unwrap().unwrap();
        assert_eq!(supervisor.snapshot().state, VstWorkerState::Disabled);
    }

    #[test]
    fn midi_submission_does_not_wait_for_control_or_status_locks() {
        let supervisor = VstWorkerSupervisor::new();
        let (tx, mut rx) = crate::midi_queue::async_channel(1);
        supervisor.inner.midi_route.store(Some(Arc::new(tx)));
        let _session = supervisor.inner.session.lock();
        let status = supervisor.inner.status.lock();
        assert!(supervisor.try_send_midi(&[0x90, 60, 100]));
        assert!(!supervisor.try_send_midi(&[0x80, 60, 0]));
        assert_eq!(supervisor.inner.midi_drops.load(Ordering::Relaxed), 1);
        assert_eq!(rx.try_recv().unwrap().bytes(), &[0x90, 60, 100]);
        drop(status);
        assert_eq!(supervisor.snapshot().midi_drops, 1);
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
