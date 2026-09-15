//! Real UDP session churn, bidirectional notes and socket ownership endurance.
use midi_types::{Channel, MidiMessage};
use rtpmidi::{
    packets::midi_packets::midi_event::MidiEvent,
    sessions::{
        events::event_handling::{MidiMessageEvent, ParticipantJoinedEvent, ParticipantLeftEvent},
        invite_responder::InviteResponder,
        rtp_midi_session::RtpMidiSession,
    },
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut duration = 3600u64;
    let mut cycles_limit = u64::MAX;
    let mut report = PathBuf::from("rtp-reliability.json");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing argument value")?;
        match flag.as_str() {
            "--duration-seconds" => duration = value.parse()?,
            "--cycles" => cycles_limit = value.parse()?,
            "--report" => report = value.into(),
            _ => return Err(format!("Unknown argument {flag}").into()),
        }
    }
    if duration == 0 || duration > 43200 || cycles_limit == 0 {
        return Err("Invalid duration or cycle limit".into());
    }
    let started = Instant::now();
    let server = RtpMidiSession::start(0, "Endurance receiver", 1, InviteResponder::Accept).await?;
    let mut destination = server.local_addr()?;
    destination.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let overflows = Arc::new(AtomicU64::new(0));
    let rejected = overflows.clone();
    server
        .add_listener(MidiMessageEvent, move |(message, _)| {
            if tx.try_send(message).is_err() {
                rejected.fetch_add(1, Ordering::Relaxed);
            }
        })
        .await;
    let (left_tx, mut left_rx) = tokio::sync::mpsc::channel(8);
    server
        .add_listener(ParticipantLeftEvent, move |_| {
            let _ = left_tx.try_send(());
        })
        .await;
    let mut cycles = 0u64;
    let mut matched_notes = 0u64;
    let mut released_socket_pairs = 0u64;
    let mut max_cycle_us = 0u128;
    let mut cycle_samples = Vec::new();
    let mut batch_round_trip_us = Vec::new();
    let result: Result<(), String> = async {
        while started.elapsed() < Duration::from_secs(duration) && cycles < cycles_limit {
            let cycle_started = Instant::now();
            let peer = RtpMidiSession::start(0, "Reconnecting source", 2, InviteResponder::Accept).await.map_err(|error| error.to_string())?;
            let address = peer.local_addr().map_err(|error| error.to_string())?;
            let (joined_tx, mut joined_rx) = tokio::sync::mpsc::channel(1);
            peer.add_listener(ParticipantJoinedEvent, move |_| { let _ = joined_tx.try_send(()); }).await;
            let (echo_tx, mut echo_rx) = tokio::sync::mpsc::channel(128);
            let rejected = overflows.clone();
            peer.add_listener(MidiMessageEvent, move |(message, _)| {
                if echo_tx.try_send(message).is_err() { rejected.fetch_add(1, Ordering::Relaxed); }
            }).await;
            let cycle_result: Result<(), String> = async {
                peer.invite_participant(destination).await;
                tokio::time::timeout(Duration::from_secs(3), joined_rx.recv()).await.map_err(|_| "invitation timed out")?.ok_or("invitation channel closed")?;
                for _ in 0..10 {
                    for on in [true, false] {
                        let messages: Vec<_> = (48u8..64).map(|note| if on {
                            MidiMessage::NoteOn(Channel::C1, note.into(), 100.into())
                        } else { MidiMessage::NoteOff(Channel::C1, note.into(), 64.into()) }).collect();
                        let batch: Vec<_> = messages.iter().map(|message| MidiEvent::new(None, (*message).into())).collect();
                        let transmitted_at = Instant::now();
                        peer.send_midi_batch(&batch).await.map_err(|error| error.to_string())?;
                        server.send_midi_batch(&batch).await.map_err(|error| error.to_string())?;
                        for expected in messages {
                            for received in [
                                tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.map_err(|_| "forward MIDI timeout")?,
                                tokio::time::timeout(Duration::from_secs(2), echo_rx.recv()).await.map_err(|_| "reverse MIDI timeout")?,
                            ] {
                                if received != Some(expected) { return Err(format!("MIDI mismatch: {received:?}, expected {expected:?}")); }
                                if !on { matched_notes += 1; }
                            }
                        }
                        batch_round_trip_us.push(transmitted_at.elapsed().as_micros() as u64);
                    }
                }
                Ok(())
            }.await;
            peer.stop_gracefully().await;
            drop(peer);
            cycle_result?;
            tokio::time::timeout(Duration::from_secs(2), left_rx.recv()).await.map_err(|_| "BY timeout")?.ok_or("BY channel closed")?;
            if !server.participants().await.is_empty() { return Err("peer survived BY".into()); }
            let control = std::net::UdpSocket::bind(address).map_err(|error| format!("control socket leaked: {error}"))?;
            let midi = std::net::UdpSocket::bind((address.ip(), address.port() + 1)).map_err(|error| format!("MIDI socket leaked: {error}"))?;
            drop((control, midi));
            released_socket_pairs += 1;
            cycles += 1;
            max_cycle_us = max_cycle_us.max(cycle_started.elapsed().as_micros());
            if cycles.is_multiple_of(60) {
                println!("Cycles {cycles}, matched notes {matched_notes}, callback overflows {}", overflows.load(Ordering::Relaxed));
                cycle_samples.push(serde_json::json!({"elapsedMs": started.elapsed().as_millis(), "cycles": cycles, "matchedNotes": matched_notes}));
            }
            // Keep the long run representative of repeated user reconnections,
            // while every connected interval sends a burst in both directions.
            tokio::time::sleep(Duration::from_secs(1).saturating_sub(cycle_started.elapsed())).await;
        }
        Ok(())
    }.await;
    server.stop_gracefully().await;
    drop(server);
    batch_round_trip_us.sort_unstable();
    let percentile = |percent: usize| {
        batch_round_trip_us
            .get(batch_round_trip_us.len().saturating_sub(1) * percent / 100)
            .copied()
    };
    let overflow_count = overflows.load(Ordering::Relaxed);
    let passed = result.is_ok()
        && overflow_count == 0
        && cycles > 0
        && matched_notes == cycles * 320
        && cycles == released_socket_pairs;
    std::fs::write(
        &report,
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed": passed, "error": result.err(), "elapsedMs": started.elapsed().as_millis(),
            "requestedDurationSeconds": duration, "requestedCycles": cycles_limit,
            "cycles": cycles, "matchedNoteOnsAndOffs": matched_notes,
            "releasedSocketPairs": released_socket_pairs, "callbackOverflows": overflow_count,
            "batchBidirectionalLatencyUs": { "p50": percentile(50), "p95": percentile(95), "p99": percentile(99), "max": batch_round_trip_us.last(), "samples": batch_round_trip_us.len() },
            "maxCycleUs": max_cycle_us, "observations": cycle_samples,
            "scope": "Loopback UDP, real AppleMIDI invitations/BY, repeated same-SSRC connections, bidirectional note matching and socket rebinding. No physical MIDI driver."
        }))?,
    )?;
    if !passed {
        return Err("RTP endurance failed; inspect report".into());
    }
    Ok(())
}
