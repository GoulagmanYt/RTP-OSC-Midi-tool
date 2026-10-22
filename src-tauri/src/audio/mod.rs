//! Module audio - moteur audio de production.

mod callback;
mod callback_gate;
mod callback_midi;
mod callback_render;
mod device_selection;
pub mod engine;
mod engine_editor;
mod engine_lifecycle;
mod engine_status;
mod equalizer;
mod game_mode;
mod midi_playback;
mod plugin_host;
#[cfg(target_os = "windows")]
mod remote_bridge;
mod render_ahead;
mod runtime_state;
mod state_codec;
mod stream_config;
mod stream_runtime;
mod thread_affinity;
pub mod windows_tuning;

pub use engine::AudioEngine;
pub(crate) use engine::MidiSendOutcome;
pub use plugin_host::MAX_CACHED_PARAMETERS;
#[cfg(target_os = "windows")]
pub use remote_bridge::run_dsp_bridge_from_env;
pub(crate) use runtime_state::{default_render_ahead_blocks, deserialize_render_ahead_blocks};
pub use runtime_state::{AudioError, AudioSettings, DspThreadPriority};

#[doc(hidden)]
pub fn restore_vst3_state_file_for_diagnostic(
    instance: &mut rack::vst3::Vst3Plugin,
    entry: &crate::types::VstPluginEntry,
    path: &std::path::Path,
) -> Result<usize, String> {
    state_codec::restore_vst3_state_file_for_diagnostic(instance, entry, path)
}
