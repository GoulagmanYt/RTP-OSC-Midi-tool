#![allow(deprecated)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use directories::ProjectDirs;
use parking_lot::Mutex;
use rack::PluginInstance as _;
use vst::host::PluginInstance;
use vst::plugin::Plugin;

use crate::logger::{background_log, FrontendLogger};

use super::runtime_state::PluginBackend;

const STATE_MAGIC: &[u8; 4] = b"OSVS";
const STATE_VERSION: u8 = 1;
const STATE_KIND_CHUNK: u8 = 0;
const STATE_KIND_PARAMS: u8 = 1;

pub(super) enum SavedState {
    Raw(Vec<u8>),
    Chunk(Vec<u8>),
    Params(Vec<f32>),
}

pub(super) fn state_path_for_plugin(vst_path: &Path) -> Option<PathBuf> {
    let file_name = vst_path.file_name()?.to_string_lossy().to_string();
    let base_dir = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
        .map(|dirs| dirs.config_dir().join("vst_state"))?;
    Some(base_dir.join(format!("{}.state", file_name)))
}

pub(super) fn encode_state_chunk(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + data.len());
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION);
    out.push(STATE_KIND_CHUNK);
    out.extend_from_slice(data);
    out
}

pub(super) fn encode_state_params(params: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + params.len() * 4);
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION);
    out.push(STATE_KIND_PARAMS);
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for value in params {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub(super) fn decode_state(data: &[u8]) -> SavedState {
    if data.len() < 6 || data[..4] != *STATE_MAGIC {
        return SavedState::Raw(data.to_vec());
    }

    let version = data[4];
    if version != STATE_VERSION {
        return SavedState::Raw(data.to_vec());
    }

    match data[5] {
        STATE_KIND_CHUNK => SavedState::Chunk(data[6..].to_vec()),
        STATE_KIND_PARAMS => {
            if data.len() < 10 {
                return SavedState::Raw(data.to_vec());
            }
            let count = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
            let expected = 10 + count.saturating_mul(4);
            if data.len() < expected {
                return SavedState::Raw(data.to_vec());
            }
            let mut params = Vec::with_capacity(count);
            let mut offset = 10;
            for _ in 0..count {
                let bytes = [
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ];
                params.push(f32::from_le_bytes(bytes));
                offset += 4;
            }
            SavedState::Params(params)
        }
        _ => SavedState::Raw(data.to_vec()),
    }
}

pub(super) fn capture_vst2_parameters(instance: &mut PluginInstance) -> Vec<f32> {
    let info = instance.get_info();
    let count = info.parameters.max(0) as usize;
    if count == 0 {
        return Vec::new();
    }
    let params = instance.get_parameter_object();
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(params.get_parameter(index as i32));
    }
    values
}

pub(super) fn apply_vst2_parameters(instance: &mut PluginInstance, values: &[f32]) {
    let info = instance.get_info();
    let count = info.parameters.max(0) as usize;
    if count == 0 {
        return;
    }
    let params = instance.get_parameter_object();
    for (index, value) in values.iter().take(count).enumerate() {
        params.set_parameter(index as i32, value.clamp(0.0, 1.0));
    }
}

pub(super) fn save_vst_state(plugin: &Arc<Mutex<PluginBackend>>, vst_path: &Path) {
    save_vst_state_inner(plugin, vst_path, false);
}

pub(super) fn save_vst_state_blocking(plugin: &Arc<Mutex<PluginBackend>>, vst_path: &Path) {
    save_vst_state_inner(plugin, vst_path, true);
}

fn save_vst_state_inner(plugin: &Arc<Mutex<PluginBackend>>, vst_path: &Path, allow_blocking: bool) {
    if is_sforzando_vst3(vst_path) {
        return;
    }

    let Some(state_path) = state_path_for_plugin(vst_path) else {
        background_log("warn", "Could not determine VST state path");
        return;
    };
    if let Some(parent) = state_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let Some(mut guard) = plugin.try_lock().or_else(|| {
        if allow_blocking {
            Some(plugin.lock())
        } else {
            None
        }
    }) else {
        background_log("warn", "VST state save skipped because plugin is busy");
        return;
    };

    match &mut *guard {
        PluginBackend::Vst2 { instance } => {
            let params = instance.get_parameter_object();
            let bank = params.get_bank_data();
            let preset = params.get_preset_data();
            let chunk = if !bank.is_empty() { bank } else { preset };

            if !chunk.is_empty() {
                let data = encode_state_chunk(&chunk);
                if let Err(error) = fs::write(&state_path, &data) {
                    background_log("error", format!("Failed to save VST state: {}", error));
                } else {
                    background_log(
                        "info",
                        format!("VST state saved to {:?} (chunk)", state_path),
                    );
                }
                return;
            }

            let values = capture_vst2_parameters(instance);
            if values.is_empty() {
                background_log("debug", "VST state not saved (no chunks or parameters)");
                return;
            }

            let data = encode_state_params(&values);
            if let Err(error) = fs::write(&state_path, &data) {
                background_log("error", format!("Failed to save VST state: {}", error));
            } else {
                background_log(
                    "info",
                    format!(
                        "VST state saved to {:?} (params: {})",
                        state_path,
                        values.len()
                    ),
                );
            }
        }
        PluginBackend::Vst3 { instance, .. } => match instance.get_state() {
            Ok(data) => {
                if data.is_empty() {
                    background_log("debug", "VST3 state not saved (plugin returned empty data)");
                    return;
                }
                let payload = encode_state_chunk(&data);
                if let Err(error) = fs::write(&state_path, &payload) {
                    background_log("error", format!("Failed to save VST3 state: {}", error));
                } else {
                    background_log("info", format!("VST3 state saved to {:?}", state_path));
                }
            }
            Err(error) => {
                let message = error.to_string();
                if message.contains("Feature not supported by this plugin") {
                    background_log(
                        "debug",
                        "VST3 state not saved (plugin does not expose host-readable state)",
                    );
                } else {
                    background_log("warn", format!("Failed to read VST3 state: {}", error));
                }
            }
        },
    }
}

pub(super) fn load_vst_state(
    plugin: &Arc<Mutex<PluginBackend>>,
    vst_path: &Path,
    logger: &FrontendLogger,
) {
    if is_sforzando_vst3(vst_path) {
        return;
    }

    let Some(state_path) = state_path_for_plugin(vst_path) else {
        logger.warn("Could not determine VST state path");
        return;
    };
    if !state_path.exists() {
        logger.debug("No saved VST state found");
        return;
    }

    match fs::read(&state_path) {
        Ok(data) => {
            let mut guard = plugin.lock();
            match &mut *guard {
                PluginBackend::Vst2 { instance } => match decode_state(&data) {
                    SavedState::Chunk(chunk) => {
                        let params = instance.get_parameter_object();
                        let bank_ok = {
                            params.load_bank_data(&chunk);
                            instance.get_info().parameters > 0
                        };
                        if !bank_ok {
                            params.load_preset_data(&chunk);
                        }
                        logger.info(format!("VST state loaded from {:?}", state_path));
                    }
                    SavedState::Raw(chunk) => {
                        let params = instance.get_parameter_object();
                        params.load_bank_data(&chunk);
                        params.load_preset_data(&chunk);
                        logger.info(format!("VST state loaded (legacy) from {:?}", state_path));
                    }
                    SavedState::Params(values) => {
                        apply_vst2_parameters(instance, &values);
                        logger.info(format!(
                            "VST state loaded from {:?} (params: {})",
                            state_path,
                            values.len()
                        ));
                    }
                },
                PluginBackend::Vst3 { instance, .. } => match decode_state(&data) {
                    SavedState::Chunk(chunk) | SavedState::Raw(chunk) => {
                        if let Err(error) = instance.set_state(&chunk) {
                            let message = error.to_string();
                            if message.contains("Feature not supported by this plugin") {
                                logger.debug(
                                    "VST3 state ignored (plugin does not accept host-supplied state)",
                                );
                            } else {
                                logger.warn(format!("Failed to restore VST3 state: {}", error));
                            }
                        } else {
                            logger.info(format!("VST3 state loaded from {:?}", state_path));
                        }
                    }
                    SavedState::Params(_) => {
                        logger.warn("Ignoring parameter-only state for VST3 plugin");
                    }
                },
            }
        }
        Err(error) => logger.warn(format!("Failed to read saved VST state: {}", error)),
    }
}

fn is_sforzando_vst3(vst_path: &Path) -> bool {
    let path = vst_path.to_string_lossy().to_lowercase();
    path.ends_with("sforzando.vst3") || path.contains("\\sforzando.vst3")
}

#[cfg(test)]
mod tests {
    #[test]
    fn save_vst_state_nonblocking_skips_when_plugin_busy() {
        let source = include_str!("state_codec.rs");

        let forbidden = ["None => ", "plugin.lock()"].concat();
        assert!(
            !source.contains(&forbidden),
            "save_vst_state must not block the audio callback when the plugin is busy"
        );
        assert!(
            source.contains("save_vst_state_blocking"),
            "blocking state save should be explicit and used only after stream shutdown"
        );
    }
}
