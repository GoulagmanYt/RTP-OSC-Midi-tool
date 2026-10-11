use crate::midi_queue::Sender;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, LazyLock,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{bridge::pipeline::try_enqueue_midi_frame, midi::MidiFrame};

const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_RELIABLE_PLAYBACK_ADDR: &str = "0.0.0.0:5056";
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 1024;
const MAX_CONNECTIONS: usize = 8;

static MESSAGES_IN: AtomicU64 = AtomicU64::new(0);
static MESSAGES_OUT: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static MAX_LATE_US: AtomicU64 = AtomicU64::new(0);
static ACTIVE_SESSION: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReliablePlaybackMetricsSnapshot {
    pub messages_in: u64,
    pub messages_out: u64,
    pub dropped: u64,
    pub max_late_us: u64,
    pub active_session: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReliablePlaybackEvent {
    pub seq: u64,
    pub due_us: u64,
    pub data: SmallVec<[u8; 3]>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClientFrame {
    Prepare {
        version: u8,
        #[serde(rename = "sessionId")]
        session_id: String,
        song: String,
        total: usize,
        events: Vec<ReliablePlaybackEvent>,
    },
    Start {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "startDelayMs")]
        start_delay_ms: u64,
    },
    Stop {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ServerFrame<'a> {
    Prepared {
        #[serde(rename = "sessionId")]
        session_id: &'a str,
        received: usize,
    },
    Completed {
        #[serde(rename = "sessionId")]
        session_id: &'a str,
        processed: u64,
        #[serde(rename = "maxLateUs")]
        max_late_us: u64,
        dropped: u64,
    },
    Stopped {
        #[serde(rename = "sessionId")]
        session_id: &'a str,
    },
    Error {
        #[serde(rename = "sessionId")]
        session_id: Option<&'a str>,
        message: String,
    },
}

#[derive(Clone)]
struct PreparedSession {
    session_id: String,
    song: String,
    events: Vec<ReliablePlaybackEvent>,
}

pub struct ReliablePlaybackServer {
    local_addr: SocketAddr,
    tx: Sender<MidiFrame>,
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

struct ConnectionPermit(Arc<AtomicUsize>);

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ReliablePlaybackServer {
    pub fn start<A: ToSocketAddrs>(addr: A, tx: Sender<MidiFrame>) -> Result<Self, String> {
        let listener = TcpListener::bind(addr)
            .map_err(|err| format!("Reliable playback bind failed: {err}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|err| format!("Reliable playback nonblocking failed: {err}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| format!("Reliable playback local addr failed: {err}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let workers = Arc::new(Mutex::new(Vec::new()));
        let workers_for_thread = Arc::clone(&workers);
        let tx_for_server = tx.clone();
        let handle =
            thread::spawn(move || accept_loop(listener, tx, stop_for_thread, workers_for_thread));

        Ok(Self {
            local_addr,
            tx: tx_for_server,
            stop,
            handle: Mutex::new(Some(handle)),
            workers,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let mut wake_addr = self.local_addr;
        if wake_addr.ip().is_unspecified() {
            wake_addr.set_ip(if wake_addr.is_ipv6() {
                std::net::Ipv6Addr::LOCALHOST.into()
            } else {
                std::net::Ipv4Addr::LOCALHOST.into()
            });
        }
        let _ = TcpStream::connect_timeout(&wake_addr, Duration::from_millis(100));
        if let Some(handle) = self.handle.lock().take() {
            let _ = handle.join();
        }
        loop {
            let handles = std::mem::take(&mut *self.workers.lock());
            if handles.is_empty() {
                break;
            }
            for handle in handles {
                let _ = handle.join();
            }
        }
        inject_all_notes_off(&self.tx, "ReliablePLV:server-stop");
        *ACTIVE_SESSION.lock() = None;
    }
}

pub fn reliable_playback_metrics_snapshot() -> ReliablePlaybackMetricsSnapshot {
    ReliablePlaybackMetricsSnapshot {
        messages_in: MESSAGES_IN.load(Ordering::Relaxed),
        messages_out: MESSAGES_OUT.load(Ordering::Relaxed),
        dropped: DROPPED.load(Ordering::Relaxed),
        max_late_us: MAX_LATE_US.load(Ordering::Relaxed),
        active_session: ACTIVE_SESSION.lock().clone(),
    }
}

pub fn reset_reliable_playback_metrics() {
    MESSAGES_IN.store(0, Ordering::Relaxed);
    MESSAGES_OUT.store(0, Ordering::Relaxed);
    DROPPED.store(0, Ordering::Relaxed);
    MAX_LATE_US.store(0, Ordering::Relaxed);
    *ACTIVE_SESSION.lock() = None;
}

fn accept_loop(
    listener: TcpListener,
    tx: Sender<MidiFrame>,
    stop: Arc<AtomicBool>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
) {
    let active_connections = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        reap_finished_workers(&workers);
        match listener.accept() {
            Ok((stream, _addr)) => {
                let acquired = active_connections
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                        (active < MAX_CONNECTIONS).then_some(active + 1)
                    })
                    .is_ok();
                if !acquired {
                    log::warn!("Reliable playback connection limit reached");
                    continue;
                }
                let tx = tx.clone();
                let stop = Arc::clone(&stop);
                let permit = ConnectionPermit(Arc::clone(&active_connections));
                let handle = thread::spawn(move || {
                    let _permit = permit;
                    handle_connection(stream, tx, stop);
                });
                workers.lock().push(handle);
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => {
                log::warn!("Reliable playback accept failed: {err}");
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn reap_finished_workers(workers: &Mutex<Vec<thread::JoinHandle<()>>>) {
    let mut finished = Vec::new();
    {
        let mut guard = workers.lock();
        let mut index = 0;
        while index < guard.len() {
            if guard[index].is_finished() {
                finished.push(guard.swap_remove(index));
            } else {
                index += 1;
            }
        }
    }
    for handle in finished {
        let _ = handle.join();
    }
}

fn handle_connection(mut stream: TcpStream, tx: Sender<MidiFrame>, server_stop: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let writer = match stream.try_clone() {
        Ok(writer) => Arc::new(Mutex::new(writer)),
        Err(err) => {
            log::warn!("Reliable playback stream clone failed: {err}");
            return;
        }
    };

    let prepared = Arc::new(
        match read_client_frame(&mut stream, &server_stop, None, false) {
            Ok(ClientFrame::Prepare {
                version,
                session_id,
                song,
                total,
                events,
            }) => match validate_prepare(version, session_id, song, total, events) {
                Ok(prepared) => {
                    MESSAGES_IN.fetch_add(prepared.events.len() as u64, Ordering::Relaxed);
                    let _ = send_server_frame(
                        &writer,
                        &ServerFrame::Prepared {
                            session_id: &prepared.session_id,
                            received: prepared.events.len(),
                        },
                    );
                    prepared
                }
                Err((session_id, message)) => {
                    let _ = send_server_frame(
                        &writer,
                        &ServerFrame::Error {
                            session_id: session_id.as_deref(),
                            message,
                        },
                    );
                    return;
                }
            },
            Ok(_) => {
                let _ = send_server_frame(
                    &writer,
                    &ServerFrame::Error {
                        session_id: None,
                        message: "first frame must be prepare".to_string(),
                    },
                );
                return;
            }
            Err(err) => {
                let _ = send_server_frame(
                    &writer,
                    &ServerFrame::Error {
                        session_id: None,
                        message: format!("read prepare failed: {err}"),
                    },
                );
                log::debug!("Reliable playback read prepare failed: {err}");
                return;
            }
        },
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let mut started = false;
    let mut scheduler = None;
    let mut stop_requested = false;
    let mut connection_failed = false;

    while !server_stop.load(Ordering::Relaxed) && !done.load(Ordering::Acquire) {
        match read_client_frame(&mut stream, &server_stop, Some(&done), true) {
            Ok(ClientFrame::Start {
                session_id,
                start_delay_ms,
            }) if session_id == prepared.session_id && !started => {
                started = true;
                *ACTIVE_SESSION.lock() = Some(session_id.clone());
                let tx = tx.clone();
                let cancel = Arc::clone(&cancel);
                let done = Arc::clone(&done);
                let session = Arc::clone(&prepared);
                let server_stop = Arc::clone(&server_stop);
                scheduler = Some(
                    thread::Builder::new()
                        .name("midi-playback".into())
                        .spawn(move || {
                            let _priority = crate::audio::windows_tuning::prioritize_midi_thread();
                            let completion =
                                run_schedule(session, start_delay_ms, tx, cancel, server_stop);
                            done.store(true, Ordering::Release);
                            completion
                        })
                        .expect("failed to spawn midi-playback thread"),
                );
            }
            Ok(ClientFrame::Stop { session_id }) if session_id == prepared.session_id => {
                stop_requested = true;
                break;
            }
            Ok(_) => {
                let _ = send_server_frame(
                    &writer,
                    &ServerFrame::Error {
                        session_id: Some(&prepared.session_id),
                        message: "unexpected reliable playback frame".to_string(),
                    },
                );
                connection_failed = true;
                break;
            }
            Err(err)
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => {}
            Err(_) => {
                connection_failed = true;
                break;
            }
        }
    }

    if stop_requested || connection_failed || server_stop.load(Ordering::Relaxed) {
        cancel.store(true, Ordering::Relaxed);
    }
    // The connection owns the scheduler and every terminal reply. Wait until
    // admission has stopped before releasing notes or acknowledging Stop/EOF.
    // Even an admission racing with the cancel check therefore precedes reset.
    let completion = scheduler.and_then(|handle| handle.join().ok()).flatten();
    if started || stop_requested {
        *ACTIVE_SESSION.lock() = None;
    }
    if server_stop.load(Ordering::Relaxed) {
        return; // The server injects its reset after joining all connections.
    }
    if stop_requested || connection_failed {
        if started || stop_requested {
            inject_all_notes_off(&tx, &format!("ReliablePLV:{}", prepared.song));
        }
        if stop_requested {
            let _ = send_server_frame(
                &writer,
                &ServerFrame::Stopped {
                    session_id: &prepared.session_id,
                },
            );
        }
    } else if let Some(completion) = completion {
        let _ = send_server_frame(
            &writer,
            &ServerFrame::Completed {
                session_id: &prepared.session_id,
                processed: completion.processed,
                max_late_us: completion.max_late_us,
                dropped: completion.dropped,
            },
        );
    }
}

fn validate_prepare(
    version: u8,
    session_id: String,
    song: String,
    total: usize,
    events: Vec<ReliablePlaybackEvent>,
) -> Result<PreparedSession, (Option<String>, String)> {
    if version != PROTOCOL_VERSION {
        return Err((
            Some(session_id),
            format!("unsupported reliable playback version {version}"),
        ));
    }
    if total != events.len() {
        return Err((
            Some(session_id),
            format!("total {total} does not match event count {}", events.len()),
        ));
    }
    let mut previous_due = 0u64;
    for (idx, event) in events.iter().enumerate() {
        if event.seq != idx as u64 {
            return Err((
                Some(session_id),
                format!("invalid seq {} at index {idx}", event.seq),
            ));
        }
        if event.due_us < previous_due {
            return Err((
                Some(session_id),
                "events must be sorted by dueUs".to_string(),
            ));
        }
        if event.due_us > 86_400_000_000 {
            return Err((
                Some(session_id),
                format!(
                    "dueUs {} exceeds maximum allowed duration (24h)",
                    event.due_us
                ),
            ));
        }
        if event.data.is_empty() {
            return Err((
                Some(session_id),
                format!("empty MIDI data at seq {}", event.seq),
            ));
        }
        if event.data.len() > MAX_EVENT_BYTES {
            return Err((
                Some(session_id),
                format!("MIDI data too large at seq {}", event.seq),
            ));
        }
        previous_due = event.due_us;
    }
    Ok(PreparedSession {
        session_id,
        song,
        events,
    })
}

struct PlaybackCompletion {
    processed: u64,
    dropped: u64,
    max_late_us: u64,
}

fn run_schedule(
    session: Arc<PreparedSession>,
    start_delay_ms: u64,
    tx: Sender<MidiFrame>,
    cancel: Arc<AtomicBool>,
    server_stop: Arc<AtomicBool>,
) -> Option<PlaybackCompletion> {
    let source: Arc<str> = Arc::from(format!("ReliablePLV:{}", session.song));
    let start = Instant::now()
        .checked_add(Duration::from_millis(start_delay_ms.min(60_000)))
        .unwrap_or_else(Instant::now);
    let mut processed = 0u64;
    let mut dropped = 0u64;
    let mut max_late_us = 0u64;
    let mut index = 0usize;

    while index < session.events.len() {
        if cancel.load(Ordering::Relaxed) || server_stop.load(Ordering::Relaxed) {
            return None;
        }
        let due_us = session.events[index].due_us;
        let target = start
            .checked_add(Duration::from_micros(due_us))
            .unwrap_or(start);
        wait_until(target, &cancel, &server_stop);
        if cancel.load(Ordering::Relaxed) || server_stop.load(Ordering::Relaxed) {
            return None;
        }
        let late_us = Instant::now().saturating_duration_since(target).as_micros() as u64;
        max_late_us = max_late_us.max(late_us);
        record_max_late(max_late_us);

        while index < session.events.len() && session.events[index].due_us == due_us {
            if cancel.load(Ordering::Relaxed) || server_stop.load(Ordering::Relaxed) {
                return None;
            }
            let event = &session.events[index];
            let frame = MidiFrame {
                generation: MidiFrame::current_generation(),
                ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                data: SmallVec::from_slice(&event.data),
                source: Arc::clone(&source),
            };
            match try_enqueue_midi_frame(&tx, frame) {
                Ok(
                    crate::bridge::pipeline::EnqueueOutcome::Enqueued
                    | crate::bridge::pipeline::EnqueueOutcome::Filtered,
                ) => {
                    processed += 1;
                    MESSAGES_OUT.fetch_add(1, Ordering::Relaxed);
                }
                Err(crate::bridge::pipeline::EnqueueError::Disconnected) => {
                    return None;
                }
                Ok(crate::bridge::pipeline::EnqueueOutcome::DroppedNonCritical) | Err(_) => {
                    dropped += 1;
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
            }
            index += 1;
        }
    }

    Some(PlaybackCompletion {
        processed,
        max_late_us,
        dropped,
    })
}

fn wait_until(target: Instant, cancel: &AtomicBool, server_stop: &AtomicBool) {
    loop {
        if cancel.load(Ordering::Relaxed) || server_stop.load(Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        if now >= target {
            return;
        }
        let remaining = target - now;
        if remaining > Duration::from_millis(2) {
            // Recheck cancellation even during a long silence or start delay.
            thread::sleep((remaining - Duration::from_millis(1)).min(Duration::from_millis(5)));
        } else {
            thread::yield_now();
        }
    }
}

fn inject_all_notes_off(tx: &Sender<MidiFrame>, source: &str) {
    let source: Arc<str> = Arc::from(source.to_string());
    for channel in 0u8..16 {
        for data in [[0xB0 | channel, 64, 0], [0xB0 | channel, 123, 0]] {
            let _ = try_enqueue_midi_frame(
                tx,
                MidiFrame {
                    generation: MidiFrame::current_generation(),
                    ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
                    data: SmallVec::from_slice(&data),
                    source: Arc::clone(&source),
                },
            );
        }
    }
}

fn record_max_late(late_us: u64) {
    let mut current = MAX_LATE_US.load(Ordering::Relaxed);
    while late_us > current {
        match MAX_LATE_US.compare_exchange(current, late_us, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn read_client_frame(
    stream: &mut TcpStream,
    stop: &AtomicBool,
    done: Option<&AtomicBool>,
    return_on_idle_timeout: bool,
) -> std::io::Result<ClientFrame> {
    read_frame(stream, stop, done, return_on_idle_timeout)
}

fn read_frame(
    stream: &mut TcpStream,
    stop: &AtomicBool,
    done: Option<&AtomicBool>,
    return_on_idle_timeout: bool,
) -> std::io::Result<ClientFrame> {
    let mut header = [0u8; 4];
    read_exact_preserving_progress(stream, &mut header, stop, done, return_on_idle_timeout)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut data = vec![0u8; size];
    read_exact_preserving_progress(stream, &mut data, stop, done, false)?;
    serde_json::from_slice(&data).map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))
}

fn read_exact_preserving_progress(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    stop: &AtomicBool,
    done: Option<&AtomicBool>,
    return_on_idle_timeout: bool,
) -> std::io::Result<()> {
    let mut offset = 0usize;
    while offset < buffer.len() {
        if stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                ErrorKind::Interrupted,
                "server stopping",
            ));
        }
        if done.map(|d| d.load(Ordering::Acquire)).unwrap_or(false) {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "playback completed",
            ));
        }
        match stream.read(&mut buffer[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "connection closed",
                ))
            }
            Ok(read) => offset += read,
            Err(err)
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut =>
            {
                if (offset == 0 && return_on_idle_timeout)
                    || done.map(|d| d.load(Ordering::Acquire)).unwrap_or(false)
                {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn send_server_frame(
    writer: &Arc<Mutex<TcpStream>>,
    frame: &ServerFrame<'_>,
) -> std::io::Result<()> {
    let data = serde_json::to_vec(frame)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))?;
    let mut guard = writer.lock();
    guard.write_all(&(data.len() as u32).to_be_bytes())?;
    guard.write_all(&data)?;
    guard.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cancellation_during_equal_deadline_batch_stops_before_reset() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        // Pause a real admission after its cancellation check. Joining the
        // scheduler must allow that one admission to finish before reset, and
        // cancellation must prevent the remaining event in the same batch.
        for server_shutdown in [false, true] {
            let (tx, rx) = crate::midi_queue::bounded(64);
            let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
            let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
            let resume_rx = Mutex::new(resume_rx);
            tx.set_preparation(move |frame: &mut MidiFrame| {
                if frame.data.as_slice() == [0x90, 61, 100] {
                    entered_tx.send(()).unwrap();
                    resume_rx.lock().recv().unwrap();
                }
                true
            });
            let session = Arc::new(PreparedSession {
                session_id: "cancel-batch".into(),
                song: "batch.mid".into(),
                events: (60..63)
                    .enumerate()
                    .map(|(seq, note)| ReliablePlaybackEvent {
                        seq: seq as u64,
                        due_us: 0,
                        data: smallvec::smallvec![0x90, note, 100],
                    })
                    .collect(),
            });
            let cancel = Arc::new(AtomicBool::new(false));
            let server_stop = Arc::new(AtomicBool::new(false));
            let scheduler = thread::spawn({
                let tx = tx.clone();
                let cancel = cancel.clone();
                let server_stop = server_stop.clone();
                move || run_schedule(session, 0, tx, cancel, server_stop)
            });
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if server_shutdown {
                server_stop.store(true, Ordering::Relaxed);
            } else {
                cancel.store(true, Ordering::Relaxed);
            }
            resume_tx.send(()).unwrap();
            assert!(scheduler.join().unwrap().is_none());
            inject_all_notes_off(&tx, "ReliablePLV:batch.mid");
            let frames: Vec<_> = rx.try_iter().collect();
            assert_eq!(frames.len(), 34);
            assert_eq!(frames[0].data.as_slice(), &[0x90, 60, 100]);
            assert_eq!(frames[1].data.as_slice(), &[0x90, 61, 100]);
            assert!(frames[2..].iter().all(|frame| frame.data[0] & 0xF0 == 0xB0));
            assert_eq!(frames.last().unwrap().data.as_slice(), &[0xBF, 123, 0]);
        }
    }

    #[test]
    fn client_frame_deserializes_prepare_protocol_shape() {
        let frame: ClientFrame = serde_json::from_value(json!({
            "type": "prepare",
            "version": 1,
            "sessionId": "session-1",
            "song": "dup.mid",
            "total": 1,
            "events": [
                {"seq": 0, "dueUs": 0, "data": [0x90, 60, 100]}
            ]
        }))
        .expect("prepare frame should deserialize");

        match frame {
            ClientFrame::Prepare {
                version,
                session_id,
                song,
                total,
                events,
            } => {
                assert_eq!(version, 1);
                assert_eq!(session_id, "session-1");
                assert_eq!(song, "dup.mid");
                assert_eq!(total, 1);
                assert_eq!(events[0].due_us, 0);
                assert_eq!(events[0].data.as_slice(), &[0x90, 60, 100]);
            }
            _ => panic!("expected prepare frame"),
        }
    }

    #[test]
    fn server_frame_serializes_protocol_type_names() {
        let frame = serde_json::to_value(ServerFrame::Prepared {
            session_id: "session-1",
            received: 1,
        })
        .expect("prepared frame should serialize");

        assert_eq!(frame["type"], "prepared");
        assert_eq!(frame["sessionId"], "session-1");
        assert_eq!(frame["received"], 1);
    }
}
