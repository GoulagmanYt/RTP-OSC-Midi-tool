use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

use super::{
    engine::{db_to_linear, AudioEngine},
    runtime_state::{AudioError, AudioLifecycleState},
};

use crate::logger::FrontendLogger;

impl AudioEngine {
    pub fn lifecycle_state(&self) -> &'static str {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return match self.worker.snapshot().state {
                crate::vst_worker::VstWorkerState::Disabled => "stopped",
                crate::vst_worker::VstWorkerState::Starting
                | crate::vst_worker::VstWorkerState::Loading => "loading",
                crate::vst_worker::VstWorkerState::Ready => "running",
                crate::vst_worker::VstWorkerState::Stopping => "stopping",
                crate::vst_worker::VstWorkerState::Faulted => "faulted",
            };
        }
        AudioLifecycleState::from_u8(
            self.lifecycle_state
                .load(std::sync::atomic::Ordering::Acquire),
        )
        .as_str()
    }

    pub fn set_gain(&self, gain_db: f32) {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let _ = self.worker.try_set_gain(gain_db);
            return;
        }
        if let Some(rt) = self.runtime.lock().as_mut() {
            rt.controls
                .gain_bits
                .store(db_to_linear(gain_db).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn set_limiter_enabled(&self, enabled: bool) {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let _ = self.worker.try_set_limiter_enabled(enabled);
            return;
        }
        if let Some(rt) = self.runtime.lock().as_mut() {
            rt.controls
                .limiter_enabled
                .store(enabled, Ordering::Relaxed);
        }
    }

    pub fn current_backend(&self) -> Option<String> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.snapshot().status.map(|status| status.backend);
        }
        self.runtime.lock().as_ref().map(|r| r.backend.clone())
    }

    pub fn current_device(&self) -> Option<String> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.snapshot().status.map(|status| status.device);
        }
        self.runtime.lock().as_ref().map(|r| r.device.clone())
    }

    pub fn current_sample_rate(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.sample_rate);
        }
        self.runtime.lock().as_ref().map(|r| r.sample_rate)
    }

    pub fn current_buffer_size(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.stream_buffer_size);
        }
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.telemetry.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(frames)
            }
        })
    }

    pub fn requested_buffer_size(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.requested_buffer_size);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.requested_buffer_size)
    }

    pub fn stream_buffer_size(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.stream_buffer_size);
        }
        self.runtime
            .lock()
            .as_ref()
            .and_then(|r| r.stream_buffer_size)
    }

    pub fn buffer_size_mismatch(&self) -> Option<bool> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.stream_buffer_size != status.requested_buffer_size);
        }
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.telemetry.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(frames != r.requested_buffer_size)
            }
        })
    }

    pub fn vst_midi_compatible(&self) -> Option<bool> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.vst_midi_compatible);
        }
        self.runtime.lock().as_ref().map(|r| r.vst_midi_compatible)
    }

    pub fn xrun_count(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.audio_xruns);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.xruns.load(Ordering::Relaxed))
    }

    pub fn limiter_enabled(&self) -> Option<bool> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.limiter_enabled);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.controls.limiter_enabled.load(Ordering::Relaxed))
    }

    pub fn peak_levels(&self) -> Option<(f32, f32)> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let metrics = self.worker.snapshot().metrics;
            return Some((metrics.audio_peak_l, metrics.audio_peak_r));
        }
        self.runtime.lock().as_ref().map(|r| {
            let left = f32::from_bits(r.telemetry.meter_left.load(Ordering::Relaxed));
            let right = f32::from_bits(r.telemetry.meter_right.load(Ordering::Relaxed));
            (left, right)
        })
    }

    pub fn current_latency_ms(&self) -> Option<f32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.snapshot().status.and_then(|status| {
                (status.sample_rate > 0).then_some(
                    ((status.stream_buffer_size + status.plugin_latency_samples) as f32
                        / status.sample_rate as f32)
                        * 1000.0,
                )
            });
        }
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.telemetry.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(((frames + r.plugin_latency_samples) as f32 / r.sample_rate as f32) * 1000.0)
            }
        })
    }

    pub fn audio_buffer_period_ms(&self) -> Option<f32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self.worker.snapshot().status.and_then(|status| {
                (status.sample_rate > 0).then_some(
                    (status.stream_buffer_size as f32 / status.sample_rate as f32) * 1000.0,
                )
            });
        }
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.telemetry.block_size_frames.load(Ordering::Relaxed);
            (frames > 0 && r.sample_rate > 0)
                .then_some((frames as f32 / r.sample_rate as f32) * 1000.0)
        })
    }

    pub fn plugin_latency_samples(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.plugin_latency_samples);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|runtime| runtime.plugin_latency_samples)
    }

    pub fn midi_drop_count(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let snapshot = self.worker.snapshot();
            return Some(
                snapshot
                    .metrics
                    .audio_midi_drops
                    .saturating_add(snapshot.midi_drops),
            );
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.midi_drop_count.load(Ordering::Relaxed))
    }

    pub fn audio_lock_miss_count(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.audio_lock_misses);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.audio_lock_miss_count.load(Ordering::Relaxed))
    }

    pub fn emergency_reset_count(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.audio_emergency_resets);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.emergency_reset_count.load(Ordering::Relaxed))
    }

    pub fn callback_last_us(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.callback_last_us);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.callback_last_us.load(Ordering::Relaxed))
    }

    pub fn callback_max_us(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.callback_max_us);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.callback_max_us.load(Ordering::Relaxed))
    }

    pub fn callback_over_budget_count(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.callback_over_budget_count);
        }
        self.runtime.lock().as_ref().map(|r| {
            r.telemetry
                .callback_over_budget_count
                .load(Ordering::Relaxed)
        })
    }

    pub fn consecutive_deadline_misses(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.consecutive_deadline_misses);
        }
        self.runtime.lock().as_ref().map(|r| {
            r.telemetry
                .consecutive_deadline_misses
                .load(Ordering::Relaxed)
        })
    }

    pub fn dsp_process_last_us(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.dsp_process_last_us);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.dsp_process_last_us.load(Ordering::Relaxed))
    }

    pub fn dsp_process_max_us(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.dsp_process_max_us);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.dsp_process_max_us.load(Ordering::Relaxed))
    }

    pub fn dsp_process_percentile_us(&self, percentile: u32) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            let metrics = self.worker.snapshot().metrics;
            return Some(if percentile >= 99 {
                metrics.dsp_process_p99_us
            } else {
                metrics.dsp_process_p95_us
            });
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.dsp_percentile_us(percentile))
    }

    pub fn audio_midi_queue_depth(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.midi_queue_depth);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.audio_midi_queue_depth.load(Ordering::Relaxed))
    }

    pub fn audio_midi_queue_max_depth(&self) -> Option<u32> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.midi_queue_max_depth);
        }
        self.runtime.lock().as_ref().map(|r| {
            r.telemetry
                .audio_midi_queue_max_depth
                .load(Ordering::Relaxed)
        })
    }

    pub fn audio_midi_oldest_us(&self) -> Option<u64> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return Some(self.worker.snapshot().metrics.midi_oldest_us);
        }
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.telemetry.audio_midi_oldest_us.load(Ordering::Relaxed))
    }

    pub fn mmcss_enabled(&self) -> Option<bool> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.mmcss_enabled);
        }
        super::windows_tuning::audio_mmcss_enabled()
    }

    pub fn power_throttling_disabled(&self) -> Option<bool> {
        #[cfg(target_os = "windows")]
        if self.is_worker_enabled() {
            return self
                .worker
                .snapshot()
                .status
                .map(|status| status.power_throttling_disabled);
        }
        super::windows_tuning::audio_power_throttling_disabled()
    }

    pub fn vst_worker_state(&self) -> String {
        #[cfg(target_os = "windows")]
        {
            self.worker.snapshot().state.as_str().to_string()
        }
        #[cfg(not(target_os = "windows"))]
        {
            "disabled".to_string()
        }
    }

    pub fn vst_worker_restarts(&self) -> u32 {
        #[cfg(target_os = "windows")]
        {
            self.worker.snapshot().restarts
        }
        #[cfg(not(target_os = "windows"))]
        {
            0
        }
    }

    pub fn vst_worker_last_exit(&self) -> Option<String> {
        #[cfg(target_os = "windows")]
        {
            self.worker.snapshot().last_exit
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    pub fn vst_worker_editor_open(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.worker.snapshot().editor_open
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    pub fn ping(&self) -> Result<(), AudioError> {
        if !self.is_running() {
            return Err(AudioError::Message("Audio runtime not started".into()));
        }

        self.send_midi(&[0x90, 60, 100]);
        let cloned = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(220));
            cloned.send_midi(&[0x80, 60, 0]);
        });
        Ok(())
    }

    pub fn reload(
        &self,
        settings: super::runtime_state::AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        self.start(settings, vst_fallback, logger)
    }
}
