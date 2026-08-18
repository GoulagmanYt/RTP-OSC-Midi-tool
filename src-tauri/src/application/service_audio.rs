use tauri::{AppHandle, Window};

use crate::{
    error::CommandError,
    plugin_probe::probe_plugins_isolated,
    tauri::{
        state::AppState,
        utils::{
            audio_settings_from_config, fallback_vst_path, retain_instrument_entries,
            save_vst_cache_to_disk,
        },
    },
    types::{BridgeStatus, VstParameter, VstPluginEntry},
    vst_scan::{default_vst_scan_roots, scan_vst_plugins_in_roots_cached},
};

use super::service_common::{
    audio_error, frontend_logger, persist_resolved_audio_device, with_audio_status,
};

pub fn list_audio_backends(state: &AppState) -> Vec<String> {
    state.audio.list_backends()
}

pub fn list_audio_devices(backend: Option<String>, state: &AppState) -> Vec<String> {
    state.audio.list_devices(backend)
}

pub fn list_vst_plugins(state: &AppState) -> Vec<VstPluginEntry> {
    state
        .vst_cache
        .lock()
        .clone()
        .map(retain_instrument_entries)
        .unwrap_or_default()
}

pub fn refresh_vst_plugins(state: &AppState) -> Vec<VstPluginEntry> {
    let mut roots = default_vst_scan_roots();
    let cfg = state.config_store.load();
    for custom in &cfg.audio.vst_scan_paths {
        let path = std::path::PathBuf::from(custom);
        if path.exists() && !roots.iter().any(|root| root == &path) {
            roots.push(path);
        }
    }
    if roots.is_empty() {
        return state
            .vst_cache
            .lock()
            .clone()
            .map(retain_instrument_entries)
            .unwrap_or_default();
    }

    let cached = state.vst_cache.lock().clone().unwrap_or_default();
    let plugins =
        retain_instrument_entries(scan_vst_plugins_in_roots_cached(&roots, &cached, false));
    *state.vst_cache.lock() = Some(plugins.clone());
    save_vst_cache_to_disk(&plugins);
    if cfg.audio.vst_plugin_id.is_none() {
        if let Some(path) = cfg.audio.vst_path.as_deref() {
            if let Some(entry) = plugins
                .iter()
                .find(|entry| entry.path.eq_ignore_ascii_case(path) && entry.supported)
            {
                let mut migrated = cfg;
                migrated.audio.vst_plugin_id = Some(entry.id.clone());
                migrated.version = crate::config::AppConfig::CURRENT_VERSION;
                let _ = state.config_store.save(&migrated);
            }
        }
    }
    plugins
}

pub fn retest_vst_plugin(id: &str, state: &AppState) -> Result<VstPluginEntry, CommandError> {
    let cached = state.vst_cache.lock().clone().unwrap_or_default();
    let previous = cached
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or_else(|| audio_error("audio.vst-not-found", "VST catalogue entry not found"))?;
    let tested_entries = probe_plugins_isolated(
        std::path::Path::new(&previous.path),
        std::time::Duration::from_secs(60),
    );
    let tested = tested_entries
        .iter()
        .find(|entry| entry.id == id)
        .or_else(|| tested_entries.iter().find(|entry| entry.supported))
        .or_else(|| tested_entries.first())
        .cloned()
        .ok_or_else(|| audio_error("audio.vst-probe-empty", "VST probe returned no classes"))?;
    let mut updated = cached;
    updated.retain(|entry| !entry.path.eq_ignore_ascii_case(&previous.path));
    updated.extend(tested_entries);
    updated.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    *state.vst_cache.lock() = Some(updated.clone());
    save_vst_cache_to_disk(&updated);
    Ok(tested)
}

pub fn open_vst_folder(id: &str, state: &AppState) -> Result<(), CommandError> {
    let entry = state
        .vst_cache
        .lock()
        .as_ref()
        .and_then(|entries| entries.iter().find(|entry| entry.id == id).cloned())
        .ok_or_else(|| audio_error("audio.vst-not-found", "VST catalogue entry not found"))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(&entry.path)
        .spawn()
        .map_err(|error| audio_error("audio.open-vst-folder-failed", error.to_string()))?;
    Ok(())
}

pub fn list_vst_parameters(state: &AppState) -> Result<Vec<VstParameter>, CommandError> {
    state
        .audio
        .list_vst_parameters()
        .map_err(|err| audio_error("audio.list-vst-parameters-failed", err.to_string()))
}

pub fn set_vst_parameter(index: usize, value: f32, state: &AppState) -> Result<(), CommandError> {
    state
        .audio
        .set_vst_parameter(index, value)
        .map_err(|err| audio_error("audio.set-vst-parameter-failed", err.to_string()))
}

pub fn open_vst_ui(app: &AppHandle, window: &Window, state: &AppState) -> Result<(), CommandError> {
    let cfg = state.config_store.load();
    if !cfg.audio.enabled {
        return Err(audio_error(
            "audio.disabled",
            "Audio engine is disabled in settings",
        ));
    }

    let logger = frontend_logger(window, state);
    if !state.audio.is_running() {
        state
            .audio
            .start(
                audio_settings_from_config(&cfg),
                fallback_vst_path(app),
                logger.clone(),
            )
            .map_err(|err| audio_error("audio.start-failed", err.to_string()))?;
        persist_resolved_audio_device(&cfg, state);
    }
    state
        .audio
        .open_vst_ui(app.clone())
        .map_err(|err| audio_error("audio.open-vst-ui-failed", err.to_string()))
}

pub fn close_vst_ui(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    state
        .audio
        .close_vst_ui(app.clone())
        .map_err(|err| audio_error("audio.close-vst-ui-failed", err.to_string()))
}

pub fn set_master_gain(gain_db: f32, state: &AppState) -> Result<(), CommandError> {
    state.audio.set_gain(gain_db);
    Ok(())
}

pub fn set_audio_limiter(enabled: bool, state: &AppState) -> Result<(), CommandError> {
    state.audio.set_limiter_enabled(enabled);
    Ok(())
}

pub fn ping_audio(app: &AppHandle, window: &Window, state: &AppState) -> Result<(), CommandError> {
    let cfg = state.config_store.load();
    if !cfg.audio.enabled {
        return Err(audio_error(
            "audio.disabled",
            "Audio engine is disabled in settings",
        ));
    }
    let logger = frontend_logger(window, state);
    if !state.audio.is_running() {
        state
            .audio
            .start(
                audio_settings_from_config(&cfg),
                fallback_vst_path(app),
                logger.clone(),
            )
            .map_err(|err| audio_error("audio.start-failed", err.to_string()))?;
        persist_resolved_audio_device(&cfg, state);
    }
    state
        .audio
        .ping()
        .map_err(|err| audio_error("audio.ping-failed", err.to_string()))
}

pub fn reload_vst(
    app: &AppHandle,
    window: &Window,
    state: &AppState,
) -> Result<BridgeStatus, CommandError> {
    let cfg = state.config_store.load();
    if !cfg.audio.enabled {
        return Err(audio_error(
            "audio.disabled",
            "Audio engine is disabled in settings",
        ));
    }
    let logger = frontend_logger(window, state);
    state
        .audio
        .reload(
            audio_settings_from_config(&cfg),
            fallback_vst_path(app),
            logger,
        )
        .map_err(|err| audio_error("audio.reload-failed", err.to_string()))?;
    persist_resolved_audio_device(&cfg, state);
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}
