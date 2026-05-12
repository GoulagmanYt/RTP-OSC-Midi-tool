use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

use super::{
    engine::{db_to_linear, AudioEngine},
    runtime_state::AudioError,
};

use crate::logger::FrontendLogger;

impl AudioEngine {
    pub fn set_gain(&self, gain_db: f32) {
        if let Some(rt) = self.runtime.lock().as_mut() {
            rt.gain_bits
                .store(db_to_linear(gain_db).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn set_limiter_enabled(&self, enabled: bool) {
        if let Some(rt) = self.runtime.lock().as_mut() {
            rt.limiter_enabled.store(enabled, Ordering::Relaxed);
        }
    }

    pub fn current_backend(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.backend.clone())
    }

    pub fn current_device(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.device.clone())
    }

    pub fn current_sample_rate(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|r| r.sample_rate)
    }

    pub fn current_buffer_size(&self) -> Option<u32> {
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(frames)
            }
        })
    }

    pub fn requested_buffer_size(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.requested_buffer_size)
    }

    pub fn stream_buffer_size(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .and_then(|r| r.stream_buffer_size)
    }

    pub fn buffer_size_mismatch(&self) -> Option<bool> {
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(frames != r.requested_buffer_size)
            }
        })
    }

    pub fn vst_midi_compatible(&self) -> Option<bool> {
        self.runtime.lock().as_ref().map(|r| r.vst_midi_compatible)
    }

    pub fn xrun_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.xruns.load(Ordering::Relaxed))
    }

    pub fn limiter_enabled(&self) -> Option<bool> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.limiter_enabled.load(Ordering::Relaxed))
    }

    pub fn peak_levels(&self) -> Option<(f32, f32)> {
        self.runtime.lock().as_ref().map(|r| {
            let left = f32::from_bits(r.meter_left.load(Ordering::Relaxed));
            let right = f32::from_bits(r.meter_right.load(Ordering::Relaxed));
            (left, right)
        })
    }

    pub fn current_latency_ms(&self) -> Option<f32> {
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some((frames as f32 / r.sample_rate as f32) * 1000.0 * 2.0)
            }
        })
    }

    pub fn midi_drop_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.midi_drop_count.load(Ordering::Relaxed))
    }

    pub fn audio_lock_miss_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.audio_lock_miss_count.load(Ordering::Relaxed))
    }

    pub fn emergency_reset_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.emergency_reset_count.load(Ordering::Relaxed))
    }

    pub fn callback_last_us(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.callback_last_us.load(Ordering::Relaxed))
    }

    pub fn callback_max_us(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.callback_max_us.load(Ordering::Relaxed))
    }

    pub fn callback_over_budget_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.callback_over_budget_count.load(Ordering::Relaxed))
    }

    pub fn mmcss_enabled(&self) -> Option<bool> {
        super::windows_tuning::audio_mmcss_enabled()
    }

    pub fn power_throttling_disabled(&self) -> Option<bool> {
        super::windows_tuning::audio_power_throttling_disabled()
    }

    pub fn ping(&self) -> Result<(), AudioError> {
        if self.runtime.lock().is_none() {
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
