#![allow(deprecated)]
use super::{
    engine::AudioEngine,
    runtime_state::{AudioError, PluginBackend},
};
use crate::{
    types::{VstParameter, VstPluginEntry},
    vst_worker::WorkerMetrics,
};
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
};

pub(super) struct MaintenanceGuard {
    engine: AudioEngine,
    telemetry: Option<Arc<super::runtime_state::AudioTelemetry>>,
}
impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        // This token belongs to the captured stream, never a replacement runtime.
        if let Some(telemetry) = &self.telemetry {
            telemetry.callback_gate.reopen();
        }
        self.engine.maintenance.store(false, Ordering::Release);
    }
}

pub(super) fn finish_maintenance(guard: MaintenanceGuard, publish: impl FnOnce()) {
    drop(guard);
    publish();
}

/// Control serialization excludes other threads; generations protect same-thread reentry.
fn flush_pending_actions(
    save: &AtomicBool,
    editor: &AtomicU8,
    generations: (&AtomicU64, &AtomicU64),
    capture: impl FnOnce() -> Result<(), AudioError>,
    edit: impl FnOnce(u8) -> Result<(), AudioError>,
) -> Result<(), AudioError> {
    let mut failure = None;
    if save.load(Ordering::Acquire) {
        let generation = generations.0.load(Ordering::Acquire);
        match capture() {
            Ok(()) => {
                if generations.0.load(Ordering::Acquire) == generation {
                    save.store(false, Ordering::Release);
                }
            }
            Err(error) => failure = Some(error),
        }
    }
    let intent = editor.load(Ordering::Acquire);
    if intent != 0 {
        let generation = generations.1.load(Ordering::Acquire);
        match edit(intent) {
            Ok(()) => {
                if generations.1.load(Ordering::Acquire) == generation {
                    editor.store(0, Ordering::Release);
                }
            }
            Err(error) => {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

impl AudioEngine {
    fn transition_game_mode(
        &self,
        enabled: bool,
        operation: impl FnOnce() -> Result<(), AudioError>,
    ) -> Result<(), AudioError> {
        let _control = self.control_gate.lock();
        let generation = self
            .game_mode_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let previous = self.game_mode.swap(enabled, Ordering::AcqRel);
        let result = operation();
        if result.is_err() && self.game_mode_generation.load(Ordering::Acquire) == generation {
            self.game_mode.store(previous, Ordering::Release);
        }
        result
    }

    pub(super) fn apply_editor_intent(
        &self,
        intent: u8,
        operation: impl FnOnce() -> Result<(), AudioError>,
    ) -> Result<(), AudioError> {
        let _control = self.control_gate.lock();
        let generation = self
            .deferred_editor_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.deferred_editor.store(intent, Ordering::Release);
        if self.game_mode.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = operation();
        if result.is_ok() && self.deferred_editor_generation.load(Ordering::Acquire) == generation {
            self.deferred_editor.store(0, Ordering::Release);
        }
        result
    }

    pub(super) fn wait_for_maintenance_completion(&self) -> Result<(), AudioError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.maintenance.load(Ordering::Acquire) {
            if std::time::Instant::now() >= deadline {
                return Err(AudioError::Message(
                    "Previous native VST maintenance is still executing".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(())
    }

    pub(super) fn begin_maintenance(&self) -> Result<MaintenanceGuard, AudioError> {
        if self.maintenance.swap(true, Ordering::AcqRel) {
            return Err(AudioError::Message(
                "Audio maintenance already active".into(),
            ));
        }
        let telemetry = self
            .runtime
            .lock()
            .as_ref()
            .map(|runtime| runtime.telemetry.clone());
        if let Some(telemetry) = &telemetry {
            // Taking the runtime lock above finishes any parameter admission that
            // observed maintenance=false. Drain it before closing callback admission.
            if let Err(error) = wait_for_parameters(telemetry, std::time::Duration::from_secs(5)) {
                self.maintenance.store(false, Ordering::Release);
                return Err(error);
            }
            if !telemetry
                .callback_gate
                .close_and_wait(std::time::Duration::from_secs(5))
            {
                telemetry.callback_gate.reopen();
                self.maintenance.store(false, Ordering::Release);
                return Err(AudioError::Message(
                    "Timed out waiting for audio callback quiescence".into(),
                ));
            }
        }
        Ok(MaintenanceGuard {
            engine: self.clone(),
            telemetry,
        })
    }

    pub fn set_game_mode(&self, enabled: bool, app: tauri::AppHandle) -> Result<(), AudioError> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return crate::tauri::utils::safe_block_on(self.worker.set_game_mode(enabled))
                .map_err(AudioError::Message);
        }
        let _control = self.control_gate.lock();
        if enabled == self.game_mode.load(Ordering::Acquire)
            && (enabled
                || (!self.deferred_save.load(Ordering::Acquire)
                    && self.deferred_editor.load(Ordering::Acquire) == 0))
        {
            self.game_mode_generation.fetch_add(1, Ordering::AcqRel);
            return Ok(());
        }
        if enabled {
            self.transition_game_mode(true, || self.close_vst_ui_now(app))?;
        } else {
            self.transition_game_mode(false, || {
                if let Err(error) = self.flush_deferred_actions(app.clone()) {
                    crate::logger::background_log(
                        "warn",
                        format!("Game Mode exited; pending VST actions will be retried: {error}"),
                    );
                    self.schedule_deferred_retry(app);
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn flush_deferred_actions(&self, app: tauri::AppHandle) -> Result<(), AudioError> {
        flush_pending_actions(
            &self.deferred_save,
            &self.deferred_editor,
            (
                &self.deferred_save_generation,
                &self.deferred_editor_generation,
            ),
            || self.checkpoint_vst_state(app.clone()),
            |intent| match intent {
                1 => self.open_vst_ui(app.clone()),
                2 => self.close_vst_ui(app.clone()),
                _ => Ok(()),
            },
        )
    }

    pub(super) fn schedule_deferred_retry(&self, app: tauri::AppHandle) {
        if self.deferred_retry_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut delay = std::time::Duration::from_millis(100);
            loop {
                tokio::time::sleep(delay).await;
                let attempt = engine.clone();
                let app = app.clone();
                let retry = tauri::async_runtime::spawn_blocking(move || {
                    let _control = attempt.control_gate.lock();
                    if attempt.game_mode.load(Ordering::Acquire) || attempt.runtime.lock().is_none()
                    {
                        attempt
                            .deferred_retry_scheduled
                            .store(false, Ordering::Release);
                        return false;
                    }
                    if let Err(error) = attempt.flush_deferred_actions(app) {
                        crate::logger::background_log(
                            "warn",
                            format!("Deferred VST actions remain pending: {error}"),
                        );
                    }
                    let pending = attempt.deferred_save.load(Ordering::Acquire)
                        || attempt.deferred_editor.load(Ordering::Acquire) != 0;
                    if !pending {
                        attempt
                            .deferred_retry_scheduled
                            .store(false, Ordering::Release);
                    }
                    pending
                })
                .await;
                match retry {
                    Ok(true) => delay = (delay * 2).min(std::time::Duration::from_secs(5)),
                    Ok(false) => break,
                    Err(error) => {
                        engine
                            .deferred_retry_scheduled
                            .store(false, Ordering::Release);
                        crate::logger::background_log(
                            "error",
                            format!("Deferred VST retry task failed: {error}"),
                        );
                        break;
                    }
                }
            }
        });
    }

    /// Atomic-only snapshot: heartbeat must not touch runtime or plugin locks.
    pub fn realtime_metrics(&self) -> WorkerMetrics {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.snapshot().metrics;
        }
        let mut metrics = WorkerMetrics {
            midi_transport_failed: self.midi_transport_failed.load(Ordering::Acquire),
            midi_io_threads: crate::realtime::thread_snapshot(),
            game_mode: self.game_mode.load(Ordering::Acquire),
            editor_open: self.editor_open.load(Ordering::Acquire),
            maintenance: self.maintenance.load(Ordering::Acquire),
            deferred_save: self.deferred_save.load(Ordering::Acquire),
            deferred_editor: match self.deferred_editor.load(Ordering::Acquire) {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            },
            x86_worker_alive: true,
            ..WorkerMetrics::default()
        };
        let published = self.health_telemetry.load();
        if let Some(t) = published.as_ref() {
            metrics.editor_open = t.editor_open.load(Ordering::Acquire);
            metrics.pipe_wait_max_us = t.pipe_wait_max_us.load(Ordering::Relaxed);
            metrics.audio_wait_max_us = t.audio_wait_max_us.load(Ordering::Relaxed);
            metrics.midi_delivery_max_us = t.midi_delivery_max_us.load(Ordering::Relaxed);
            metrics.midi_delivery_p99_us = t.midi_delivery_percentile(99);
            metrics.x86_bridge_underruns = t.x86_underruns.load(Ordering::Relaxed);
            metrics.x86_bridge_overruns = t.x86_overruns.load(Ordering::Relaxed);
            metrics.x86_worker_alive = t.x86_alive.load(Ordering::Relaxed);
            metrics.callback_count = t.callback_count.load(Ordering::Relaxed);
            metrics.callback_interval_max_us = t.callback_interval_max_us.load(Ordering::Relaxed);
            let tri_state = |cell: &std::sync::atomic::AtomicU8| match cell.load(Ordering::Relaxed)
            {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            };
            metrics.audio_mmcss_owned = tri_state(&t.mmcss_owned);
            metrics.audio_mmcss_critical = tri_state(&t.mmcss_critical);
            metrics.audio_peak_l = f32::from_bits(t.meter_left.load(Ordering::Relaxed));
            metrics.audio_peak_r = f32::from_bits(t.meter_right.load(Ordering::Relaxed));
            metrics.audio_xruns = t.xruns.load(Ordering::Relaxed);
            metrics.stream_recovery_requests = t.stream_recovery_requests.load(Ordering::Relaxed);
            metrics.stream_route_changes = t.stream_route_changes.load(Ordering::Relaxed);
            metrics.backend_latency_us = t.backend_latency_us.load(Ordering::Relaxed);
            metrics.audio_midi_drops = t.midi_drop_count.load(Ordering::Relaxed);
            metrics.audio_lock_misses = t.audio_lock_miss_count.load(Ordering::Relaxed);
            metrics.audio_emergency_resets = t.emergency_reset_count.load(Ordering::Relaxed);
            metrics.callback_last_us = t.callback_last_us.load(Ordering::Relaxed);
            metrics.callback_max_us = t.callback_max_us.load(Ordering::Relaxed);
            metrics.callback_over_budget_count =
                t.callback_over_budget_count.load(Ordering::Relaxed);
            metrics.consecutive_deadline_misses =
                t.consecutive_deadline_misses.load(Ordering::Relaxed);
            metrics.dsp_process_last_us = t.dsp_process_last_us.load(Ordering::Relaxed);
            metrics.dsp_process_max_us = t.dsp_process_max_us.load(Ordering::Relaxed);
            metrics.dsp_process_p95_us = t.dsp_percentile_us(95);
            metrics.dsp_process_p99_us = t.dsp_percentile_us(99);
            metrics.midi_queue_depth = t.audio_midi_queue_depth.load(Ordering::Relaxed);
            metrics.midi_queue_max_depth = t.audio_midi_queue_max_depth.load(Ordering::Relaxed);
            metrics.midi_oldest_us = t.audio_midi_oldest_us.load(Ordering::Relaxed);
        }
        metrics
    }

    pub fn realtime_diagnostics(&self) -> crate::realtime::RealtimeDiagnostics {
        let mut snapshot = crate::realtime::diagnostics(self.realtime_metrics());
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let worker = self.worker.snapshot();
            snapshot.heartbeat_delayed = worker.heartbeat_delayed;
            snapshot.transport_healthy = Some(
                !worker.midi_transport_failed
                    && !worker.metrics.midi_transport_failed
                    && worker.state == crate::vst_worker::VstWorkerState::Ready,
            );
            snapshot.control_healthy = Some(
                worker.state == crate::vst_worker::VstWorkerState::Ready
                    && !worker.heartbeat_delayed,
            );
            snapshot.callback_healthy = Some(
                worker.state == crate::vst_worker::VstWorkerState::Ready && worker.callback_healthy,
            );
        }
        snapshot
    }

    pub(super) fn persist_live_parameters(&self) -> Result<(), AudioError> {
        let (entry, values) = {
            let runtime = self.runtime.lock();
            let runtime = runtime
                .as_ref()
                .ok_or_else(|| AudioError::Message("Audio not started".into()))?;
            let changes = self.live_parameters.lock();
            let cache = runtime.parameter_cache.lock();
            (
                runtime.vst_plugin.clone(),
                changes
                    .keys()
                    .filter_map(|index| cache.get(*index).cloned())
                    .collect::<Vec<_>>(),
            )
        };
        persist_parameter_snapshot(&entry, values)
    }
}

pub(super) fn persist_parameter_snapshot(
    entry: &VstPluginEntry,
    values: Vec<VstParameter>,
) -> Result<(), AudioError> {
    let path = super::state_codec::state_path_for_plugin(entry)
        .ok_or_else(|| AudioError::Message("VST state path unavailable".into()))?
        .with_extension("live.json");
    let payload = serde_json::to_vec(&LiveParameters {
        plugin_id: entry.id.clone(),
        parameters: values,
    })
    .map_err(|e| AudioError::Message(e.to_string()))?;
    crate::atomic_file::write_atomically(&path, &payload)
        .map_err(|e| AudioError::Message(e.to_string()))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LiveParameters {
    plugin_id: String,
    parameters: Vec<VstParameter>,
}

pub(super) fn restore_live_parameters(
    plugin: &Arc<Mutex<PluginBackend>>,
    cache: &Arc<Mutex<Vec<VstParameter>>>,
    entry: &VstPluginEntry,
) -> Result<BTreeMap<usize, f32>, AudioError> {
    let Some(path) =
        super::state_codec::state_path_for_plugin(entry).map(|p| p.with_extension("live.json"))
    else {
        return Ok(BTreeMap::new());
    };
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let bytes = std::fs::read(&path).map_err(|e| AudioError::Message(e.to_string()))?;
    let saved: LiveParameters =
        serde_json::from_slice(&bytes).map_err(|e| AudioError::Message(e.to_string()))?;
    if saved.plugin_id != entry.id {
        return Err(AudioError::Message(
            "Live parameter identity mismatch".into(),
        ));
    }
    let mut cache = cache.lock();
    let mut restored = BTreeMap::new();
    for value in saved.parameters {
        let Some(current) = cache.get_mut(value.index) else {
            continue;
        };
        if current.id != value.id
            || current.name != value.name
            || !value.value.is_finite()
            || current.flags & (1 << 1) != 0
        {
            continue;
        }
        // Submit these values once through the backend's audio parameter path
        // below. An immediate UI setter would enqueue a second untracked copy
        // and could let native capture outrun component restoration.
        current.value = value.value.clamp(0.0, 1.0);
        restored.insert(value.index, current.value);
    }
    let indices = restored.keys().copied().collect::<Vec<_>>();
    match &mut *plugin.lock() {
        PluginBackend::Vst3 { instance, .. } => instance
            .prepare_live_parameter_restore(&indices)
            .map_err(|e| AudioError::Message(e.to_string()))?,
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        PluginBackend::Remote { instance } => instance
            .prepare_live_parameter_restore(&indices)
            .map_err(AudioError::Message)?,
        PluginBackend::Vst2 { .. } => {}
    }
    Ok(restored)
}

pub(super) fn queue_restored_parameters(
    sender: &crossbeam_channel::Sender<super::runtime_state::ParameterCommand>,
    cache: &[VstParameter],
    restored: &BTreeMap<usize, f32>,
    telemetry: &super::runtime_state::AudioTelemetry,
) -> Result<(), AudioError> {
    let remaining = sender.capacity().unwrap_or(0).saturating_sub(sender.len());
    if restored.len() > remaining {
        return Err(AudioError::Message(
            "Restored live parameters exceed the existing command queue; sidecar retained".into(),
        ));
    }
    let first = telemetry
        .parameter_accepted_sequence
        .load(Ordering::Acquire);
    first
        .checked_add(restored.len() as u64)
        .ok_or_else(|| AudioError::Message("Parameter generation exhausted".into()))?;
    let mut commands = Vec::with_capacity(restored.len());
    for (offset, (&index, &value)) in restored.iter().enumerate() {
        let metadata = cache
            .get(index)
            .ok_or_else(|| AudioError::Message("Restored parameter metadata unavailable".into()))?;
        commands.push(super::runtime_state::ParameterCommand {
            sequence: first + offset as u64 + 1,
            index,
            id: metadata.id,
            flags: metadata.flags,
            step_count: metadata.step_count,
            value,
        });
    }
    for command in commands {
        let sequence = command.sequence;
        sender.try_send(command).map_err(|_| {
            AudioError::Message("Restored parameter queue unavailable; sidecar retained".into())
        })?;
        telemetry
            .parameter_accepted_sequence
            .store(sequence, Ordering::Release);
    }
    Ok(())
}

pub(super) fn wait_for_parameters(
    telemetry: &super::runtime_state::AudioTelemetry,
    timeout: std::time::Duration,
) -> Result<(), AudioError> {
    let accepted = telemetry
        .parameter_accepted_sequence
        .load(Ordering::Acquire);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if telemetry.parameter_failed_sequence.load(Ordering::Acquire) != 0 {
            return Err(AudioError::Message(
                "An accepted VST parameter could not be applied; native capture deferred".into(),
            ));
        }
        if telemetry.parameter_applied_sequence.load(Ordering::Acquire) >= accepted {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(AudioError::Message(
                "Timed out waiting for accepted VST parameters; native capture deferred".into(),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::super::runtime_state::AudioTelemetry;
    use super::*;

    #[test]
    fn newer_game_mode_request_wins_during_native_transition() {
        let engine = AudioEngine::new();
        engine.game_mode.store(false, Ordering::Release);
        engine
            .transition_game_mode(true, || {
                assert!(engine.game_mode.load(Ordering::Acquire));
                engine.transition_game_mode(false, || Ok(()))?;
                Ok(())
            })
            .unwrap();
        assert!(!engine.game_mode.load(Ordering::Acquire));
        assert!(engine
            .transition_game_mode(true, || {
                engine.transition_game_mode(true, || Ok(()))?;
                Err(AudioError::Message("older transition failed".into()))
            })
            .is_err());
        assert!(engine.game_mode.load(Ordering::Acquire));
    }

    #[test]
    fn new_editor_request_supersedes_failed_deferred_intent() {
        let engine = AudioEngine::new();
        engine.game_mode.store(false, Ordering::Release);
        engine.deferred_editor.store(1, Ordering::Release);
        engine.apply_editor_intent(2, || Ok(())).unwrap();
        assert_eq!(engine.deferred_editor.load(Ordering::Acquire), 0);
        assert!(engine
            .apply_editor_intent(1, || Err(AudioError::Message("busy".into())))
            .is_err());
        assert_eq!(engine.deferred_editor.load(Ordering::Acquire), 1);
    }

    #[test]
    fn reentrant_editor_request_is_not_erased_by_older_success() {
        for latest in [1, 2] {
            let engine = AudioEngine::new();
            engine.game_mode.store(false, Ordering::Release);
            engine
                .apply_editor_intent(1, || {
                    assert!(engine
                        .apply_editor_intent(latest, || Err(AudioError::Message("busy".into())))
                        .is_err());
                    Ok(())
                })
                .unwrap();
            assert_eq!(engine.deferred_editor.load(Ordering::Acquire), latest);
        }
    }

    #[test]
    fn capture_completion_preserves_newer_save_request() {
        let save = AtomicBool::new(true);
        let editor = AtomicU8::new(0);
        let save_generation = AtomicU64::new(1);
        let editor_generation = AtomicU64::new(0);
        flush_pending_actions(
            &save,
            &editor,
            (&save_generation, &editor_generation),
            || {
                save_generation.fetch_add(1, Ordering::AcqRel);
                save.store(true, Ordering::Release);
                Ok(())
            },
            |_| panic!("no editor request"),
        )
        .unwrap();
        assert!(save.load(Ordering::Acquire));
    }

    #[test]
    fn game_mode_retains_editor_intent_without_native_access() {
        let engine = AudioEngine::new();
        engine
            .apply_editor_intent(1, || panic!("Game Mode must defer native access"))
            .unwrap();
        engine
            .apply_editor_intent(2, || panic!("Game Mode must defer native access"))
            .unwrap();
        assert_eq!(engine.deferred_editor.load(Ordering::Acquire), 2);
    }

    #[test]
    fn failed_save_does_not_block_editor_and_is_retried() {
        use std::sync::atomic::{AtomicBool, AtomicU8};
        let save = AtomicBool::new(true);
        let editor = AtomicU8::new(1);
        let mut opened = false;
        assert!(flush_pending_actions(
            &save,
            &editor,
            (&AtomicU64::new(0), &AtomicU64::new(0)),
            || Err(AudioError::Message("transient capture failure".into())),
            |intent| {
                assert_eq!(intent, 1);
                opened = true;
                Ok(())
            },
        )
        .is_err());
        assert!(opened);
        assert!(save.load(Ordering::Acquire));
        assert_eq!(editor.load(Ordering::Acquire), 0);
        let mut captures = 0;
        flush_pending_actions(
            &save,
            &editor,
            (&AtomicU64::new(0), &AtomicU64::new(0)),
            || {
                captures += 1;
                Ok(())
            },
            |_| panic!("successful editor action must not be repeated"),
        )
        .unwrap();
        assert_eq!(captures, 1);
        assert!(!save.load(Ordering::Acquire));
    }

    #[test]
    fn failed_editor_retains_latest_intent_without_repeating_successful_save() {
        use std::sync::atomic::{AtomicBool, AtomicU8};
        let save = AtomicBool::new(true);
        let editor = AtomicU8::new(1);
        assert!(flush_pending_actions(
            &save,
            &editor,
            (&AtomicU64::new(0), &AtomicU64::new(0)),
            || Ok(()),
            |_| Err(AudioError::Message("transient editor failure".into()))
        )
        .is_err());
        assert!(!save.load(Ordering::Acquire));
        assert_eq!(editor.load(Ordering::Acquire), 1);
        editor.store(2, Ordering::Release);
        flush_pending_actions(
            &save,
            &editor,
            (&AtomicU64::new(0), &AtomicU64::new(0)),
            || panic!("successful capture must not be repeated"),
            |intent| {
                assert_eq!(intent, 2);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(editor.load(Ordering::Acquire), 0);
    }

    #[test]
    fn maintenance_is_released_before_completion_is_published() {
        let engine = AudioEngine::new();
        let guard = engine.begin_maintenance().unwrap();
        finish_maintenance(guard, || {
            assert!(!engine.maintenance.load(Ordering::Acquire));
            assert!(engine.begin_maintenance().is_ok());
        });
    }

    #[test]
    fn maintenance_only_reopens_its_captured_stream() {
        let engine = AudioEngine::new();
        let old = Arc::new(AudioTelemetry::new());
        let replacement = Arc::new(AudioTelemetry::new());
        assert!(old.callback_gate.close_and_wait(std::time::Duration::ZERO));
        assert!(replacement
            .callback_gate
            .close_and_wait(std::time::Duration::ZERO));
        engine.maintenance.store(true, Ordering::Release);
        let guard = MaintenanceGuard {
            engine: engine.clone(),
            telemetry: Some(old.clone()),
        };
        drop(guard);
        assert!(old.callback_gate.try_enter().is_some());
        assert!(replacement.callback_gate.try_enter().is_none());
    }

    fn parameter(index: usize) -> VstParameter {
        VstParameter {
            index,
            id: index as u32,
            flags: 0,
            step_count: 0,
            unit_id: 0,
            name: format!("parameter{index}"),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: String::new(),
            value: 0.0,
        }
    }

    #[test]
    fn restored_sidecar_values_need_audio_acknowledgement_before_capture() {
        let (sender, receiver) = crossbeam_channel::bounded(2);
        let telemetry = AudioTelemetry::new();
        let restored = BTreeMap::from([(0, 0.75), (1, 0.25)]);
        queue_restored_parameters(
            &sender,
            &[parameter(0), parameter(1)],
            &restored,
            &telemetry,
        )
        .unwrap();
        assert_eq!(
            telemetry
                .parameter_accepted_sequence
                .load(Ordering::Acquire),
            2
        );
        assert!(wait_for_parameters(&telemetry, std::time::Duration::ZERO).is_err());
        let first = receiver.try_recv().unwrap();
        let last = receiver.try_recv().unwrap();
        assert_eq!((first.sequence, first.index, first.value), (1, 0, 0.75));
        assert_eq!((last.sequence, last.index, last.value), (2, 1, 0.25));
        telemetry
            .parameter_applied_sequence
            .store(last.sequence, Ordering::Release);
        assert!(wait_for_parameters(&telemetry, std::time::Duration::ZERO).is_ok());
    }

    #[test]
    fn oversized_sidecar_restore_fails_without_partial_admission() {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let telemetry = AudioTelemetry::new();
        let restored = BTreeMap::from([(0, 0.75), (1, 0.25)]);
        assert!(queue_restored_parameters(
            &sender,
            &[parameter(0), parameter(1)],
            &restored,
            &telemetry
        )
        .is_err());
        assert!(receiver.is_empty());
        assert_eq!(
            telemetry
                .parameter_accepted_sequence
                .load(Ordering::Acquire),
            0
        );
    }

    #[test]
    fn native_capture_barrier_rejects_unapplied_and_failed_commands() {
        let telemetry = AudioTelemetry::new();
        telemetry
            .parameter_accepted_sequence
            .store(10, Ordering::Release);
        telemetry
            .parameter_applied_sequence
            .store(9, Ordering::Release);
        assert!(wait_for_parameters(&telemetry, std::time::Duration::ZERO).is_err());
        telemetry
            .parameter_applied_sequence
            .store(10, Ordering::Release);
        assert!(wait_for_parameters(&telemetry, std::time::Duration::ZERO).is_ok());
        telemetry
            .parameter_failed_sequence
            .store(8, Ordering::Release);
        assert!(wait_for_parameters(&telemetry, std::time::Duration::ZERO).is_err());
    }

    #[test]
    fn native_capture_barrier_waits_for_audio_application() {
        let telemetry = Arc::new(AudioTelemetry::new());
        telemetry
            .parameter_accepted_sequence
            .store(1, Ordering::Release);
        let audio = telemetry.clone();
        let callback = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            audio.parameter_applied_sequence.store(1, Ordering::Release);
        });
        wait_for_parameters(&telemetry, std::time::Duration::from_secs(1)).unwrap();
        callback.join().unwrap();
    }
}
