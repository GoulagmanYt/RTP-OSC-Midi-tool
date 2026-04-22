#![allow(deprecated)]

use std::path::{Path, PathBuf};

use rack::{prelude::PluginScanner as _, vst3::Vst3Scanner, PluginInstance as _};
use vst::{
    api::Supported,
    host::{Host, PluginLoader},
    plugin::{CanDo, Plugin},
};

use crate::{
    logger::FrontendLogger,
    plugin_probe::{
        detect_vst3_channels, select_vst3_plugin_info_for_path, vst3_scan_root_for_path,
    },
    vst_scan::{is_vst2_path, is_vst3_path},
};

use super::runtime_state::{AudioError, AudioSettings, PluginBackend};

pub(super) struct LoadedPluginBackend {
    pub(super) backend: PluginBackend,
    pub(super) input_channels: usize,
    pub(super) output_channels: usize,
    pub(super) vst_midi_compatible: bool,
}

pub(super) fn resolve_vst_path(
    settings: &AudioSettings,
    fallback: Option<PathBuf>,
    logger: &FrontendLogger,
) -> Result<PathBuf, AudioError> {
    if let Some(path) = settings.vst_path.as_ref() {
        let source = PathBuf::from(path);
        if let Ok(real) = source.canonicalize() {
            if real.exists() {
                logger.info(format!("Using VST from {:?}", real));
                return Ok(real);
            }
        } else if source.exists() {
            logger.info(format!("Using VST from {:?}", source));
            return Ok(source);
        }
        logger.warn(format!("Configured VST path not found: {:?}", source));
    }

    let mut sources = Vec::new();
    if let Some(fallback) = fallback {
        sources.push(fallback);
    }
    sources.push(PathBuf::from("Bitsonic").join("Keyzone Classic.dll"));

    for source in sources {
        let Ok(real) = source.canonicalize() else {
            continue;
        };
        if !real.exists() {
            continue;
        }
        if is_vst2_path(&real) || is_vst3_path(&real) {
            logger.info(format!("Using VST fallback {:?}", real));
            return Ok(real);
        }
    }

    logger.warn("No valid VST path found");
    Err(AudioError::Message("No VST found".into()))
}

pub(super) fn load_plugin_backend(
    vst_path: &Path,
    sample_rate: u32,
    initial_block_size: u32,
    max_block_size: usize,
    logger: &FrontendLogger,
) -> Result<LoadedPluginBackend, AudioError> {
    if is_vst3_path(vst_path) {
        let (plugin, inputs, outputs) =
            load_vst3_plugin(vst_path, sample_rate, max_block_size, logger)?;
        logger.debug(format!(
            "VST3 block size budget: {} samples (initial stream={})",
            max_block_size, initial_block_size
        ));
        return Ok(LoadedPluginBackend {
            backend: PluginBackend::Vst3 {
                instance: plugin,
                input_channels: inputs,
                output_channels: outputs,
            },
            input_channels: inputs,
            output_channels: outputs,
            vst_midi_compatible: true,
        });
    }

    let host = std::sync::Arc::new(std::sync::Mutex::new(SimpleHost));
    let mut loader = PluginLoader::load(vst_path, host).map_err(|error| {
        AudioError::Message(format!(
            "Failed to load VST plugin: {} ({:?})",
            error, vst_path
        ))
    })?;
    let mut instance = loader
        .instance()
        .map_err(|error| AudioError::Message(format!("Failed to instantiate VST: {}", error)))?;
    instance.init();
    instance.set_sample_rate(sample_rate as f32);
    instance.set_block_size(initial_block_size as i64);
    logger.debug(format!("VST2 block size: {} samples", initial_block_size));

    let info = instance.get_info();
    let input_channels = info.inputs as usize;
    let output_channels = info.outputs as usize;
    logger.debug(format!(
        "VST2 '{}': {} inputs / {} outputs",
        info.name, info.inputs, info.outputs
    ));
    let vst_midi_compatible = matches!(
        instance.can_do(CanDo::ReceiveMidiEvent),
        Supported::Yes | Supported::Maybe
    ) || matches!(
        instance.can_do(CanDo::ReceiveEvents),
        Supported::Yes | Supported::Maybe
    );
    if !vst_midi_compatible {
        logger.warn(format!(
            "VST2 '{}' does not advertise MIDI input support.",
            info.name
        ));
    }

    instance.resume();

    Ok(LoadedPluginBackend {
        backend: PluginBackend::Vst2 { instance },
        input_channels,
        output_channels,
        vst_midi_compatible,
    })
}

fn load_vst3_plugin(
    vst_path: &Path,
    sample_rate: u32,
    max_block_size: usize,
    logger: &FrontendLogger,
) -> Result<(rack::vst3::Vst3Plugin, usize, usize), AudioError> {
    let scanner = Vst3Scanner::new().map_err(|error| {
        AudioError::Message(format!("Failed to initialise VST3 scanner: {}", error))
    })?;
    let scan_root = vst3_scan_root_for_path(vst_path);
    let plugins = scanner
        .scan_path(&scan_root)
        .map_err(|error| AudioError::Message(format!("Failed to scan VST3 plugins: {}", error)))?;
    let info = select_vst3_plugin_info_for_path(&plugins, vst_path).ok_or_else(|| {
        AudioError::Message(format!(
            "VST3 plugin info not found for {:?} (scan root: {:?})",
            vst_path, scan_root
        ))
    })?;

    let mut plugin = scanner
        .load(&info)
        .map_err(|error| AudioError::Message(format!("Failed to load VST3 plugin: {}", error)))?;
    plugin
        .initialize(sample_rate as f64, max_block_size)
        .map_err(|error| {
            AudioError::Message(format!("Failed to initialise VST3 plugin: {}", error))
        })?;

    let (inputs, outputs) =
        detect_vst3_channels(&mut plugin, &info, max_block_size).map_err(|error| {
            logger.warn(format!("VST3 channel probe failed: {error}"));
            AudioError::Message(format!("VST3 channel probe failed: {error}"))
        })?;
    logger.debug(format!(
        "VST3 '{}': {} inputs / {} outputs",
        info.name, inputs, outputs
    ));

    Ok((plugin, inputs, outputs))
}

pub(super) struct SimpleHost;

impl Host for SimpleHost {}
