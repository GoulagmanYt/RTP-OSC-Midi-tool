//! Dedicated, cancellable MIDI I/O executors. Resources must be created on
//! this executor so their reactor does not depend on the desktop runtime.
use std::{future::Future, sync::mpsc, thread};

pub struct MidiRuntime {
    handle: tokio::runtime::Handle,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MidiRuntime {
    pub fn new(name: &str) -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _priority = crate::audio::windows_tuning::prioritize_midi_thread();
                if ready_tx.send(Ok(runtime.handle().clone())).is_err() {
                    return;
                }
                runtime.block_on(async {
                    let _ = stopped.await;
                });
                // Dropping the runtime cancels pending I/O and all of its tasks.
            })
            .map_err(|error| error.to_string())?;
        let handle = ready_rx.recv().map_err(|error| error.to_string())??;
        Ok(Self {
            handle,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(future)
    }
}

impl Drop for MidiRuntime {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn reserve(capacity: usize) -> usize {
    256.min(capacity / 8)
}

pub(crate) fn configure_udp(socket: &std::net::UdpSocket) -> Result<usize, String> {
    let socket = socket2::SockRef::from(socket);
    socket
        .set_recv_buffer_size(1024 * 1024)
        .map_err(|e| format!("UDP receive buffer: {e}"))?;
    socket
        .recv_buffer_size()
        .map_err(|e| format!("UDP receive buffer query: {e}"))
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDiagnostics {
    pub name: String,
    pub mmcss_profile: String,
    pub high_priority: bool,
    pub fallback_applied: bool,
}
static THREADS: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::BTreeMap<String, ThreadDiagnostics>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::BTreeMap::new()));
static PUBLISHED_THREADS: std::sync::LazyLock<arc_swap::ArcSwap<Vec<ThreadDiagnostics>>> =
    std::sync::LazyLock::new(|| arc_swap::ArcSwap::from_pointee(Vec::new()));
pub(crate) fn thread_snapshot() -> Vec<ThreadDiagnostics> {
    PUBLISHED_THREADS.load().as_ref().clone()
}
pub(crate) fn record_thread(profile: &str, high: bool, fallback: bool) {
    let name = thread::current()
        .name()
        .unwrap_or("midi-unnamed")
        .to_owned();
    let mut threads = THREADS.lock();
    threads.insert(
        name.clone(),
        ThreadDiagnostics {
            name,
            mmcss_profile: profile.to_owned(),
            high_priority: high,
            fallback_applied: fallback,
        },
    );
    PUBLISHED_THREADS.store(std::sync::Arc::new(threads.values().cloned().collect()));
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeDiagnostics {
    pub worker: crate::vst_worker::WorkerMetrics,
    pub queues: Vec<crate::midi_queue::QueueDiagnostics>,
    pub threads: Vec<ThreadDiagnostics>,
    pub heartbeat_delayed: bool,
    pub transport_healthy: Option<bool>,
    pub control_healthy: Option<bool>,
    pub callback_healthy: Option<bool>,
    pub network: NetworkDiagnostics,
    pub common_resets: u64,
    pub dropped_diagnostic_logs: u64,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDiagnostics {
    pub rtp_midi_receive_buffer_bytes: usize,
    pub rtp_control_receive_buffer_bytes: usize,
    pub rtp_invalid_datagrams: u64,
    pub rtp_socket_errors: u64,
    pub rtp_lost_packets: u64,
    pub osc_receive_buffer_bytes: usize,
    pub osc_invalid_datagrams: u64,
    pub osc_socket_errors: u64,
    pub osc_scheduler_overflows: u64,
}
pub(crate) static RTP_LOST_PACKETS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
pub(crate) static OSC_RECEIVE_BUFFER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
pub(crate) static OSC_INVALID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static OSC_SOCKET_ERRORS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
pub(crate) static OSC_OVERFLOWS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
pub fn diagnostics(worker: crate::vst_worker::WorkerMetrics) -> RealtimeDiagnostics {
    use std::sync::atomic::Ordering::Relaxed;
    RealtimeDiagnostics {
        worker,
        common_resets: crate::midi::MidiFrame::current_generation(),
        dropped_diagnostic_logs: crate::logger::DROPPED_LOGS.load(Relaxed),
        queues: crate::midi_queue::diagnostics(),
        threads: thread_snapshot(),
        network: NetworkDiagnostics {
            rtp_midi_receive_buffer_bytes: rtpmidi::diagnostics::MIDI_RECEIVE_BUFFER.load(Relaxed),
            rtp_control_receive_buffer_bytes: rtpmidi::diagnostics::CONTROL_RECEIVE_BUFFER
                .load(Relaxed),
            rtp_invalid_datagrams: rtpmidi::diagnostics::INVALID_DATAGRAMS.load(Relaxed),
            rtp_socket_errors: rtpmidi::diagnostics::SOCKET_ERRORS.load(Relaxed),
            rtp_lost_packets: RTP_LOST_PACKETS.load(Relaxed),
            osc_receive_buffer_bytes: OSC_RECEIVE_BUFFER.load(Relaxed),
            osc_invalid_datagrams: OSC_INVALID.load(Relaxed),
            osc_socket_errors: OSC_SOCKET_ERRORS.load(Relaxed),
            osc_scheduler_overflows: OSC_OVERFLOWS.load(Relaxed),
        },
        ..RealtimeDiagnostics::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_is_single_thread_and_cancels_pending_io_on_drop() {
        let runtime = MidiRuntime::new("midi-runtime-test").unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let task = runtime.spawn(async move {
            tx.send(thread::current().id()).unwrap();
            std::future::pending::<()>().await;
        });
        assert_ne!(rx.recv().unwrap(), thread::current().id());
        drop(runtime);
        assert!(task.is_finished());
    }
}
