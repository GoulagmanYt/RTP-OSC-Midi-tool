//! Exercise the production isolated worker against a selected installed ASIO/VST rig.
use osc_midi_bridge::{
    audio::AudioSettings,
    config::ConfigStore,
    vst_worker::{VstWorkerState, VstWorkerSupervisor},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut buffer = None;
    let mut notes = 10_000usize;
    let mut duration_seconds = 0u64;
    let mut active_seconds: Option<u64> = None;
    let mut cycle_seconds: Option<u64> = None;
    let mut report = PathBuf::from("audio-reliability.json");
    let mut stop_file: Option<PathBuf> = None;
    let mut plugin = None;
    let mut device = String::from("Voicemeeter AUX Virtual ASIO");
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {flag}"))?;
        match flag.as_str() {
            "--buffer" => buffer = Some(value.parse::<u32>()?),
            "--notes" => notes = value.parse()?,
            "--duration-seconds" => duration_seconds = value.parse()?,
            "--active-seconds" => active_seconds = Some(value.parse()?),
            "--cycle-seconds" => cycle_seconds = Some(value.parse()?),
            "--report" => report = value.into(),
            "--vst" => plugin = Some(value),
            "--device" => device = value,
            "--stop-file" => stop_file = Some(value.into()),
            _ => return Err(format!("Unknown option {flag}").into()),
        }
    }
    if notes == 0 || notes > 100_000 {
        return Err("--notes must be 1..100000".into());
    }
    if duration_seconds > 43_200 {
        return Err("--duration-seconds must be 0..43200".into());
    }
    if active_seconds.is_some_and(|seconds| duration_seconds == 0 || seconds > duration_seconds) {
        return Err(
            "--active-seconds requires duration mode and must not exceed its duration".into(),
        );
    }
    if cycle_seconds.is_some_and(|cycle| {
        cycle == 0 || cycle > duration_seconds || active_seconds.is_none_or(|active| active > cycle)
    }) {
        return Err("--cycle-seconds requires --active-seconds <= cycle <= duration".into());
    }
    let config = ConfigStore::new().load();
    let settings = AudioSettings {
        backend: Some("ASIO".into()),
        device_id: Some(format!("asio:{device}")),
        device: Some(device),
        sample_rate: 48_000,
        buffer_size: buffer.unwrap_or(config.audio.buffer_size),
        vst_plugin_id: if plugin.is_none() {
            config.audio.vst_plugin_id.clone()
        } else {
            None
        },
        vst_path: plugin.or(config.audio.vst_path.clone()),
        gain_db: -24.0,
        limiter_enabled: true,
        ..AudioSettings::default()
    };
    let supervisor = VstWorkerSupervisor::new();
    let started = Instant::now();
    println!(
        "Loading {:?} with {:?}...",
        settings.vst_path, settings.device
    );
    let status = match supervisor.start(settings, None).await {
        Ok(status) => status,
        Err(error) => {
            let _ = supervisor.stop().await;
            std::fs::write(
                &report,
                serde_json::to_vec_pretty(&serde_json::json!({ "loadError": error }))?,
            )?;
            return Err(error.into());
        }
    };
    println!(
        "Ready: {} Hz, {} frames, {:.3} ms block period",
        status.sample_rate,
        status.stream_buffer_size,
        f64::from(status.stream_buffer_size) * 1000.0 / f64::from(status.sample_rate)
    );
    let mut rejected = 0;
    let mut sent_on = 0;
    let mut sent_off = 0;
    let mut peak = 0.0f32;
    let mut max_midi_age_us = 0;
    let mut observations = Vec::new();
    let mut next_observation = Instant::now();
    let mut failure = None;
    let mut max_xruns = 0;
    let mut max_over_budget = 0;
    let mut max_drops = 0;
    let transmitting_at = Instant::now();
    while if duration_seconds == 0 {
        sent_on < notes
    } else {
        transmitting_at.elapsed() < Duration::from_secs(duration_seconds)
    } {
        if stop_file.as_ref().is_some_and(|path| path.exists()) {
            failure = Some("external stop requested");
            break;
        }
        if supervisor.snapshot().state != VstWorkerState::Ready {
            failure = Some("worker left ready state");
            break;
        }
        let phase_seconds = cycle_seconds.map_or_else(
            || transmitting_at.elapsed().as_secs(),
            |cycle| transmitting_at.elapsed().as_secs() % cycle,
        );
        let count = if active_seconds.is_some_and(|seconds| phase_seconds >= seconds) {
            0
        } else if duration_seconds == 0 {
            (notes - sent_on).min(16)
        } else {
            16
        };
        for index in 0..count {
            if !supervisor.try_send_midi(&[0x90, 48 + index as u8, 80]) {
                rejected += 1;
            }
            sent_on += 1;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        for index in 0..count {
            if !supervisor.try_send_midi(&[0x80, 48 + index as u8, 0]) {
                rejected += 1;
            }
            sent_off += 1;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        let snapshot = supervisor.snapshot();
        max_xruns = max_xruns.max(snapshot.metrics.audio_xruns);
        max_over_budget = max_over_budget.max(snapshot.metrics.callback_over_budget_count);
        max_drops = max_drops.max(snapshot.metrics.audio_midi_drops);
        peak = peak
            .max(snapshot.metrics.audio_peak_l)
            .max(snapshot.metrics.audio_peak_r);
        max_midi_age_us = max_midi_age_us.max(snapshot.metrics.midi_oldest_us);
        if Instant::now() >= next_observation {
            println!(
                "Notes {sent_on}, xruns={}, drops={}, DSP p99={} us",
                snapshot.metrics.audio_xruns,
                snapshot.metrics.audio_midi_drops,
                snapshot.metrics.dsp_process_p99_us
            );
            observations.push(serde_json::json!({"elapsedMs": started.elapsed().as_millis(), "metrics": snapshot.metrics}));
            next_observation = Instant::now() + Duration::from_secs(1);
        }
    }
    let transmission_ms = transmitting_at.elapsed().as_millis();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let before_panic = supervisor.snapshot();
    supervisor.request_emergency_reset();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let final_snapshot = supervisor.snapshot();
    max_xruns = max_xruns.max(final_snapshot.metrics.audio_xruns);
    max_over_budget = max_over_budget.max(final_snapshot.metrics.callback_over_budget_count);
    max_drops = max_drops.max(final_snapshot.metrics.audio_midi_drops);
    let delivery_passed = failure.is_none()
        && (duration_seconds != 0 || sent_on == notes)
        && sent_off == sent_on
        && before_panic.metrics.midi_queue_depth == 0
        && rejected == 0
        && max_drops == 0
        && final_snapshot.state == VstWorkerState::Ready
        && final_snapshot.restarts == 0
        && final_snapshot.midi_drops == 0
        && final_snapshot.metrics.midi_queue_depth == 0;
    let stopped_at = Instant::now();
    let stop = supervisor.stop().await;
    let stop_ms = stopped_at.elapsed().as_millis();
    let result = serde_json::json!({
        "status": status, "requestedNotes": notes, "requestedDurationSeconds": duration_seconds,
        "transmissionMs": transmission_ms, "requestedActiveSeconds": active_seconds, "requestedCycleSeconds": cycle_seconds, "metricsBeforeFinalPanic": before_panic.metrics, "noteOnsSubmitted": sent_on,
        "noteOffsSubmitted": sent_off, "rejectedSubmissions": rejected,
        "peakObserved": peak, "maxObservedMidiAgeUs": max_midi_age_us,
        "finalMetrics": final_snapshot.metrics, "workerStateBeforeStop": final_snapshot.state,
        "workerRestarts": final_snapshot.restarts, "workerExit": final_snapshot.last_exit,
        "failure": failure, "stopError": stop.as_ref().err(), "stopMs": stop_ms,
        "elapsedMs": started.elapsed().as_millis(),
        "deliveryPassed": delivery_passed, "timingPassed": max_xruns == 0 && max_over_budget == 0,
        "maxXruns": max_xruns, "maxCallbackOverBudgetCount": max_over_budget,
        "supervisorMidiDrops": final_snapshot.midi_drops,
        "audioSignalObserved": peak > 0.00001,
        "observations": observations,
        "scope": "Production worker + installed VST + ASIO output; MIDI injected at worker IPC. No physical loopback latency or downstream audibility measurement."
    });
    std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
    println!("Report: {}", report.display());
    if !delivery_passed || max_xruns != 0 || max_over_budget != 0 || stop.is_err() {
        return Err("Audio reliability run found failures; see report".into());
    }
    Ok(())
}
