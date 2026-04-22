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
        last_error: None,
        audio_running: audio.is_running(),
        audio_latency_ms: audio.current_latency_ms(),
        audio_backend: audio.current_backend(),
        audio_device: audio.current_device(),
        audio_sample_rate: audio.current_sample_rate(),
        audio_buffer_size: audio.current_buffer_size(),
        audio_requested_buffer_size: audio.requested_buffer_size(),
        audio_stream_buffer_size: audio.stream_buffer_size(),
        audio_buffer_mismatch: audio.buffer_size_mismatch(),
        vst_loaded: audio.is_running(),
        vst_midi_compatible: audio.vst_midi_compatible(),
        audio_xruns: audio.xrun_count(),
        audio_limiter_enabled: audio.limiter_enabled(),
    }
}

pub(super) fn refresh_runtime_status(
    runtime: &mut BridgeRuntime,
    config: &Config,
    rtp_active: bool,
    rtp_bound_port: Option<u16>,
) {
    *runtime.actual_midi_in.lock() = super::midi_io::initial_connected_input(config);
    *runtime.actual_midi_out.lock() = super::midi_io::initial_connected_output(config);
    runtime.status.midi_input = runtime.actual_midi_in.lock().clone();
    runtime.status.midi_output = runtime.actual_midi_out.lock().clone();
    runtime.status.osc_target = format!("{}:{}", config.osc.target_ip, config.osc.target_port);
    runtime.status.rtp_active = rtp_active;
    runtime.status.rtp_bound_port = rtp_bound_port;
}

pub(super) fn enrich_status_from_runtime(status: &mut BridgeStatus, runtime: &BridgeRuntime) {
    status.midi_input = runtime.actual_midi_in.lock().clone();
    status.midi_output = runtime.actual_midi_out.lock().clone();
}
