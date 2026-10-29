use crate::{audio::AudioEngine, config::Config, types::BridgeStatus};

use super::runtime::BridgeRuntime;

pub(super) fn build_initial_status(
    config: &Config,
    audio: &AudioEngine,
    rtp_requested: bool,
) -> BridgeStatus {
    BridgeStatus {
        running: true,
        midi_input: config.midi.input_device.clone(),
        midi_output: config.midi.output_device.clone(),
        osc_target: format!("{}:{}", config.osc.target_ip, config.osc.target_port),
        rtp_active: rtp_requested,
        rtp_bound_port: None,
        rtp_advertised_host: None,
        rtp_advertised_addresses: Vec::new(),
        rtp_network_warning: None,
        last_error: None,
        audio_running: audio.is_running(),
        audio_latency_ms: audio.current_latency_ms(),
        audio_buffer_period_ms: audio.audio_buffer_period_ms(),
        audio_backend_latency_ms: audio.backend_latency_ms(),
        plugin_latency_samples: audio.plugin_latency_samples(),
        bridge_latency_samples: audio.bridge_latency_samples(),
        vst_hosting_mode: audio.vst_hosting_mode(),
        audio_backend: audio.current_backend(),
        audio_device: audio.current_device(),
        audio_device_id: audio.current_device_id(),
        audio_sample_rate: audio.current_sample_rate(),
        audio_buffer_size: audio.current_buffer_size(),
        audio_requested_buffer_size: audio.requested_buffer_size(),
        audio_stream_buffer_size: audio.stream_buffer_size(),
        audio_buffer_mismatch: audio.buffer_size_mismatch(),
        vst_loaded: audio.is_running(),
        vst_midi_compatible: audio.vst_midi_compatible(),
        audio_xruns: audio.xrun_count(),
        audio_stream_recovery_requests: audio.stream_recovery_requests(),
        audio_stream_route_changes: audio.stream_route_changes(),
        audio_midi_drops: audio.midi_drop_count(),
        audio_lock_misses: audio.audio_lock_miss_count(),
        audio_emergency_resets: audio.emergency_reset_count(),
        audio_callback_max_us: audio.callback_max_us(),
        audio_callback_last_us: audio.callback_last_us(),
        audio_callback_over_budget_count: audio.callback_over_budget_count(),
        consecutive_deadline_misses: audio.consecutive_deadline_misses(),
        dsp_process_last_us: audio.dsp_process_last_us(),
        dsp_process_p95_us: audio.dsp_process_percentile_us(95),
        dsp_process_p99_us: audio.dsp_process_percentile_us(99),
        dsp_process_max_us: audio.dsp_process_max_us(),
        audio_midi_queue_depth: audio.audio_midi_queue_depth(),
        audio_midi_queue_max_depth: audio.audio_midi_queue_max_depth(),
        audio_midi_oldest_us: audio.audio_midi_oldest_us(),
        audio_lifecycle_state: audio.lifecycle_state().to_string(),
        vst_worker_state: "disabled".to_string(),
        vst_worker_restarts: 0,
        vst_worker_last_exit: None,
        realtime: audio.realtime_diagnostics(),
        audio_mmcss_enabled: audio.mmcss_enabled(),
        audio_power_throttling_disabled: audio.power_throttling_disabled(),
        audio_limiter_enabled: audio.limiter_enabled(),
        x86_bridge_underruns: Some(audio.x86_bridge_metrics().0),
        x86_bridge_overruns: Some(audio.x86_bridge_metrics().1),
        x86_worker_alive: Some(audio.x86_bridge_metrics().2),
    }
}

pub(super) fn refresh_runtime_status(
    runtime: &mut BridgeRuntime,
    previous: &Config,
    config: &Config,
    rtp_active: bool,
    rtp_bound_port: Option<u16>,
) {
    // Connection owners publish actual physical ports only when they reconnect.
    // An unrelated save must not erase their still-valid connection status.
    if previous.midi.input_device != config.midi.input_device
        || previous.midi.hotplug != config.midi.hotplug
    {
        *runtime.actual_midi_in.lock() = super::midi_io::initial_connected_input(config);
    }
    if previous.midi.output_device != config.midi.output_device {
        *runtime.actual_midi_out.lock() = super::midi_io::initial_connected_output(config);
    }
    runtime.status.midi_input = runtime.actual_midi_in.lock().clone();
    runtime.status.midi_output = runtime.actual_midi_out.lock().clone();
    runtime.status.osc_target = format!("{}:{}", config.osc.target_ip, config.osc.target_port);
    runtime.status.rtp_active = rtp_active;
    runtime.status.rtp_bound_port = if rtp_active { rtp_bound_port } else { None };
    if !rtp_active {
        runtime.status.rtp_advertised_host = None;
        runtime.status.rtp_advertised_addresses.clear();
        runtime.status.rtp_network_warning = None;
    }
}

pub(super) fn enrich_status_from_runtime(status: &mut BridgeStatus, runtime: &BridgeRuntime) {
    status.midi_input = runtime.actual_midi_in.lock().clone();
    status.midi_output = runtime.actual_midi_out.lock().clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::{atomic::AtomicBool, atomic::AtomicU64, Arc};

    fn connected_runtime(config: &Config) -> BridgeRuntime {
        BridgeRuntime {
            status: BridgeStatus::from(config),
            stop: Arc::new(AtomicBool::new(false)),
            processing: None,
            midi_watcher: None,
            activity_emitter: None,
            midi_event_emitter: None,
            reliable_playback: None,
            osc_input: None,
            config: Arc::new(Mutex::new(config.clone())),
            config_rev: Arc::new(AtomicU64::new(1)),
            actual_midi_in: Arc::new(Mutex::new(config.midi.input_device.clone())),
            actual_midi_out: Arc::new(Mutex::new(config.midi.output_device.clone())),
        }
    }

    fn physical_config() -> Config {
        let mut config = Config::default();
        config.midi.input_device = Some("Keyboard".into());
        config.midi.output_device = Some("Synth".into());
        config
    }

    #[test]
    fn unrelated_settings_preserve_actual_physical_connection_status() {
        let previous = physical_config();
        for connected in [true, false] {
            let mut runtime = connected_runtime(&previous);
            if !connected {
                *runtime.actual_midi_in.lock() = None;
                *runtime.actual_midi_out.lock() = None;
            }
            let mut next = previous.clone();
            next.ui.corner_radius = 6.0;
            *runtime.config.lock() = next.clone();
            refresh_runtime_status(&mut runtime, &previous, &next, true, Some(5004));
            assert_eq!(runtime.status.midi_input.is_some(), connected);
            assert_eq!(runtime.status.midi_output.is_some(), connected);
        }
    }

    #[test]
    fn selecting_a_new_port_invalidates_only_its_own_connection_status() {
        let previous = physical_config();
        for change_input in [true, false] {
            let mut runtime = connected_runtime(&previous);
            let mut next = previous.clone();
            if change_input {
                next.midi.input_device = Some("Other keyboard".into());
            } else {
                next.midi.output_device = Some("Other synth".into());
            }
            *runtime.config.lock() = next.clone();
            refresh_runtime_status(&mut runtime, &previous, &next, true, Some(5004));
            assert_eq!(runtime.status.midi_input.is_some(), !change_input);
            assert_eq!(runtime.status.midi_output.is_some(), change_input);
        }
    }
}
