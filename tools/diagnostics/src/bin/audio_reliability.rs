//! Exercise the production isolated worker against a selected installed ASIO/VST rig.
use osc_midi_bridge::{
    audio::{AudioSettings, DspThreadPriority},
    config::ConfigStore,
    vst_worker::{VstWorkerState, VstWorkerSupervisor},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

// Timer resolution belongs to the generating process; a request in the worker
// does not make this diagnostic process's Tokio deadlines accurate on Windows.
#[cfg(windows)]
struct TimerResolution(bool);
#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}
#[cfg(windows)]
impl TimerResolution {
    fn request() -> Self {
        let active = unsafe { timeBeginPeriod(1) } == 0;
        if !active {
            eprintln!("Windows rejected diagnostic timer resolution");
        }
        Self(active)
    }
}
#[cfg(windows)]
impl Drop for TimerResolution {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                timeEndPeriod(1);
            }
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let _timer_resolution = TimerResolution::request();
    let mut args = std::env::args().skip(1);
    let mut buffer = None;
    let mut render_ahead_blocks = 1;
    let mut dsp_thread_priority = DspThreadPriority::Mmcss;
    let mut events_per_second = None;
    let mut exercise_game_mode = false;
    let mut game_mode = true;
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
            "--events-per-second" => events_per_second = Some(value.parse::<u32>()?),
            "--game-mode" => game_mode = value.parse()?,
            "--exercise-game-mode" => exercise_game_mode = value.parse()?,
            "--render-ahead-blocks" => render_ahead_blocks = value.parse()?,
            "--dsp-priority" => {
                dsp_thread_priority = match value.as_str() {
                    "mmcss" => DspThreadPriority::Mmcss,
                    "timeCritical" => DspThreadPriority::TimeCritical,
                    _ => return Err("Invalid DSP priority".into()),
                }
            }
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
    if events_per_second.is_some_and(|rate| rate == 0 || rate > 100_000) {
        return Err("events per second must be 1..100000".into());
    }
    let config = ConfigStore::new().load();
    let settings = AudioSettings {
        game_mode,
        render_ahead_blocks,
        dsp_thread_priority,
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
    let mut game_mode_validation = None;
    if exercise_game_mode {
        supervisor.set_game_mode(true).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let before = supervisor.snapshot();
        let parameters = supervisor.list_parameters().await?;
        if let Some(parameter) = parameters.first() {
            supervisor
                .set_parameter(parameter.index, parameter.value)
                .await?;
        }
        supervisor.save_state().await?;
        supervisor.open_editor().await?;
        supervisor.close_editor().await?; // latest deferred intent is closed: no UI is shown by this test.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let deferred = supervisor.snapshot();
        if !deferred.metrics.deferred_save
            || deferred.metrics.deferred_editor != Some(false)
            || deferred.metrics.maintenance
            || deferred.metrics.callback_count <= before.metrics.callback_count
            || deferred.metrics.audio_lock_misses != before.metrics.audio_lock_misses
        {
            return Err("game-mode protection failed".into());
        }
        supervisor.set_game_mode(false).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let flushed = supervisor.snapshot();
        if flushed.metrics.deferred_save
            || flushed.metrics.deferred_editor.is_some()
            || flushed.metrics.maintenance
        {
            return Err("deferred operations were not completed".into());
        }
        game_mode_validation = Some(
            serde_json::json!({ "before": before.metrics, "deferred": deferred.metrics, "flushed": flushed.metrics }),
        );
        supervisor.set_game_mode(game_mode).await?;
    }
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
    let batch_size = events_per_second.map_or(16, |rate| if rate <= 1000 { 1 } else { 16 });
    let half_period = events_per_second.map_or(Duration::from_millis(10), |rate| {
        Duration::from_secs_f64(batch_size as f64 / f64::from(rate))
    });
    let mut pacing = tokio::time::interval(half_period);
    pacing.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    pacing.tick().await;
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
            (notes - sent_on).min(batch_size)
        } else {
            batch_size
        };
        for index in 0..count {
            if !supervisor.try_send_midi(&[0x90, 48 + index as u8, 80]) {
                rejected += 1;
            }
            sent_on += 1;
        }
        pacing.tick().await;
        for index in 0..count {
            if !supervisor.try_send_midi(&[0x80, 48 + index as u8, 0]) {
                rejected += 1;
            }
            sent_off += 1;
        }
        pacing.tick().await;
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
        && before_panic.metrics.audio_emergency_resets == 0
        && before_panic.metrics.audio_lock_misses == 0
        && rejected == 0
        && max_drops == 0
        && final_snapshot.state == VstWorkerState::Ready
        && final_snapshot.restarts == 0
        && final_snapshot.midi_drops == 0
        && final_snapshot.metrics.midi_queue_depth == 0;
    let stopped_at = Instant::now();
    let stop = supervisor.stop().await;
    let stop_ms = stopped_at.elapsed().as_millis();
    let period_us =
        u64::from(status.stream_buffer_size) * 1_000_000 / u64::from(status.sample_rate);
    let delivery_timing_passed = before_panic.metrics.midi_delivery_p99_us <= period_us * 2
        && before_panic.metrics.midi_delivery_max_us <= period_us * 5;
    let result = serde_json::json!({
        "eventsPerSecondRequested": events_per_second,
        "eventsPerSecondObserved": (sent_on + sent_off) as f64 * 1000.0 / transmission_ms.max(1) as f64, "gameMode": game_mode, "gameModeValidation": game_mode_validation,
        "deliveryTimingPassed": delivery_timing_passed,
        "status": status, "requestedNotes": notes, "requestedDurationSeconds": duration_seconds,
        "transmissionMs": transmission_ms, "requestedActiveSeconds": active_seconds, "requestedCycleSeconds": cycle_seconds, "metricsBeforeFinalPanic": before_panic.metrics, "noteOnsSubmitted": sent_on,
        "noteOffsSubmitted": sent_off, "rejectedSubmissions": rejected,
        "peakObserved": peak, "maxObservedMidiAgeUs": max_midi_age_us,
        "finalMetrics": final_snapshot.metrics, "workerStateBeforeStop": final_snapshot.state,
        "workerRestarts": final_snapshot.restarts, "workerExit": final_snapshot.last_exit,
        "failure": failure, "stopError": stop.as_ref().err(), "stopMs": stop_ms,
        "elapsedMs": started.elapsed().as_millis(),
        "deliveryPassed": delivery_passed, "timingPassed": max_xruns == 0 && max_over_budget == 0 && final_snapshot.metrics.render_underflows == 0 && final_snapshot.metrics.callback_gap_count == 0,
        "maxXruns": max_xruns, "maxCallbackOverBudgetCount": max_over_budget,
        "supervisorMidiDrops": final_snapshot.midi_drops,
        "audioSignalObserved": peak > 0.00001,
        "observations": observations,
        "scope": "Production worker + installed VST + ASIO output; MIDI injected at worker IPC. No physical loopback latency or downstream audibility measurement."
    });
    std::fs::write(&report, serde_json::to_vec_pretty(&result)?)?;
    println!("Report: {}", report.display());
    if !delivery_passed
        || final_snapshot.metrics.render_underflows > 0
        || final_snapshot.metrics.callback_gap_count > 0
        || !delivery_timing_passed
        || max_xruns != 0
        || max_over_budget != 0
        || stop.is_err()
    {
        return Err("Audio reliability run found failures; see report".into());
    }
    Ok(())
}
