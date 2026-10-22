//! Replay an explicit, timestamped MIDI scenario through the production worker.
//! Usage: audio_scenario <scenario.json> <new-report.json>
//! Requires explicit worker paths and an isolated, absolute VST state directory.
//! Observations are heartbeat snapshots, not per-callback audio captures.
use osc_midi_bridge::{
    audio::AudioSettings,
    vst_worker::{protocol::monotonic_qpc, VstWorkerState, VstWorkerSupervisor},
};
use serde_json::{json, Value};
use std::{fs::OpenOptions, io::Write, path::Path, time::Duration};

fn number(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("Missing integer {key}"))
}

fn parse_events(value: &Value, duration: u64) -> Result<Vec<(u64, Vec<u8>)>, String> {
    let rows = value.as_array().ok_or("events must be an array")?;
    if rows.is_empty() || rows.len() > 100_000 {
        return Err("events must contain 1..100000 events".into());
    }
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let at = number(row, "atUs")?;
        if at >= duration || events.last().is_some_and(|(previous, _)| *previous > at) {
            return Err("Events must be sorted and precede durationUs".into());
        }
        let bytes: Vec<u8> = row["data"]
            .as_array()
            .ok_or("Missing data")?
            .iter()
            .map(|byte| {
                byte.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or("Invalid MIDI byte")
            })
            .collect::<Result<_, _>>()?;
        if bytes.len() != 3
            || !matches!(bytes[0] & 0xf0, 0x80 | 0x90 | 0xb0)
            || bytes[1..].iter().any(|b| *b > 127)
        {
            return Err("Use three-byte note or CC messages with 7-bit data".into());
        }
        events.push((at, bytes));
    }
    Ok(events)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: audio_scenario <scenario.json> <new-report.json>".into());
    }
    let scenario: Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let duration = number(&scenario, "durationUs")?;
    if duration == 0 || duration > 43_200_000_000 {
        return Err("durationUs must be 1..43200000000".into());
    }
    let events = parse_events(&scenario["events"], duration)?;
    let settings: AudioSettings = serde_json::from_value(scenario["settings"].clone())?;
    let plugin = settings
        .vst_path
        .as_deref()
        .ok_or("Explicit vstPath required")?;
    if !Path::new(plugin).is_absolute() || !Path::new(plugin).exists() {
        return Err("vstPath must be an existing absolute plugin path".into());
    }
    for name in [
        "OSCMIDI_VST_WORKER_X64_PATH",
        "OSCMIDI_VST_WORKER_X86_PATH",
        "OSCMIDI_VST_STATE_DIR",
    ] {
        let path = std::env::var(name).map_err(|_| format!("Set {name} explicitly"))?;
        if !Path::new(&path).is_absolute() || !Path::new(&path).exists() {
            return Err(format!("{name} must be an existing absolute path").into());
        }
    }
    // Reserve evidence paths before starting audio; never overwrite prior runs.
    let mut report = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    let mut observations = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(format!("{}.jsonl", args[1]))?;
    let supervisor = VstWorkerSupervisor::new();
    let status = match supervisor.start(settings.clone(), None).await {
        Ok(status) => status,
        Err(error) => {
            let _ = supervisor.stop().await;
            serde_json::to_writer_pretty(
                &mut report,
                &json!({"scenario":scenario,"loadError":error}),
            )?;
            return Err(error.into());
        }
    };
    if scenario["openEditorBeforeStart"].as_bool().unwrap_or(false) {
        supervisor.set_game_mode(false).await?;
        supervisor.open_editor().await?;
        supervisor.set_game_mode(settings.game_mode).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let start = tokio::time::Instant::now();
    let start_qpc = monotonic_qpc();
    let mut next = 0;
    let mut submitted = Vec::with_capacity(events.len());
    let mut failed = false;
    let mut observations_count = 0u64;
    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while start.elapsed().as_micros() < u128::from(duration) {
        // An absolute deadline prevents cumulative drift and catch-up bursts.
        let deadline = events
            .get(next)
            .map_or(start + Duration::from_micros(duration), |(at, _)| {
                start + Duration::from_micros(*at)
            });
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                while let Some((at, data)) = events.get(next) {
                    if u128::from(*at) > start.elapsed().as_micros() { break; }
                    let qpc = monotonic_qpc();
                    let accepted = supervisor.try_send_midi(data);
                    failed |= !accepted;
                    submitted.push(json!({"index":next,"dueUs":at,"elapsedUs":start.elapsed().as_micros(),"qpc":qpc,"accepted":accepted,"data":data}));
                    next += 1;
                }
            }
            _ = ticker.tick() => {
                let snapshot = supervisor.snapshot();
                let row = json!({"elapsedUs":start.elapsed().as_micros(),"qpc":monotonic_qpc(),"state":snapshot.state,"restarts":snapshot.restarts,"midiDrops":snapshot.midi_drops,"metrics":snapshot.metrics});
                serde_json::to_writer(&mut observations, &row)?;
                writeln!(observations)?;
                observations_count += 1;
                if snapshot.state != VstWorkerState::Ready { failed = true; break; }
            }
        }
    }
    failed |= next != events.len();
    let before_reset = supervisor.snapshot();
    // Explicit cleanup occurs after the measured scenario, never inside it.
    supervisor.request_emergency_reset();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stop = supervisor.stop().await;
    let result = json!({"scenario":scenario,"status":status,"startQpc":start_qpc,
        "submitted":submitted,"observations":observations_count,"metricsBeforeCleanup":before_reset.metrics,
        "workerState":before_reset.state,"workerRestarts":before_reset.restarts,
        "workerExit":before_reset.last_exit,"stopError":stop.as_ref().err(),"submissionFailed":failed,
        "scope":"Worker IPC MIDI injection. Heartbeat snapshots can miss individual audio incidents. No DAC or audibility assertion."});
    serde_json::to_writer_pretty(&mut report, &result)?;
    println!(
        "Report: {} ({} events, {} snapshots)",
        args[1], next, observations_count
    );
    if failed || stop.is_err() {
        return Err("Scenario failed; inspect report".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_or_out_of_order_midi_before_starting_audio() {
        for rows in [
            json!([]),
            json!([{"atUs":0,"data":[144,60,256]}]),
            json!([{"atUs":0,"data":[144,128,80]}]),
            json!([{"atUs":2,"data":[144,60,80]},{"atUs":1,"data":[128,60,0]}]),
            json!([{"atUs":10,"data":[144,60,80]}]),
        ] {
            assert!(parse_events(&rows, 10).is_err());
        }
    }
    #[test]
    fn preserves_equal_deadline_chords_and_sustain() {
        let rows = json!([{"atUs":0,"data":[144,60,80]},{"atUs":0,"data":[144,64,80]},
            {"atUs":1,"data":[176,64,127]},{"atUs":2,"data":[128,60,0]}]);
        assert_eq!(
            parse_events(&rows, 3).unwrap(),
            vec![
                (0, vec![144, 60, 80]),
                (0, vec![144, 64, 80]),
                (1, vec![176, 64, 127]),
                (2, vec![128, 60, 0])
            ]
        );
    }
}
