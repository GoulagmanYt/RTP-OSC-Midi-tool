//! Optional OSC input with borrowed decoding and a bounded deadline queue.
mod packet;
use crate::{
    bridge::pipeline::{
        midi_reset_generation, request_critical_midi_reset, try_enqueue_midi_frame, EnqueueOutcome,
    },
    config::OscConfig,
    midi::MidiFrame,
};
use crossbeam_channel::Sender;
use packet::{ScheduledMidi, MAX_PACKET_MESSAGES};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::Arc,
    time::{Instant, SystemTime},
};
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::oneshot;
const PENDING_CAPACITY: usize = 4096;

pub(crate) struct OscInputServer {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}
impl OscInputServer {
    pub(crate) fn start(
        config: &OscConfig,
        sink: Sender<MidiFrame>,
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
        let socket =
            UdpSocket::bind(SocketAddr::new(ip, config.listen_port)).map_err(|e| e.to_string())?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let source: Arc<str> =
            Arc::from(format!("OSC:{}:{}", config.listen_ip, config.listen_port));
        let (stop, cancelled) = oneshot::channel();
        let task = async_runtime::spawn(receive(socket, sink, source, cancelled));
        Ok(Some(Self {
            stop: Some(stop),
            task: Some(task),
        }))
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

async fn receive(
    socket: UdpSocket,
    sink: Sender<MidiFrame>,
    source: Arc<str>,
    mut stop: oneshot::Receiver<()>,
) {
    let Ok(socket) = tokio::net::UdpSocket::from_std(socket) else {
        request_critical_midi_reset();
        return;
    };
    let mut buffer = vec![0; 65_536];
    let mut staging = Vec::with_capacity(MAX_PACKET_MESSAGES);
    let mut pending: BinaryHeap<Reverse<ScheduledMidi>> =
        BinaryHeap::with_capacity(PENDING_CAPACITY);
    let mut order = 0u64;
    let mut generation = midi_reset_generation();
    loop {
        let current = midi_reset_generation();
        if generation != current {
            pending.clear();
            generation = current;
        }
        let due = pending.peek().map(|event| event.0.deadline);
        tokio::select! {
            biased;
            _ = &mut stop => break,
            _ = async { if let Some(due) = due { tokio::time::sleep_until(due.into()).await; } }, if due.is_some() => {
                for _ in 0..256 {
                    if pending.peek().is_none_or(|event| event.0.deadline > Instant::now()) { break; }
                    let event = pending.pop().unwrap().0;
                    if event.generation != midi_reset_generation() { continue; }
                    let frame = MidiFrame { data: smallvec::SmallVec::from_slice(&event.data[..usize::from(event.len)]), source: source.clone() };
                    if !matches!(try_enqueue_midi_frame(&sink, frame), Ok(EnqueueOutcome::Enqueued)) {
                        request_critical_midi_reset(); pending.clear(); break;
                    }
                }
            },
            received = socket.recv_from(&mut buffer) => {
                let Ok((size, _)) = received else { request_critical_midi_reset(); break; };
                let now = Instant::now();
                if packet::decode(&buffer[..size], SystemTime::now(), now, generation, &mut staging).is_err()
                    || staging.len() > PENDING_CAPACITY - pending.len() {
                    request_critical_midi_reset(); pending.clear(); continue;
                }
                for mut event in staging.drain(..) {
                    event.order = order; order = order.wrapping_add(1);
                    pending.push(Reverse(event));
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
    fn note(index: u8) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: format!("/avatar/parameters/{index}"),
            args: vec![OscType::Int(1)],
        })
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
        let (tx, rx) = crossbeam_channel::bounded(32);
        let server = OscInputServer::start(&config, tx).unwrap().unwrap();
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
