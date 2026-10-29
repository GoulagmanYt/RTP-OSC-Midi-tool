//! Optional OSC input with borrowed decoding and a bounded deadline queue.
mod packet;
use crate::midi_queue::Sender;
use crate::{
    bridge::pipeline::{
        classify_input_frame, midi_reset_generation, request_critical_midi_reset,
        try_enqueue_midi_frame,
    },
    config::OscConfig,
    logger::{background_log, FrontendLogger},
    midi::MidiFrame,
};
use packet::{ScheduledMidi, MAX_PACKET_MESSAGES};
use parking_lot::Mutex;
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
const PENDING_CAPACITY: usize = 4096;
const SOCKET_ERROR_LIMIT: u64 = 20;

#[derive(Clone, Default)]
struct ReceiverHealth {
    error: Arc<Mutex<Option<String>>>,
    logger: Option<FrontendLogger>,
}

impl ReceiverHealth {
    fn failed(&self, message: String) {
        *self.error.lock() = Some(message.clone());
        if let Some(logger) = &self.logger {
            // Uses the existing log:entry event as well as the background log.
            logger.error(message);
        } else {
            background_log("error", message);
        }
    }

    fn recovered(&self, address: SocketAddr) {
        *self.error.lock() = None;
        let message = format!("OSC input receiver resumed on {address}");
        if let Some(logger) = &self.logger {
            logger.info(message);
        } else {
            background_log("info", message);
        }
    }
}

fn bind_socket(address: SocketAddr) -> Result<UdpSocket, String> {
    let socket = UdpSocket::bind(address).map_err(|e| e.to_string())?;
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    let effective_buffer = crate::realtime::configure_udp(&socket)?;
    crate::realtime::OSC_RECEIVE_BUFFER
        .store(effective_buffer, std::sync::atomic::Ordering::Relaxed);
    Ok(socket)
}

fn socket_error_backoff(errors: u64) -> Option<Duration> {
    (errors < SOCKET_ERROR_LIMIT).then(|| Duration::from_millis(50 * errors))
}

fn reopen_backoff(attempt: u64) -> Duration {
    Duration::from_secs(attempt.clamp(1, 5))
}

async fn reopen_socket(
    address: SocketAddr,
    stop: &mut oneshot::Receiver<()>,
    health: &ReceiverHealth,
) -> Option<tokio::net::UdpSocket> {
    let mut attempt = 1u64;
    loop {
        tokio::select! {
            biased;
            _ = &mut *stop => return None,
            _ = tokio::time::sleep(reopen_backoff(attempt)) => {}
        }
        match bind_socket(address)
            .and_then(|socket| tokio::net::UdpSocket::from_std(socket).map_err(|e| e.to_string()))
        {
            Ok(socket) => {
                health.recovered(address);
                return Some(socket);
            }
            Err(error) => {
                health.failed(format!(
                    "OSC input receiver cannot reopen {address}: {error}; retrying"
                ));
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

pub(crate) struct OscInputServer {
    _runtime: crate::realtime::MidiRuntime,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    health: ReceiverHealth,
}
impl OscInputServer {
    pub(crate) fn start(
        config: &OscConfig,
        sink: Sender<MidiFrame>,
        logger: Option<FrontendLogger>,
    ) -> Result<Option<Self>, String> {
        if !config.input_enabled {
            return Ok(None);
        }
        let ip = config
            .listen_ip
            .parse::<IpAddr>()
            .map_err(|e| e.to_string())?;
        if config.listen_port == 0 {
            return Err("OSC listening port must be nonzero".into());
        }
        let socket = bind_socket(SocketAddr::new(ip, config.listen_port))?;
        let source: Arc<str> =
            Arc::from(format!("OSC:{}:{}", config.listen_ip, config.listen_port));
        let (stop, cancelled) = oneshot::channel();
        let runtime = crate::realtime::MidiRuntime::new("osc-receive")?;
        let health = ReceiverHealth {
            logger,
            ..Default::default()
        };
        let task = runtime.spawn(receive(socket, sink, source, cancelled, health.clone()));
        Ok(Some(Self {
            _runtime: runtime,
            stop: Some(stop),
            task: Some(task),
            health,
        }))
    }
    pub(crate) fn last_error(&self) -> Option<String> {
        self.health.error.lock().clone().or_else(|| {
            self.task
                .as_ref()
                .filter(|task| task.is_finished())
                .map(|_| "OSC input receiver stopped unexpectedly".to_string())
        })
    }
    pub(crate) fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = crate::tauri::utils::safe_block_on(task);
        }
    }
}
impl Drop for OscInputServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn scheduled_frame(event: &ScheduledMidi, source: Arc<str>) -> MidiFrame {
    MidiFrame {
        generation: event.generation,
        // Project the due time at dispatch, after it has passed. Receipt time
        // would collapse distinct deadlines when a delayed batch reaches audio.
        ingress_qpc: crate::vst_worker::protocol::qpc_at(event.deadline),
        data: smallvec::SmallVec::from_slice(&event.data[..usize::from(event.len)]),
        source,
    }
}

async fn receive(
    socket: UdpSocket,
    sink: Sender<MidiFrame>,
    source: Arc<str>,
    mut stop: oneshot::Receiver<()>,
    health: ReceiverHealth,
) {
    let address = match socket.local_addr() {
        Ok(address) => address,
        Err(error) => {
            health.failed(format!(
                "OSC input receiver cannot read its address: {error}"
            ));
            request_critical_midi_reset();
            return;
        }
    };
    let mut socket = match tokio::net::UdpSocket::from_std(socket) {
        Ok(socket) => socket,
        Err(error) => {
            health.failed(format!("OSC input receiver stopped on {address}: {error}"));
            request_critical_midi_reset();
            return;
        }
    };
    let mut buffer = vec![0; 65_536];
    let mut staging = Vec::with_capacity(MAX_PACKET_MESSAGES);
    let mut pending: BinaryHeap<Reverse<ScheduledMidi>> =
        BinaryHeap::with_capacity(PENDING_CAPACITY);
    let mut order = 0u64;
    let mut generation = midi_reset_generation();
    let mut errors = 0u64;
    loop {
        let current = midi_reset_generation();
        if generation != current {
            pending.clear();
            generation = current;
        }

        // Drain any pending events that are already due:
        let now = Instant::now();
        for _ in 0..256 {
            if pending.peek().is_none_or(|event| event.0.deadline > now) {
                break;
            }
            let event = pending.pop().unwrap().0;
            if event.generation != midi_reset_generation() {
                continue;
            }
            let frame = scheduled_frame(&event, source.clone());
            let _ = try_enqueue_midi_frame(&sink, frame);
            if event.generation != midi_reset_generation() {
                pending.clear();
                break;
            }
        }

        let due = pending.peek().map(|event| event.0.deadline);
        tokio::select! {
            biased;
            _ = &mut stop => break,
            _ = async { if let Some(due) = due { tokio::time::sleep_until(due.into()).await; } }, if due.is_some() => {
                let now = Instant::now();
                for _ in 0..256 {
                    if pending.peek().is_none_or(|event| event.0.deadline > now) { break; }
                    let event = pending.pop().unwrap().0;
                    if event.generation != midi_reset_generation() { continue; }
                    let frame = scheduled_frame(&event, source.clone());
                    let _ = try_enqueue_midi_frame(&sink, frame);
                    // Admission already recovers a lost effective release. Observe its epoch;
                    // raw CC64 may have been remapped to a noncritical controller.
                    if event.generation != midi_reset_generation() { pending.clear(); break; }
                }
            },
            received = socket.recv_from(&mut buffer) => {
                let (size, _) = match received {
                    Ok(received) => received,
                    Err(error) => {
                    crate::realtime::OSC_SOCKET_ERRORS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    errors += 1;
                    if let Some(delay) = socket_error_backoff(errors) {
                        tokio::select! {
                            biased;
                            _ = &mut stop => break,
                            _ = tokio::time::sleep(delay) => {}
                        }
                    } else {
                        health.failed(format!("OSC input receiver stopped on {address} after {errors} socket errors: {error}; reopening"));
                        request_critical_midi_reset();
                        pending.clear();
                        drop(socket);
                        let Some(reopened) = reopen_socket(address, &mut stop, &health).await else { break; };
                        socket = reopened;
                        errors = 0;
                    }
                    continue;
                    }
                };
                errors = 0;
                let now = Instant::now();
                let received_generation = midi_reset_generation();
                if generation != received_generation { pending.clear(); generation = received_generation; }
                if packet::decode(&buffer[..size], SystemTime::now(), now, generation, &mut staging).is_err() {
                    crate::realtime::OSC_INVALID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                for (index, mut event) in staging.drain(..).enumerate() {
                    let candidate = MidiFrame { generation: event.generation, ingress_qpc: 0, data: smallvec::SmallVec::from_slice(&event.data[..usize::from(event.len)]), source: source.clone() };
                    let critical = classify_input_frame(&sink, &candidate);
                    let limit = if critical { PENDING_CAPACITY } else { PENDING_CAPACITY - crate::realtime::reserve(PENDING_CAPACITY) };
                    if pending.len() >= limit {
                        crate::realtime::OSC_OVERFLOWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if critical { request_critical_midi_reset(); pending.clear(); break; }
                        continue;
                    }

                    // Direct zero-latency fast-path for immediate events:
                    if pending.is_empty() && event.deadline <= now {
                        let _ = try_enqueue_midi_frame(&sink, scheduled_frame(&event, source.clone()));
                        if event.generation != midi_reset_generation() {
                            pending.clear();
                            break;
                        }
                    } else {
                        event.order = order; order = order.wrapping_add(1);
                        pending.push(Reverse(event));
                    }
                    if (index + 1) % 256 == 0 { tokio::task::yield_now().await; }
                }
            }
        }
    }
    request_critical_midi_reset();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage, OscPacket, OscType};
    use std::time::{Duration, UNIX_EPOCH};
    #[test]
    fn socket_error_policy_reopens_after_twenty_errors_with_bounded_backoff() {
        assert_eq!(socket_error_backoff(1), Some(Duration::from_millis(50)));
        assert_eq!(socket_error_backoff(19), Some(Duration::from_millis(950)));
        assert_eq!(socket_error_backoff(20), None);
        assert_eq!(socket_error_backoff(u64::MAX), None);
        assert_eq!(reopen_backoff(1), Duration::from_secs(1));
        assert_eq!(reopen_backoff(2), Duration::from_secs(2));
        assert_eq!(reopen_backoff(u64::MAX), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn reopening_retries_a_busy_endpoint_and_clears_the_visible_error() {
        let occupied = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = occupied.local_addr().unwrap();
        let health = ReceiverHealth::default();
        health.failed("OSC input receiver stopped; reopening".into());
        let (stop, mut cancelled) = oneshot::channel();
        let recovery_health = health.clone();
        let recovery =
            tokio::spawn(
                async move { reopen_socket(address, &mut cancelled, &recovery_health).await },
            );
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if health
                    .error
                    .lock()
                    .as_ref()
                    .is_some_and(|error| error.contains("cannot reopen"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!recovery.is_finished(), "a failed bind must keep retrying");
        drop(occupied);
        let socket = tokio::time::timeout(Duration::from_secs(4), recovery)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(socket.local_addr().unwrap(), address);
        assert!(health.error.lock().is_none());
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.send_to(&[1, 2, 3], address).unwrap();
        let mut packet = [0; 3];
        tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut packet))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(packet, [1, 2, 3]);
        drop(stop);
    }

    #[tokio::test]
    async fn stop_interrupts_reopen_backoff_without_clearing_the_error() {
        let health = ReceiverHealth::default();
        health.failed("OSC input unavailable".into());
        let (stop, mut cancelled) = oneshot::channel();
        stop.send(()).unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(250),
            reopen_socket("127.0.0.1:9001".parse().unwrap(), &mut cancelled, &health)
        )
        .await
        .unwrap()
        .is_none());
        assert_eq!(
            health.error.lock().as_deref(),
            Some("OSC input unavailable")
        );
    }

    #[test]
    fn unexpected_receiver_exit_is_reported_in_status() {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let (sink, _) = crate::midi_queue::bounded(32);
        let server = OscInputServer::start(
            &OscConfig {
                input_enabled: true,
                listen_port: address.port(),
                ..OscConfig::default()
            },
            sink,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(server.last_error().is_none());
        server.task.as_ref().unwrap().abort();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !server.task.as_ref().unwrap().is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(server
            .last_error()
            .unwrap()
            .contains("stopped unexpectedly"));
        server.stop();
    }

    fn note(index: u8) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: format!("/avatar/parameters/{index}"),
            args: vec![OscType::Int(1)],
        })
    }
    #[test]
    fn delayed_bundle_admission_preserves_the_scheduled_note_duration() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let (tx, rx) = crate::midi_queue::bounded(32);
        tx.set_preparation(|frame: &mut MidiFrame| {
            if frame.data[0] == 0x90 {
                // Hold the scheduler until the note-off is overdue too.
                std::thread::sleep(Duration::from_millis(120));
            }
            true
        });
        let server = OscInputServer::start(
            &OscConfig {
                input_enabled: true,
                listen_port: addr.port(),
                ..OscConfig::default()
            },
            tx,
            None,
        )
        .unwrap()
        .unwrap();
        let timetag = |time: SystemTime| {
            let unix = time.duration_since(UNIX_EPOCH).unwrap();
            (
                unix.as_secs().wrapping_add(2_208_988_800) as u32,
                ((u128::from(unix.subsec_nanos()) << 32) / 1_000_000_000) as u32,
            )
                .into()
        };
        let start = SystemTime::now() + Duration::from_millis(30);
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: timetag(start),
            content: vec![
                note(1),
                OscPacket::Bundle(OscBundle {
                    timetag: timetag(start + Duration::from_millis(60)),
                    content: vec![OscPacket::Message(OscMessage {
                        addr: "/avatar/parameters/1".into(),
                        args: vec![OscType::Int(0)],
                    })],
                }),
            ],
        });
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .send_to(&rosc::encoder::encode(&bundle).unwrap(), addr)
            .unwrap();
        let on = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let off = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        server.stop();
        assert_eq!(on.data.as_slice(), [0x90, 21, 127]);
        assert_eq!(off.data.as_slice(), [0x80, 21, 0]);
        let duration = crate::vst_worker::protocol::qpc_elapsed_us(on.ingress_qpc, off.ingress_qpc);
        assert!(
            (59_000..=61_000).contains(&duration),
            "duration: {duration} us"
        );
    }

    #[test]
    fn future_bundle_drains_all_due_batches_without_a_followup_datagram() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let config = OscConfig {
            input_enabled: true,
            listen_port: addr.port(),
            ..OscConfig::default()
        };
        let (tx, rx) = crate::midi_queue::bounded(1024);
        let server = OscInputServer::start(&config, tx, None).unwrap().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let future = (SystemTime::now() + Duration::from_millis(100))
            .duration_since(UNIX_EPOCH)
            .unwrap();
        let seconds = future.as_secs().wrapping_add(2_208_988_800) as u32;
        let fraction = ((u128::from(future.subsec_nanos()) << 32) / 1_000_000_000) as u32;
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: (seconds, fraction).into(),
            content: vec![note(1); 600],
        });
        sender
            .send_to(&rosc::encoder::encode(&bundle).unwrap(), addr)
            .unwrap();
        let deadline = Instant::now() + Duration::from_millis(600);
        let mut count = 0;
        while count < 600
            && rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_ok()
        {
            count += 1;
        }
        server.stop();
        assert_eq!(
            count, 600,
            "due OSC events must not wait for the next UDP datagram"
        );
    }
    #[test]
    fn malformed_datagram_is_counted_without_resetting_a_healthy_input() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let config = OscConfig {
            input_enabled: true,
            listen_port: addr.port(),
            ..OscConfig::default()
        };
        let (tx, rx) = crate::midi_queue::bounded(32);
        let server = OscInputServer::start(&config, tx, None).unwrap().unwrap();
        let generation = midi_reset_generation();
        let before = crate::realtime::OSC_INVALID.load(std::sync::atomic::Ordering::Relaxed);
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.send_to(&[1, 2, 3], addr).unwrap();
        sender
            .send_to(&rosc::encoder::encode(&note(3)).unwrap(), addr)
            .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap()
                .data
                .as_slice(),
            &[0x90, 23, 127]
        );
        assert_eq!(midi_reset_generation(), generation);
        assert_eq!(
            crate::realtime::OSC_INVALID.load(std::sync::atomic::Ordering::Relaxed),
            before + 1
        );
        server.stop();
    }

    #[test]
    fn udp_input_keeps_receiving_while_bundle_waits_and_releases_socket_on_stop() {
        let _guard = crate::bridge::pipeline::TEST_PIPELINE_LOCK.lock().unwrap();
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let config = OscConfig {
            input_enabled: true,
            listen_port: addr.port(),
            ..OscConfig::default()
        };
        let (tx, rx) = crate::midi_queue::bounded(32);
        let server = OscInputServer::start(&config, tx, None).unwrap().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let future = (SystemTime::now() + Duration::from_millis(250))
            .duration_since(UNIX_EPOCH)
            .unwrap();
        let seconds = future.as_secs().wrapping_add(2_208_988_800) as u32;
        let fraction = ((u128::from(future.subsec_nanos()) << 32) / 1_000_000_000) as u32;
        let bundle = OscPacket::Bundle(OscBundle {
            timetag: (seconds, fraction).into(),
            content: vec![note(1), note(2)],
        });
        sender
            .send_to(&rosc::encoder::encode(&bundle).unwrap(), addr)
            .unwrap();
        sender
            .send_to(&rosc::encoder::encode(&note(3)).unwrap(), addr)
            .unwrap();
        let immediate = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(immediate.data.as_slice(), &[0x90, 23, 127]);
        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
        for key in [21, 22] {
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .data
                    .as_slice(),
                &[0x90, key, 127]
            );
        }
        server.stop();
        let _rebound = UdpSocket::bind(addr).expect("OSC socket retained after stop");
    }
}
