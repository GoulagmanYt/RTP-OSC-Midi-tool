//! Module audio - moteur audio de production.

mod callback;
mod callback_midi;
mod callback_render;
mod device_selection;
pub mod engine;
mod engine_editor;
mod engine_lifecycle;
mod engine_status;
mod plugin_host;
#[cfg(target_os = "windows")]
mod remote_bridge;
mod runtime_state;
mod state_codec;
mod stream_config;
mod stream_runtime;
mod thread_affinity;
pub mod windows_tuning;

pub use engine::AudioEngine;
#[cfg(target_os = "windows")]
pub use remote_bridge::run_dsp_bridge_from_env;
pub use runtime_state::{AudioError, AudioSettings};

#[doc(hidden)]
pub fn restore_vst3_state_file_for_diagnostic(
    instance: &mut rack::vst3::Vst3Plugin,
    entry: &crate::types::VstPluginEntry,
    path: &std::path::Path,
) -> Result<usize, String> {
    state_codec::restore_vst3_state_file_for_diagnostic(instance, entry, path)
}
