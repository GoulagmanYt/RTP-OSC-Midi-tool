#![allow(deprecated)]

use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use cpal::{
    traits::DeviceTrait, BufferSize, Device, SampleFormat, SampleRate, Stream, StreamConfig,
};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use rtrb::{Producer, RingBuffer};

use crate::logger::FrontendLogger;
use crate::types::VstParameter;
use crate::types::VstPluginEntry;

use super::{
    callback::build_output_stream_for_sample,
    plugin_host::load_plugin_backend_thread_affine,
    runtime_state::{
        AudioControls, AudioError, AudioTelemetry, MidiPacket, ParameterCommand, PluginBackend,
        MIDI_RING_CAPACITY, PARAMETER_COMMAND_CAPACITY,
    },
    stream_config::{choose_buffer_size, max_plugin_block_size, select_output_config},
};

pub(super) type BuiltStream = (
    Stream,
    Arc<Mutex<PluginBackend>>,
    Producer<MidiPacket>,
    u32,
    Option<u32>,
    bool,
    u32,
    Sender<ParameterCommand>,
    Arc<Mutex<Vec<VstParameter>>>,
);

#[allow(clippy::too_many_arguments)]
pub(super) fn build_stream(
    device: &Device,
    sample_rate: u32,
    buffer_size: u32,
    controls: Arc<AudioControls>,
    telemetry: Arc<AudioTelemetry>,
    emergency_reset_requested: Arc<AtomicBool>,
    vst_path: PathBuf,
    logger: &FrontendLogger,
    backend_name: &str,
    device_name: &str,
    prefer_low_latency: bool,
    vst_probe: &VstPluginEntry,
) -> Result<BuiltStream, AudioError> {
    let supported: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| AudioError::Message(e.to_string()))?
        .filter(|cfg| cfg.channels() >= 2)
        .collect();

    if supported.is_empty() {
        return Err(AudioError::Message(
            "Device does not support stereo output, which is required for the VST plugin.".into(),
        ));
    }

    let (desired, actual_sample_rate) = select_output_config(&supported, sample_rate)?;
    if actual_sample_rate != sample_rate {
        logger.warn(format!(
            "Requested sample rate {} Hz not supported on '{}'. Using {} Hz instead.",
            sample_rate, device_name, actual_sample_rate
        ));
    }

    let supported_buffer_size = desired.buffer_size();
    logger.debug(format!(
        "Device supported buffer size range: {:?}",
        supported_buffer_size
    ));
    let plugin_max_block_size = max_plugin_block_size(supported_buffer_size, buffer_size);

    let mut config: StreamConfig = desired
        .with_sample_rate(SampleRate(actual_sample_rate))
        .config();
    let mut target_buffer_size = choose_buffer_size(supported_buffer_size, buffer_size);
    if backend_name.to_lowercase().contains("wasapi") {
        target_buffer_size = choose_buffer_size(supported_buffer_size, target_buffer_size.max(512));
    }
    if target_buffer_size != buffer_size {
        logger.warn(format!(
            "Requested buffer size {} not supported on '{}'. Using {} instead.",
            buffer_size, device_name, target_buffer_size
        ));
    }
    config.buffer_size = BufferSize::Fixed(target_buffer_size);

    logger.info(format!(
        "Building stream on '{}': backend={}, format={:?}, rate={}, buffer={:?} (requested={}, low-latency={}), channels={}",
        device_name,
        backend_name,
        desired.sample_format(),
        actual_sample_rate,
        config.buffer_size,
        buffer_size,
        prefer_low_latency,
        config.channels
    ));

    let channels = config.channels as usize;

    let initial_block_size = match config.buffer_size {
        BufferSize::Fixed(sz) => {
            telemetry
                .block_size_frames
                .store(sz, std::sync::atomic::Ordering::Relaxed);
            sz
        }
        BufferSize::Default => {
            telemetry
                .block_size_frames
                .store(0, std::sync::atomic::Ordering::Relaxed);
            buffer_size
        }
    };

    let loaded_plugin = load_plugin_backend_thread_affine(
        &vst_path,
        actual_sample_rate,
        initial_block_size,
        plugin_max_block_size as usize,
        logger,
        Some(vst_probe),
    )?;
    let plugin_inputs = loaded_plugin.input_channels;
    let plugin_outputs = loaded_plugin.output_channels;
    let vst_midi_compatible = loaded_plugin.vst_midi_compatible;
    let plugin_latency_samples = loaded_plugin.latency_samples;

    let parameter_cache = Arc::new(Mutex::new(loaded_plugin.parameter_cache));
    let plugin = Arc::new(Mutex::new(loaded_plugin.backend));
    let (parameter_tx, parameter_rx) = crossbeam_channel::bounded(PARAMETER_COMMAND_CAPACITY);

    let try_build_stream =
        |cfg: &StreamConfig| -> Result<(Stream, Producer<MidiPacket>), AudioError> {
            let (midi_tx, midi_rx) = RingBuffer::new(MIDI_RING_CAPACITY);

            let stream = match desired.sample_format() {
                SampleFormat::F32 => build_output_stream_for_sample::<f32>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    Arc::clone(&controls),
                    Arc::clone(&telemetry),
                    midi_rx,
                    plugin.clone(),
                    emergency_reset_requested.clone(),
                    actual_sample_rate,
                    plugin_max_block_size as usize,
                    device_name,
                    parameter_rx.clone(),
                )?,
                SampleFormat::I16 => build_output_stream_for_sample::<i16>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    Arc::clone(&controls),
                    Arc::clone(&telemetry),
                    midi_rx,
                    plugin.clone(),
                    emergency_reset_requested.clone(),
                    actual_sample_rate,
                    plugin_max_block_size as usize,
                    device_name,
                    parameter_rx.clone(),
                )?,
                SampleFormat::U16 => build_output_stream_for_sample::<u16>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    Arc::clone(&controls),
                    Arc::clone(&telemetry),
                    midi_rx,
                    plugin.clone(),
                    emergency_reset_requested.clone(),
                    actual_sample_rate,
                    plugin_max_block_size as usize,
                    device_name,
                    parameter_rx.clone(),
                )?,
                other => {
                    return Err(AudioError::Message(format!(
                        "Unsupported sample format: {other:?}"
                    )))
                }
            };

            Ok((stream, midi_tx))
        };

    match try_build_stream(&config) {
        Ok((stream, midi_tx)) => Ok((
            stream,
            plugin,
            midi_tx,
            actual_sample_rate,
            Some(target_buffer_size),
            vst_midi_compatible,
            plugin_latency_samples,
            parameter_tx.clone(),
            parameter_cache.clone(),
        )),
        Err(primary_err) => {
            if prefer_low_latency {
                for fallback in super::stream_config::buffer_fallback_candidates(
                    supported_buffer_size,
                    target_buffer_size,
                ) {
                    if fallback == target_buffer_size {
                        continue;
                    }
                    logger.warn(format!(
                        "Retrying '{}' with safer buffer size {} samples after startup failure: {}",
                        device_name, fallback, primary_err
                    ));
                    let mut fallback_cfg = config.clone();
                    fallback_cfg.buffer_size = BufferSize::Fixed(fallback);
                    if let Ok((stream, midi_tx)) = try_build_stream(&fallback_cfg) {
                        return Ok((
                            stream,
                            plugin,
                            midi_tx,
                            actual_sample_rate,
                            Some(fallback),
                            vst_midi_compatible,
                            plugin_latency_samples,
                            parameter_tx.clone(),
                            parameter_cache.clone(),
                        ));
                    }
                }
            }
            Err(primary_err)
        }
    }
}
