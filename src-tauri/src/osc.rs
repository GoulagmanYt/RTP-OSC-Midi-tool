use crate::midi::{NOTE_MAX, NOTE_MIN};
use rosc::{encoder, OscMessage, OscPacket, OscType};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

pub const PARAMETER_PATH: &str = "/avatar/parameters/";
pub const SUSTAIN_PARAM: &str = "/avatar/parameters/sustain";

static NOTE_PACKETS: LazyLock<Vec<[Box<[u8]>; 2]>> = LazyLock::new(|| {
    (1..=(NOTE_MAX - NOTE_MIN + 1))
        .map(|index| {
            [
                encode_param(&parameter_name(index), false).into_boxed_slice(),
                encode_param(&parameter_name(index), true).into_boxed_slice(),
            ]
        })
        .collect()
});

static SUSTAIN_PACKETS: LazyLock<[Box<[u8]>; 2]> = LazyLock::new(|| {
    [
        encode_param(SUSTAIN_PARAM, false).into_boxed_slice(),
        encode_param(SUSTAIN_PARAM, true).into_boxed_slice(),
    ]
});

pub fn parameter_name(index: u8) -> String {
    format!("{PARAMETER_PATH}{index}")
}

pub struct OscClient {
    socket: UdpSocket,
    target: SocketAddr,
}

impl OscClient {
    pub fn new(ip: &str, port: u16) -> Result<Self, String> {
        LazyLock::force(&NOTE_PACKETS);
        LazyLock::force(&SUSTAIN_PACKETS);
        let target = format!("{ip}:{port}")
            .parse::<SocketAddr>()
            .map_err(|e| e.to_string())?;
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_millis(25)))
            .map_err(|e| format!("socket write timeout: {e}"))?;
        Ok(Self { socket, target })
    }

    #[cfg(test)]
    fn send_param(&self, name: &str, pressed: bool) -> Result<(), String> {
        self.send_encoded(&encode_param(name, pressed))
    }

    pub fn send_note(&self, index: u8, pressed: bool) -> Result<(), String> {
        let packet = NOTE_PACKETS
            .get(usize::from(index.saturating_sub(1)))
            .ok_or_else(|| format!("OSC note index out of range: {index}"))?;
        self.send_encoded(&packet[usize::from(pressed)])
    }

    pub fn send_sustain(&self, pressed: bool) -> Result<(), String> {
        self.send_encoded(&SUSTAIN_PACKETS[usize::from(pressed)])
    }

    pub fn send_reset_all(&self) -> Result<(), String> {
        // Send K1 then set all keys to zero with a short pacing delay.
        let _ = self.send_packet(OscPacket::Message(OscMessage {
            addr: format!("{PARAMETER_PATH}K1"),
            args: vec![OscType::Float(0.0)],
        }));
        let _ = self.send_sustain(false);
        thread::sleep(Duration::from_millis(20));
        for note in NOTE_MIN..=NOTE_MAX {
            let index = (note - NOTE_MIN) + 1;
            let _ = self.send_note(index, false);
            thread::sleep(Duration::from_millis(6));
        }
        Ok(())
    }

    fn send_packet(&self, packet: OscPacket) -> Result<(), String> {
        let data = encoder::encode(&packet).map_err(|e| e.to_string())?;
        self.send_encoded(&data)
    }

    fn send_encoded(&self, data: &[u8]) -> Result<(), String> {
        let mut last_retry_error: Option<std::io::Error> = None;
        for _ in 0..3 {
            match self.socket.send_to(data, self.target) {
                Ok(_) => return Ok(()),
                Err(err)
                    if matches!(
                        err.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) =>
                {
                    last_retry_error = Some(err);
                    thread::sleep(Duration::from_millis(1));
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        Err(last_retry_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "OSC send failed after retries".to_string()))
    }
}

fn encode_param(name: &str, pressed: bool) -> Vec<u8> {
    let value = i32::from(pressed);
    encoder::encode(&OscPacket::Message(OscMessage {
        addr: name.to_owned(),
        args: vec![OscType::Int(value)],
    }))
    .expect("encoding a fixed OSC parameter packet must succeed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::ErrorKind, time::Instant};

    #[test]
    fn stress_osc_loopback_no_loss() {
        const TOTAL: usize = 30_000;

        let listener = UdpSocket::bind("127.0.0.1:0").expect("bind listener");
        listener
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("read timeout");
        let port = listener.local_addr().expect("local addr").port();

        let receiver = thread::spawn(move || {
            let mut received = 0usize;
            let mut buf = [0u8; 1536];
            let deadline = Instant::now() + Duration::from_secs(10);
            while received < TOTAL && Instant::now() < deadline {
                match listener.recv_from(&mut buf) {
                    Ok((_size, _addr)) => {
                        received += 1;
                    }
                    Err(err)
                        if err.kind() == ErrorKind::WouldBlock
                            || err.kind() == ErrorKind::TimedOut => {}
                    Err(err) => panic!("osc recv failed: {err}"),
                }
            }
            received
        });

        let client = OscClient::new("127.0.0.1", port).expect("osc client");
        let send_start = Instant::now();
        for i in 0..TOTAL {
            let pressed = i % 2 == 0;
            client
                .send_param("/avatar/parameters/1", pressed)
                .expect("osc send");
            if i % 512 == 0 {
                thread::yield_now();
            }
        }
        let send_elapsed = send_start.elapsed();

        let received = receiver.join().expect("receiver join");
        assert_eq!(
            received, TOTAL,
            "OSC stress lost packets: received {received}/{TOTAL}"
        );

        eprintln!(
            "OSC stress: {received} packets in {:?} ({:.0} pkt/s)",
            send_elapsed,
            received as f64 / send_elapsed.as_secs_f64()
        );
    }
}
