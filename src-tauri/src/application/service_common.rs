use chrono::Utc;
use serde::Serialize;
use tauri::Window;

use crate::{
    audio::AudioEngine,
    config::Config,
    error::CommandError,
    logger::{self, FrontendLogger},
    tauri::state::AppState,
    types::{BridgeStatus, RtpParticipantInfo, RtpSessionInfo},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticExport {
    pub(crate) timestamp: String,
    pub(crate) config: Config,
    pub(crate) status: BridgeStatus,
    pub(crate) rtp_participants: Vec<RtpParticipantInfo>,
    pub(crate) rtp_sessions: Vec<RtpSessionInfo>,
    pub(crate) logs: String,
}

pub(crate) fn build_diagnostic_export(state: &AppState) -> DiagnosticExport {
    let cfg = state.config_store.load();
    let status = with_audio_status(state.bridge.status(&cfg), &state.audio);
    DiagnosticExport {
        timestamp: Utc::now().to_rfc3339(),
        config: cfg,
        status,
        rtp_participants: state.bridge.rtp_participants(),
        rtp_sessions: state.rtp_discovery.cached(),
        logs: crate::tauri::utils::read_log_tail(200_000),
    }
}

pub(crate) fn with_audio_status(mut status: BridgeStatus, audio: &AudioEngine) -> BridgeStatus {
    status.audio_running = audio.is_running();
    status.vst_loaded = audio.is_vst_loaded();
    status.audio_latency_ms = audio.current_latency_ms();
    status.audio_buffer_period_ms = audio.audio_buffer_period_ms();
    status.plugin_latency_samples = audio.plugin_latency_samples();
    status.audio_backend = audio.current_backend();
    status.audio_device = audio.current_device();
    status.audio_sample_rate = audio.current_sample_rate();
    status.audio_buffer_size = audio.current_buffer_size();
    status.audio_requested_buffer_size = audio.requested_buffer_size();
    status.audio_stream_buffer_size = audio.stream_buffer_size();
    status.audio_buffer_mismatch = audio.buffer_size_mismatch();
    status.vst_midi_compatible = audio.vst_midi_compatible();
    status.audio_xruns = audio.xrun_count();
    status.audio_midi_drops = audio.midi_drop_count();
    status.audio_lock_misses = audio.audio_lock_miss_count();
    status.audio_emergency_resets = audio.emergency_reset_count();
    status.audio_callback_max_us = audio.callback_max_us();
    status.audio_callback_last_us = audio.callback_last_us();
    status.audio_callback_over_budget_count = audio.callback_over_budget_count();
    status.consecutive_deadline_misses = audio.consecutive_deadline_misses();
    status.dsp_process_last_us = audio.dsp_process_last_us();
    status.dsp_process_p95_us = audio.dsp_process_percentile_us(95);
    status.dsp_process_p99_us = audio.dsp_process_percentile_us(99);
    status.dsp_process_max_us = audio.dsp_process_max_us();
    status.audio_midi_queue_depth = audio.audio_midi_queue_depth();
    status.audio_midi_queue_max_depth = audio.audio_midi_queue_max_depth();
    status.audio_midi_oldest_us = audio.audio_midi_oldest_us();
    status.audio_lifecycle_state = audio.lifecycle_state().to_string();
    status.vst_worker_state = audio.vst_worker_state();
    status.vst_worker_restarts = audio.vst_worker_restarts();
    status.vst_worker_last_exit = audio.vst_worker_last_exit();
    status.audio_mmcss_enabled = audio.mmcss_enabled();
    status.audio_power_throttling_disabled = audio.power_throttling_disabled();
    status.audio_limiter_enabled = audio.limiter_enabled();
    status
}

pub(crate) fn frontend_logger(window: &Window, state: &AppState) -> FrontendLogger {
    FrontendLogger::new(window.clone(), state.dev_logging.clone())
}

pub(crate) fn paths_error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, "paths", message)
}

pub(crate) fn config_error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, "config", message)
}

pub(crate) fn runtime_error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, "runtime", message)
}

pub(crate) fn audio_error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, "audio", message)
}

pub(crate) fn io_to_error(code: &str, domain: &str, err: impl ToString) -> CommandError {
    CommandError::new(code, domain, err.to_string())
}

pub(crate) fn clear_log_file() -> Result<(), CommandError> {
    logger::clear_log_file().map_err(|err| paths_error("paths.clear-log-failed", err))
}

#[cfg(test)]
mod tests {
    #[test]
    fn with_audio_status_includes_audio_health_counters() {
        let source = include_str!("service_common.rs");

        for required in [
            "status.audio_midi_drops = audio.midi_drop_count();",
            "status.audio_lock_misses = audio.audio_lock_miss_count();",
            "status.audio_emergency_resets = audio.emergency_reset_count();",
            "status.audio_callback_max_us = audio.callback_max_us();",
            "status.audio_callback_last_us = audio.callback_last_us();",
            "status.audio_callback_over_budget_count = audio.callback_over_budget_count();",
            "status.audio_mmcss_enabled = audio.mmcss_enabled();",
            "status.audio_power_throttling_disabled = audio.power_throttling_disabled();",
        ] {
            assert!(
                source.contains(required),
                "missing status assignment: {required}"
            );
        }
    }
}
