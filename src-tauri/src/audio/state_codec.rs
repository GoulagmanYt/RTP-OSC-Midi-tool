#![allow(deprecated)]

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use directories::ProjectDirs;
use parking_lot::Mutex;
use rack::PluginInstance as _;
use vst::host::PluginInstance;
use vst::plugin::Plugin;

use crate::logger::{background_log, FrontendLogger};
use crate::types::VstPluginEntry;

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

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn legacy_state_path_for_plugin(vst_path: &Path) -> Option<PathBuf> {
    let file_name = vst_path.file_name()?.to_string_lossy().to_string();
    let base_dir = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
        .map(|dirs| dirs.config_dir().join("vst_state"))?;
    Some(base_dir.join(format!("{}.state", file_name)))
}

fn stable_plugin_identity(vst_path: &Path, include_uid: bool) -> String {
    let canonical = vst_path
        .canonicalize()
        .unwrap_or_else(|_| vst_path.to_path_buf());
    let format = if crate::vst_scan::is_vst3_path(vst_path) {
        "vst3"
    } else {
        "vst2"
    };
    let base = format!("{}:{}", format, canonical.to_string_lossy().to_lowercase());
    if include_uid {
        if let Some(uid) = cached_plugin_uid(vst_path) {
            return format!("{base}:{uid}");
        }
    }
    base
}

fn cached_plugin_uid(vst_path: &Path) -> Option<String> {
    let cache_path = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")?
        .config_dir()
        .join("vst_cache.json");
    let entries: Vec<VstPluginEntry> = serde_json::from_slice(&fs::read(cache_path).ok()?).ok()?;
    let target = vst_path
        .canonicalize()
        .unwrap_or_else(|_| vst_path.to_path_buf())
        .to_string_lossy()
        .to_lowercase();
    entries.into_iter().find_map(|entry| {
        let candidate = PathBuf::from(&entry.path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&entry.path))
            .to_string_lossy()
            .to_lowercase();
        (candidate == target).then_some(entry.class_uid).flatten()
    })
}

fn legacy_state_is_unambiguous(vst_path: &Path) -> bool {
    let Some(file_name) = vst_path.file_name() else {
        return false;
    };
    let Some(cache_path) = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
        .map(|dirs| dirs.config_dir().join("vst_cache.json"))
    else {
        return false;
    };
    let Ok(raw) = fs::read(cache_path) else {
        return false;
    };
    let Ok(entries) = serde_json::from_slice::<Vec<VstPluginEntry>>(&raw) else {
        return false;
    };
    entries
        .iter()
        .filter(|entry| Path::new(&entry.path).file_name() == Some(file_name))
        .count()
        == 1
}

fn state_path_for_identity(vst_path: &Path, include_uid: bool) -> Option<PathBuf> {
    let file_name = vst_path.file_name()?.to_string_lossy().to_string();
    let base_dir = ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
        .map(|dirs| dirs.config_dir().join("vst_state"))?;
    let hash = fnv1a64(stable_plugin_identity(vst_path, include_uid).as_bytes());
    Some(base_dir.join(format!("{}-{hash:016x}.state", file_name)))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(super) fn state_path_for_plugin(vst_path: &Path) -> Option<PathBuf> {
    state_path_for_identity(vst_path, true)
}

fn write_state_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "state path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        sequence
    ));
    let mut temp = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)?;
    if let Err(error) = temp.write_all(data).and_then(|_| temp.sync_all()) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    drop(temp);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let temp_wide: Vec<u16> = temp_path.as_os_str().encode_wide().chain(Some(0)).collect();
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let result = unsafe {
            MoveFileExW(
                PCWSTR(temp_wide.as_ptr()),
                PCWSTR(path_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if let Err(error) = result {
            let _ = fs::remove_file(&temp_path);
            return Err(std::io::Error::other(error));
        }
    }

    #[cfg(not(target_os = "windows"))]
    fs::rename(&temp_path, path)?;

    Ok(())
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

pub(super) fn save_vst_state_blocking(plugin: &Arc<Mutex<PluginBackend>>, vst_path: &Path) {
    if is_sforzando_vst3(vst_path) {
        return;
    }

    let Some(state_path) = state_path_for_plugin(vst_path) else {
        background_log("warn", "Could not determine VST state path");
        return;
    };
    // The stream is stopped before this function is called. Capture plugin-owned
    // state under the lock, then release it before any filesystem operation.
    let captured = match &mut *plugin.lock() {
        PluginBackend::Vst2 { instance } => {
            let params = instance.get_parameter_object();
            let bank = params.get_bank_data();
            let preset = params.get_preset_data();
            let chunk = if !bank.is_empty() { bank } else { preset };

            if !chunk.is_empty() {
                Some((encode_state_chunk(&chunk), "chunk".to_string()))
            } else {
                let values = capture_vst2_parameters(instance);
                if values.is_empty() {
                    None
                } else {
                    let description = format!("params: {}", values.len());
                    Some((encode_state_params(&values), description))
                }
            }
        }
        PluginBackend::Vst3 { instance, .. } => match instance.get_state() {
            Ok(data) => {
                if data.is_empty() {
                    None
                } else {
                    Some((encode_state_chunk(&data), "VST3 chunk".to_string()))
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
                None
            }
        },
    };

    let Some((payload, description)) = captured else {
        background_log(
            "debug",
            "VST state not saved (plugin returned no persistent state)",
        );
        return;
    };
    if let Err(error) = write_state_atomically(&state_path, &payload) {
        background_log("error", format!("Failed to save VST state: {error}"));
    } else {
        background_log(
            "info",
            format!(
                "VST state saved atomically to {:?} ({description})",
                state_path
            ),
        );
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

    let Some(new_state_path) = state_path_for_plugin(vst_path) else {
        logger.warn("Could not determine VST state path");
        return;
    };
    let legacy_path = legacy_state_path_for_plugin(vst_path);
    let path_identity_path = state_path_for_identity(vst_path, false);
    let (state_path, migrate_legacy) = if new_state_path.exists() {
        (new_state_path.clone(), false)
    } else if let Some(path) = path_identity_path.filter(|path| path.exists()) {
        (path, true)
    } else if let Some(path) =
        legacy_path.filter(|path| path.exists() && legacy_state_is_unambiguous(vst_path))
    {
        (path, true)
    } else {
        logger.debug("No saved VST state found");
        return;
    };

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
            drop(guard);
            if migrate_legacy {
                if let Err(error) = write_state_atomically(&new_state_path, &data) {
                    logger.warn(format!("Failed to migrate legacy VST state: {error}"));
                } else {
                    logger.info(format!("Migrated legacy VST state to {:?}", new_state_path));
                }
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
