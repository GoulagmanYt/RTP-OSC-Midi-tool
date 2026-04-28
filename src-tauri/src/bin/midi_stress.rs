use std::{
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time::sleep;

fn print_usage() {
    println!("Usage: midi_stress <mode> [options]");
    println!("Modes:");
    println!("  rtp    [--target 127.0.0.1:5004] [--rate 5000] [--duration 10]");
    println!("  local  [--port-name OSCMidi] [--rate 50000] [--duration 10]");
    println!("\nExamples:");
    println!("  cargo run --release --bin midi_stress -- local --rate 100000");
    println!("  cargo run --release --bin midi_stress -- rtp --target 127.0.0.1:5004 --rate 5000");
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let mode = args[1].as_str();
    let mut rate = 50_000;
    let mut duration = 10;
    let mut target = "127.0.0.1:5004".to_string();
    let mut port_name = "OSCMidi".to_string();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--rate" => rate = args[i + 1].parse().unwrap_or(rate),
            "--duration" => duration = args[i + 1].parse().unwrap_or(duration),
            "--target" => target = args[i + 1].clone(),
            "--port-name" => port_name = args[i + 1].clone(),
            _ => {}
        }
        i += 2;
    }

    match mode {
        "rtp" => rtp_mode(&target, rate, duration).await?,
        "local" => local_mode(&port_name, rate, duration).await?,
        _ => print_usage(),
    }

    Ok(())
}

async fn rtp_mode(
    target: &str,
    rate: usize,
    duration: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use midi_types::MidiMessage;
    use rtpmidi::sessions::{invite_responder::InviteResponder, rtp_midi_session::RtpMidiSession};

    println!("Starting RTP stress test...");
    println!("Target: {}", target);
    println!("Rate: {} msgs/sec", rate);
    println!("Duration: {} seconds", duration);

    let ssrc = 0x87654321;
    let session =
        RtpMidiSession::start(0, "MidiStressClient", ssrc, InviteResponder::Accept).await?;
    let target_addr: std::net::SocketAddr = target.parse()?;

    println!("Inviting {}...", target_addr);
    session.invite_participant(target_addr).await;

    // Attendre la connexion
    sleep(Duration::from_secs(2)).await;

    let start = Instant::now();
    let duration_d = Duration::from_secs(duration);
    let interval = Duration::from_nanos(1_000_000_000 / rate.max(1) as u64);

    let sent = Arc::new(AtomicUsize::new(0));
    let mut next_tick = start;

    let sent_clone = sent.clone();
    let monitor = tokio::spawn(async move {
        let mut prev = 0;
        loop {
            sleep(Duration::from_secs(1)).await;
            let current = sent_clone.load(Ordering::Relaxed);
            println!("Sent {} RTP msgs in the last second", current - prev);
            prev = current;
        }
    });

    while start.elapsed() < duration_d {
        // Envoi NoteOn/NoteOff
        let msg = rtpmidi::packets::midi_packets::rtp_midi_message::RtpMidiMessage::MidiMessage(
            MidiMessage::NoteOn(
                midi_types::Channel::C1,
                midi_types::Note::C3,
                midi_types::Value7::new(100),
            ),
        );
        let _ = session.send_midi(&msg).await;
        sent.fetch_add(1, Ordering::Relaxed);

        next_tick += interval;
        let now = Instant::now();
        if now < next_tick {
            sleep(next_tick - now).await;
        }
    }

    monitor.abort();
    let total = sent.load(Ordering::Relaxed);
    println!(
        "FINAL: Injected {} RTP messages in {}s (Avg: {}/s)",
        total,
        duration,
        total as f64 / duration as f64
    );

    Ok(())
}

async fn local_mode(
    port_name: &str,
    rate: usize,
    duration: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    use midir::MidiOutput;

    println!("Starting LOCAL (Virtual Port) stress test...");
    println!("Target Port: {}", port_name);
    println!("Rate: {} msgs/sec", rate);
    println!("Duration: {} seconds", duration);

    let midi_out = MidiOutput::new("MidiStressClient")?;
    let out_ports = midi_out.ports();
    let port = out_ports.into_iter().find(|p| {
        if let Ok(name) = midi_out.port_name(p) {
            name.contains(port_name)
        } else {
            false
        }
    });

    let Some(port) = port else {
        println!(
            "Port '{}' non trouvé. Assurez-vous qu'OSCMidi est lancé.",
            port_name
        );
        return Ok(());
    };

    let mut conn_out = midi_out
        .connect(&port, "midi-stress-out")
        .map_err(|e| e.to_string())?;

    let start = Instant::now();
    let duration_d = Duration::from_secs(duration);
    let interval = Duration::from_nanos(1_000_000_000 / rate.max(1) as u64);

    let mut next_tick = start;
    let mut sent = 0;
    let mut prev_sent = 0;
    let mut last_print = Instant::now();

    while start.elapsed() < duration_d {
        // Envoi NoteOn
        let _ = conn_out.send(&[0x90, 60, 100]);
        sent += 1;

        next_tick += interval;
        let now = Instant::now();
        if now < next_tick {
            // Busy wait for nanosecond precision on high rates
            while Instant::now() < next_tick {}
        }

        if last_print.elapsed() >= Duration::from_secs(1) {
            println!("Sent {} LOCAL msgs in the last second", sent - prev_sent);
            prev_sent = sent;
            last_print = Instant::now();
        }
    }

    println!(
        "FINAL: Injected {} LOCAL messages in {}s (Avg: {}/s)",
        sent,
        duration,
        sent as f64 / duration as f64
    );

    Ok(())
}
