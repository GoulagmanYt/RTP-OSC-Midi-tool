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
mod runtime_state;
mod state_codec;
mod stream_config;
mod stream_runtime;
mod thread_affinity;
pub mod windows_tuning;

pub use engine::AudioEngine;
pub use runtime_state::{AudioError, AudioSettings};
