//! High-throughput MIDI and RTP-MIDI load generator for developer diagnostics.

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
    println!(
        "  cargo run --release --manifest-path tools/diagnostics/Cargo.toml --target-dir tools/diagnostics/target --bin midi_stress -- local --rate 100000"
    );
    println!(
        "  cargo run --release --manifest-path tools/diagnostics/Cargo.toml --target-dir tools/diagnostics/target --bin midi_stress -- rtp --target 127.0.0.1:5004 --rate 5000"
    );
}

fn next_val(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    if *i + 1 < args.len() {
        *i += 1;
        Ok(args[*i].clone())
    } else {
        Err(format!("Missing value for {flag}"))
    }
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
            "--rate" => {
                rate = next_val(&args, &mut i, "--rate")?.parse()?;
            }
            "--duration" => {
                duration = next_val(&args, &mut i, "--duration")?.parse()?;
            }
            "--target" => {
                target = next_val(&args, &mut i, "--target")?;
            }
            "--port-name" => {
                port_name = next_val(&args, &mut i, "--port-name")?;
            }
            unknown => {
                return Err(format!("Unknown argument: {unknown}").into());
            }
        }
        i += 1;
    }

    if rate == 0 || duration == 0 {
        return Err("Rate and duration must be positive".into());
    }

    match mode {
        "rtp" => rtp_mode(&target, rate, duration).await?,
        "local" => local_mode(&port_name, rate, duration).await?,
        _ => return Err(format!("Unknown mode: {mode}").into()),
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
    let errors = Arc::new(AtomicUsize::new(0));
    let mut next_tick = start;
    let mut toggle = false;

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
        // Envoi NoteOn/NoteOff alterné
        let midi_msg = if toggle {
            MidiMessage::NoteOff(
                midi_types::Channel::C1,
                midi_types::Note::C3,
                midi_types::Value7::new(0),
            )
        } else {
            MidiMessage::NoteOn(
                midi_types::Channel::C1,
                midi_types::Note::C3,
                midi_types::Value7::new(100),
            )
        };
        toggle = !toggle;

        let msg =
            rtpmidi::packets::midi_packets::rtp_midi_message::RtpMidiMessage::MidiMessage(midi_msg);
        match session.send_midi(&msg).await {
            Ok(_) => {
                sent.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                errors.fetch_add(1, Ordering::Relaxed);
            }
        }

        next_tick += interval;
        let now = Instant::now();
        if now < next_tick {
            sleep(next_tick - now).await;
        }
    }

    // NoteOff final pour laisser la cible dans un état propre
    let final_note_off =
        rtpmidi::packets::midi_packets::rtp_midi_message::RtpMidiMessage::MidiMessage(
            MidiMessage::NoteOff(
                midi_types::Channel::C1,
                midi_types::Note::C3,
                midi_types::Value7::new(0),
            ),
        );
    let release_result = session.send_midi(&final_note_off).await;
    session.stop_gracefully().await;
    release_result?;

    monitor.abort();
    let total = sent.load(Ordering::Relaxed);
    let total_errors = errors.load(Ordering::Relaxed);
    println!(
        "FINAL: Injected {} RTP messages in {}s (Avg: {}/s, Errors: {})",
        total,
        duration,
        total as f64 / duration as f64,
        total_errors
    );

    if total_errors > 0 {
        return Err(format!("{total_errors} RTP sends failed").into());
    }
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
        return Err(
            format!("Port '{port_name}' non trouvé. Assurez-vous qu'OSCMidi est lancé.").into(),
        );
    };

    let mut conn_out = midi_out
        .connect(&port, "midi-stress-out")
        .map_err(|e| e.to_string())?;

    let start = Instant::now();
    let duration_d = Duration::from_secs(duration);
    let interval = Duration::from_nanos(1_000_000_000 / rate.max(1) as u64);

    let mut next_tick = start;
    let mut prev_sent = 0;
    let mut last_print = Instant::now();

    let sent = send_local_stream(
        |bytes| conn_out.send(bytes),
        |sent| {
            if sent > 0 {
                next_tick += interval;
            }
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
            start.elapsed() < duration_d
        },
    )?;

    println!(
        "FINAL: Injected {} LOCAL messages in {}s (Avg: {}/s)",
        sent,
        duration,
        sent as f64 / duration as f64
    );

    Ok(())
}

fn send_local_stream<E>(
    mut send: impl FnMut(&[u8]) -> Result<(), E>,
    mut keep_running: impl FnMut(usize) -> bool,
) -> Result<usize, E> {
    let mut sent = 0;
    while keep_running(sent) {
        let bytes = if sent % 2 == 0 {
            [0x90, 60, 100]
        } else {
            [0x80, 60, 0]
        };
        if let Err(error) = send(&bytes) {
            // The failed call may have reached the driver. Attempt recovery,
            // but preserve the original error and never count a failed send.
            let _ = send(&[0x80, 60, 0]);
            return Err(error);
        }
        sent += 1;
    }
    send(&[0x80, 60, 0])?;
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_stream_alternates_and_releases_after_odd_or_even_message_counts() {
        for count in [1, 2, 3, 4] {
            let mut messages = Vec::new();
            let sent = send_local_stream(
                |bytes| {
                    messages.push(bytes.to_vec());
                    Ok::<_, ()>(())
                },
                |sent| sent < count,
            )
            .unwrap();
            assert_eq!(sent, count);
            assert_eq!(messages.len(), count + 1);
            for (index, bytes) in messages[..count].iter().enumerate() {
                assert_eq!(
                    bytes.as_slice(),
                    if index % 2 == 0 {
                        &[0x90, 60, 100]
                    } else {
                        &[0x80, 60, 0]
                    }
                );
            }
            assert_eq!(messages.last().unwrap(), &[0x80, 60, 0]);
        }
    }

    #[test]
    fn local_stream_reports_send_failure_and_attempts_final_release() {
        let mut messages = Vec::new();
        let result = send_local_stream(
            |bytes| {
                messages.push(bytes.to_vec());
                if messages.len() == 2 {
                    Err("driver disconnected")
                } else {
                    Ok(())
                }
            },
            |sent| sent < 3,
        );
        assert_eq!(result, Err("driver disconnected"));
        assert_eq!(messages.last().unwrap(), &[0x80, 60, 0]);
    }

    #[test]
    fn local_stream_reports_final_release_failure() {
        assert_eq!(
            send_local_stream(|_| Err("release failed"), |_| false),
            Err("release failed")
        );
    }

    #[tokio::test]
    async fn missing_local_port_is_an_error() {
        assert!(local_mode("__OSCMidi_missing_regression_port__", 1, 1)
            .await
            .is_err());
    }
    #[test]
    fn missing_option_values_return_errors_instead_of_panicking() {
        for flag in ["--rate", "--duration", "--target", "--port-name"] {
            let args = vec!["midi_stress".into(), "rtp".into(), flag.into()];
            let mut index = 2;
            assert!(next_val(&args, &mut index, flag).is_err());
        }
    }
}
