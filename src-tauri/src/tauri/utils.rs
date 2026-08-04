//! Fonctions utilitaires pour l'interface Tauri.

use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use crate::{config::Config, types::VstPluginEntry};
use directories::ProjectDirs;
use tauri::{AppHandle, Manager};

use crate::tauri::state::AppState;

/// Run a Tauri/Tokio future from synchronous lifecycle code without nesting a
/// runtime on a Tokio worker. Tokio's Runtime::block_on deliberately panics in
/// that situation; an external scoped thread may safely drive the same runtime.
pub fn safe_block_on<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|scope| {
            match scope
                .spawn(move || ::tauri::async_runtime::block_on(future))
                .join()
            {
                Ok(output) => output,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        })
    } else {
        ::tauri::async_runtime::block_on(future)
    }
}

pub fn config_dir_path() -> Result<PathBuf, String> {
    ProjectDirs::from("com", "OSCMIDI", "OSCMIDI")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .ok_or_else(|| "Impossible de determiner le dossier de configuration".to_string())
}

pub fn log_file_path() -> Result<PathBuf, String> {
    Ok(config_dir_path()?.join("app.log"))
}

pub fn read_log_tail(max_bytes: usize) -> String {
    let Ok(path) = log_file_path() else {
        return String::new();
    };
    let Ok(data) = fs::read(&path) else {
        return String::new();
    };
    let slice = if data.len() > max_bytes {
        &data[data.len() - max_bytes..]
    } else {
        &data
    };
    String::from_utf8_lossy(slice).to_string()
}

pub fn vst_cache_path() -> Result<PathBuf, String> {
    Ok(config_dir_path()?.join("vst_cache.json"))
}

pub fn is_instrument_entry(entry: &VstPluginEntry) -> bool {
    entry.kind == "instrument" && entry.supported
}

pub fn retain_instrument_entries(entries: Vec<VstPluginEntry>) -> Vec<VstPluginEntry> {
    entries
        .into_iter()
        .filter(is_instrument_entry)
        .collect::<Vec<_>>()
}

pub fn load_vst_cache_from_disk() -> Option<Vec<VstPluginEntry>> {
    let path = vst_cache_path().ok()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Vec<VstPluginEntry>>(&raw)
        .ok()
        .map(retain_instrument_entries)
}

pub fn save_vst_cache_to_disk(entries: &[VstPluginEntry]) {
    let Ok(path) = vst_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(raw) = serde_json::to_string(entries) else {
        return;
    };
    let _ = fs::write(path, raw);
}

pub fn open_folder_in_explorer(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())
            .map(|_| ())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err("Open folder not supported on this platform".to_string())
    }
}

pub fn sync_runtime_logging(config: &Config, dev_logging: &Arc<std::sync::atomic::AtomicBool>) {
    dev_logging.store(config.logging.verbose, std::sync::atomic::Ordering::Relaxed);
    crate::logger::set_global_dev_mode(config.logging.verbose);
}

pub fn sync_rtp_discovery(config: &Config, state: &AppState, app: &AppHandle) {
    if config.rtp.remote_enabled {
        state.rtp_discovery.start(app.clone());
    } else {
        state.rtp_discovery.stop();
    }
}

pub fn audio_settings_from_config(config: &Config) -> crate::audio::AudioSettings {
    crate::audio::AudioSettings {
        enabled: config.audio.enabled,
        backend: config.audio.backend.clone(),
        device: config.audio.device.clone(),
        sample_rate: config.audio.sample_rate,
        buffer_size: config.audio.buffer_size,
        gain_db: config.audio.gain_db,
        limiter_enabled: config.audio.limiter_enabled,
        vst_path: config.audio.vst_path.clone(),
    }
}

pub fn fallback_vst_path(app: &AppHandle) -> Option<PathBuf> {
    let resolver = app.path();
    resolver
        .resolve("Keyzone Classic.dll", tauri::path::BaseDirectory::Resource)
        .ok()
        .or_else(|| {
            resolver
                .resolve(
                    "Bitsonic/Keyzone Classic.dll",
                    tauri::path::BaseDirectory::Resource,
                )
                .ok()
        })
}

#[cfg(test)]
mod async_tests {
    use super::safe_block_on;

    #[test]
    fn safe_block_on_does_not_nest_the_current_tokio_runtime() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        runtime.block_on(async {
            assert_eq!(safe_block_on(async { 42u32 }), 42);
        });
    }
}
