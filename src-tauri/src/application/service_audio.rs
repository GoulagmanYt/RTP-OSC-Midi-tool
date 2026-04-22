use tauri::{AppHandle, Window};

use crate::{
    audio::AudioSettings,
    error::CommandError,
    tauri::{
        state::AppState,
        utils::{
            audio_settings_from_config, fallback_vst_path, retain_instrument_entries,
            save_vst_cache_to_disk,
        },
    },
    types::{BridgeStatus, VstParameter, VstPluginEntry},
    vst_scan::{default_vst_scan_roots, scan_vst_plugins_in_roots},
};

use super::service_common::{audio_error, frontend_logger, with_audio_status};

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
    let roots = default_vst_scan_roots();
    if roots.is_empty() {
        return state
            .vst_cache
            .lock()
            .clone()
            .map(retain_instrument_entries)
            .unwrap_or_default();
    }

    let plugins = retain_instrument_entries(scan_vst_plugins_in_roots(&roots));
    *state.vst_cache.lock() = Some(plugins.clone());
    save_vst_cache_to_disk(&plugins);
    plugins
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

pub fn start_audio(
    app: &AppHandle,
    window: &Window,
    settings: AudioSettings,
    state: &AppState,
) -> Result<(), CommandError> {
    let logger = frontend_logger(window, state);
    let fallback_vst = fallback_vst_path(app);
    state
        .audio
        .start(settings, fallback_vst, logger)
        .map_err(|err| audio_error("audio.start-failed", err.to_string()))
}

pub fn stop_audio(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    state.audio.stop(Some(app.clone()));
    Ok(())
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
    let status = state.bridge.status(&cfg);
    Ok(with_audio_status(status, &state.audio))
}
