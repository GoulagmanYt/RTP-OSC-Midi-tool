//! Facade application services re-exported by the Tauri command layer.

#[path = "service_audio.rs"]
mod service_audio;
#[path = "service_common.rs"]
mod service_common;
#[path = "service_config.rs"]
mod service_config;
#[path = "service_diagnostics.rs"]
mod service_diagnostics;
#[path = "service_paths.rs"]
mod service_paths;
#[path = "service_runtime.rs"]
mod service_runtime;

pub use service_audio::{
    close_vst_ui, list_audio_backends, list_audio_devices, list_vst_parameters, list_vst_plugins,
    open_vst_ui, ping_audio, refresh_vst_plugins, reload_vst, set_audio_limiter, set_master_gain,
    set_vst_parameter,
};
pub use service_config::{
    export_config, get_config, import_config, reset_config_defaults, save_config,
};
pub use service_diagnostics::export_diagnostics;
pub use service_paths::{clear_log_file, get_app_paths, open_app_dir};
pub use service_runtime::{
    get_status, list_midi_inputs, list_midi_outputs, panic_midi, preflight_check,
    refresh_rtp_sessions, restart_rtp, run_stress_test, send_test_midi, start_bridge, stop_bridge,
};
