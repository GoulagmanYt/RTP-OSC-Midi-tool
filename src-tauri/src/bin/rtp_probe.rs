use midi_types::MidiMessage as RtMidiMessage;
use rtpmidi::sessions::{
    events::event_handling::{MidiMessageEvent, ParticipantJoinedEvent, ParticipantLeftEvent},
    invite_responder::InviteResponder,
    rtp_midi_session::RtpMidiSession,
};
use std::{
    collections::HashSet,
    env,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct ProbeArgs {
    name: String,
    port: u16,
    duration: Duration,
    idle_timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Counters {
    total: u64,
    note_on: u64,
    note_off: u64,
    control_change: u64,
    other: u64,
}

#[derive(Default)]
struct AtomicCounters {
    total: AtomicU64,
    note_on: AtomicU64,
    note_off: AtomicU64,
    control_change: AtomicU64,
    other: AtomicU64,
}

impl AtomicCounters {
    fn snapshot(&self) -> Counters {
        Counters {
            total: self.total.load(Ordering::Relaxed),
            note_on: self.note_on.load(Ordering::Relaxed),
            note_off: self.note_off.load(Ordering::Relaxed),
            control_change: self.control_change.load(Ordering::Relaxed),
            other: self.other.load(Ordering::Relaxed),
        }
    }
}

fn parse_args() -> Result<ProbeArgs, String> {
    let mut name = String::from("OSCMidiProbe");
    let mut port: u16 = 5006;
    let mut duration = Duration::from_secs(20);
    let mut idle_timeout = Duration::from_secs(4);

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => {
                name = args
                    .next()
                    .ok_or_else(|| "Missing value for --name".to_string())?;
            }
            "--port" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "Missing value for --port".to_string())?;
                port = raw
                    .parse::<u16>()
                    .map_err(|e| format!("Invalid --port '{raw}': {e}"))?;
            }
            "--duration" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "Missing value for --duration".to_string())?;
                let secs = raw
                    .parse::<u64>()
                    .map_err(|e| format!("Invalid --duration '{raw}': {e}"))?;
                duration = Duration::from_secs(secs);
            }
            "--idle-timeout" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "Missing value for --idle-timeout".to_string())?;
                let secs = raw
                    .parse::<u64>()
                    .map_err(|e| format!("Invalid --idle-timeout '{raw}': {e}"))?;
                idle_timeout = Duration::from_secs(secs);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --bin rtp_probe -- \\\n  [--name SESSION_NAME] [--port CONTROL_PORT] [--duration SECONDS] [--idle-timeout SECONDS]"
                );
                std::process::exit(0);
            }
            other => {
                return Err(format!("Unknown argument: {other}"));
            }
        }
    }

    if port == u16::MAX {
        return Err("--port must be <= 65534".to_string());
    }

    Ok(ProbeArgs {
        name,
        port,
        duration,
        idle_timeout,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().map_err(|e| format!("argument error: {e}"))?;
    let ssrc = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;

    let session = RtpMidiSession::start(args.port, &args.name, ssrc, InviteResponder::Accept)
        .await
        .map_err(|e| format!("Failed to start RTP session on {}: {}", args.port, e))?;

    println!(
        "RTP probe listening: name='{}' ports={}/{} duration={}s idle_timeout={}s",
        args.name,
        args.port,
        args.port + 1,
        args.duration.as_secs(),
        args.idle_timeout.as_secs()
    );

    let counters = Arc::new(AtomicCounters::default());
    let participants = Arc::new(Mutex::new(HashSet::<String>::new()));
    let start = Instant::now();
    let last_msg_ms = Arc::new(AtomicU64::new(0));

    {
        let counters = counters.clone();
        let last_msg_ms = last_msg_ms.clone();
        session
            .add_listener(MidiMessageEvent, move |(message, _delta)| {
                counters.total.fetch_add(1, Ordering::Relaxed);
                match message {
                    RtMidiMessage::NoteOn(..) => counters.note_on.fetch_add(1, Ordering::Relaxed),
                    RtMidiMessage::NoteOff(..) => counters.note_off.fetch_add(1, Ordering::Relaxed),
                    RtMidiMessage::ControlChange(..) => {
                        counters.control_change.fetch_add(1, Ordering::Relaxed)
                    }
                    _ => counters.other.fetch_add(1, Ordering::Relaxed),
                };
                last_msg_ms.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
            })
            .await;
    }

    {
        let participants = participants.clone();
        session
            .add_listener(ParticipantJoinedEvent, move |participant| {
                let name = participant.name().to_str().unwrap_or("Unknown").to_string();
                let addr = participant.addr().to_string();
                if let Ok(mut guard) = participants.lock() {
                    guard.insert(addr.clone());
                }
                println!("participant_joined name='{name}' addr={addr}");
            })
            .await;
    }

    {
        session
            .add_listener(ParticipantLeftEvent, move |participant| {
                let name = participant.name().to_str().unwrap_or("Unknown");
                println!("participant_left name='{name}' addr={}", participant.addr());
            })
            .await;
    }

    let mut prev = Counters {
        total: 0,
        note_on: 0,
        note_off: 0,
        control_change: 0,
        other: 0,
    };

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let snap = counters.snapshot();
        let d_total = snap.total.saturating_sub(prev.total);
        println!(
            "tick t={}s total={} (+{}) note_on={} note_off={} cc={} other={}",
            start.elapsed().as_secs(),
            snap.total,
            d_total,
            snap.note_on,
            snap.note_off,
            snap.control_change,
            snap.other
        );
        prev = snap;

        if start.elapsed() >= args.duration {
            break;
        }

        if snap.total > 0 && args.idle_timeout.as_secs() > 0 {
            let last_ms = last_msg_ms.load(Ordering::Relaxed);
            let since_last = start
                .elapsed()
                .saturating_sub(Duration::from_millis(last_ms));
            if since_last >= args.idle_timeout {
                println!(
                    "idle timeout reached after {:?} without new RTP MIDI message",
                    since_last
                );
                break;
            }
        }
    }

    let final_counters = counters.snapshot();
    let participants_count = participants.lock().map(|guard| guard.len()).unwrap_or(0);
    session.stop_gracefully().await;

    println!(
        "FINAL total={} note_on={} note_off={} cc={} other={} participants_seen={} elapsed_ms={}",
        final_counters.total,
        final_counters.note_on,
        final_counters.note_off,
        final_counters.control_change,
        final_counters.other,
        participants_count,
        start.elapsed().as_millis()
    );

    Ok(())
}
