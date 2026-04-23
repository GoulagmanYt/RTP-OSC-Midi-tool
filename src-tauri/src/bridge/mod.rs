//! Module bridge - runtime de production.

mod activity;
mod lifecycle;
mod metrics;
mod midi_io;
pub mod pipeline;
mod processing;
mod rtp_config;
pub mod runtime;
mod status;

pub use runtime::BridgeHandle;
pub use pipeline::{pipeline_stats, reset_pipeline_stats};
