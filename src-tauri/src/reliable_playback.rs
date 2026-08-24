use crossbeam_channel::Sender;
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
        let wake_addr = SocketAddr::new(
            "127.0.0.1".parse().expect("loopback"),
            self.local_addr.port(),
        );
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
                let scheduler_workers = Arc::clone(&workers);
                let permit = ConnectionPermit(Arc::clone(&active_connections));
                let handle = thread::spawn(move || {
                    let _permit = permit;
                    handle_connection(stream, tx, stop, scheduler_workers);
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

fn handle_connection(
    mut stream: TcpStream,
    tx: Sender<MidiFrame>,
    server_stop: Arc<AtomicBool>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let writer = match stream.try_clone() {
        Ok(writer) => Arc::new(Mutex::new(writer)),
        Err(err) => {
            log::warn!("Reliable playback stream clone failed: {err}");
            return;
        }
    };

    let prepared = Arc::new(match read_client_frame(&mut stream, &server_stop, false) {
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
    });

    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let mut started = false;

    while !server_stop.load(Ordering::Relaxed) && !done.load(Ordering::Relaxed) {
        match read_client_frame(&mut stream, &server_stop, true) {
            Ok(ClientFrame::Start {
                session_id,
                start_delay_ms,
            }) if session_id == prepared.session_id && !started => {
                started = true;
                *ACTIVE_SESSION.lock() = Some(session_id.clone());
                let tx = tx.clone();
                let writer = Arc::clone(&writer);
                let cancel = Arc::clone(&cancel);
                let done = Arc::clone(&done);
                let session = Arc::clone(&prepared);
                let server_stop = Arc::clone(&server_stop);
                let handle = thread::spawn(move || {
                    run_schedule(
                        session,
                        start_delay_ms,
                        tx,
                        writer,
                        cancel,
                        done,
                        server_stop,
                    );
                });
                workers.lock().push(handle);
            }
            Ok(ClientFrame::Stop { session_id }) if session_id == prepared.session_id => {
                cancel.store(true, Ordering::Relaxed);
                inject_all_notes_off(&tx, &format!("ReliablePLV:{}:stop", prepared.song));
                *ACTIVE_SESSION.lock() = None;
                let _ = send_server_frame(
                    &writer,
                    &ServerFrame::Stopped {
                        session_id: &prepared.session_id,
                    },
                );
                done.store(true, Ordering::Relaxed);
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
                break;
            }
            Err(err)
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => {}
            Err(_) => break,
        }
    }

    if started && !done.load(Ordering::Relaxed) {
        cancel.store(true, Ordering::Relaxed);
        if !server_stop.load(Ordering::Relaxed) {
            inject_all_notes_off(&tx, &format!("ReliablePLV:{}:stop", prepared.song));
        }
        *ACTIVE_SESSION.lock() = None;
        done.store(true, Ordering::Relaxed);
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

fn run_schedule(
    session: Arc<PreparedSession>,
    start_delay_ms: u64,
    tx: Sender<MidiFrame>,
    writer: Arc<Mutex<TcpStream>>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    server_stop: Arc<AtomicBool>,
) {
    let source: Arc<str> = Arc::from(format!("ReliablePLV:{}", session.song));
    let start = Instant::now() + Duration::from_millis(start_delay_ms);
    let mut processed = 0u64;
    let mut dropped = 0u64;
    let mut max_late_us = 0u64;
    let mut index = 0usize;

    while index < session.events.len() {
        if cancel.load(Ordering::Relaxed) || server_stop.load(Ordering::Relaxed) {
            *ACTIVE_SESSION.lock() = None;
            done.store(true, Ordering::Relaxed);
            return;
        }
        let due_us = session.events[index].due_us;
        let target = start + Duration::from_micros(due_us);
        wait_until(target, &cancel, &server_stop);
        if cancel.load(Ordering::Relaxed) || server_stop.load(Ordering::Relaxed) {
            *ACTIVE_SESSION.lock() = None;
            done.store(true, Ordering::Relaxed);
            return;
        }
        let late_us = Instant::now().saturating_duration_since(target).as_micros() as u64;
        max_late_us = max_late_us.max(late_us);
        record_max_late(max_late_us);

        while index < session.events.len() && session.events[index].due_us == due_us {
            let event = &session.events[index];
            let frame = MidiFrame {
                data: SmallVec::from_slice(&event.data),
                source: Arc::clone(&source),
            };
            match try_enqueue_midi_frame(&tx, frame) {
                Ok(crate::bridge::pipeline::EnqueueOutcome::Enqueued) => {
                    processed += 1;
                    MESSAGES_OUT.fetch_add(1, Ordering::Relaxed);
                }
                Ok(crate::bridge::pipeline::EnqueueOutcome::DroppedNonCritical) | Err(_) => {
                    dropped += 1;
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
            }
            index += 1;
        }
    }

    *ACTIVE_SESSION.lock() = None;
    let _ = send_server_frame(
        &writer,
        &ServerFrame::Completed {
            session_id: &session.session_id,
            processed,
            max_late_us,
            dropped,
        },
    );
    done.store(true, Ordering::Relaxed);
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
            thread::sleep(remaining - Duration::from_millis(1));
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
    return_on_idle_timeout: bool,
) -> std::io::Result<ClientFrame> {
    read_frame(stream, stop, return_on_idle_timeout)
}

fn read_frame(
    stream: &mut TcpStream,
    stop: &AtomicBool,
    return_on_idle_timeout: bool,
) -> std::io::Result<ClientFrame> {
    let mut header = [0u8; 4];
    read_exact_preserving_progress(stream, &mut header, stop, return_on_idle_timeout)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut data = vec![0u8; size];
    read_exact_preserving_progress(stream, &mut data, stop, false)?;
    serde_json::from_slice(&data).map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))
}

fn read_exact_preserving_progress(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    stop: &AtomicBool,
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
                if offset == 0 && return_on_idle_timeout {
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
