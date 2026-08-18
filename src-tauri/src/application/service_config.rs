use std::{fs, path::PathBuf};

use tauri::{AppHandle, Manager, Window};

use crate::{
    config::Config,
    error::CommandError,
    logger::{set_log_all_to_file, set_logs_enabled},
    tauri::{
        state::AppState,
        utils::{
            audio_settings_from_config, fallback_vst_path, sync_rtp_discovery, sync_runtime_logging,
        },
    },
};

use super::service_common::{
    config_error, frontend_logger, io_to_error, persist_resolved_audio_device, runtime_error,
};

pub fn get_config(app: &AppHandle, state: &AppState) -> Config {
    let cfg = state.config_store.load();
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.logging.log_all_to_file);
    set_logs_enabled(cfg.logging.enabled);
    sync_rtp_discovery(&cfg, state, app);
    cfg
}

pub fn save_config(window: &Window, config: Config, state: &AppState) -> Result<(), CommandError> {
    sync_runtime_logging(&config, &state.dev_logging);
    set_log_all_to_file(config.logging.log_all_to_file);
    set_logs_enabled(config.logging.enabled);
    state
        .config_store
        .save(&config)
        .map_err(|err| config_error("config.save-failed", err))?;
    sync_rtp_discovery(&config, state, window.app_handle());
    let logger = frontend_logger(window, state);
    state
        .bridge
        .sync_rtp(&config, &logger, false)
        .map_err(|err| runtime_error("runtime.sync-rtp-failed", err))?;
    state
        .bridge
        .update_config(config, &logger)
        .map_err(|err| runtime_error("runtime.update-config-failed", err))
}

pub fn reset_config_defaults(window: &Window, state: &AppState) -> Result<Config, CommandError> {
    let cfg = state
        .config_store
        .reset_to_default()
        .map_err(|err| config_error("config.reset-failed", err))?;
    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.logging.log_all_to_file);
    set_logs_enabled(cfg.logging.enabled);
    let _ = state.config_store.save(&cfg);
    sync_rtp_discovery(&cfg, state, window.app_handle());
    let logger = frontend_logger(window, state);
    state
        .bridge
        .sync_rtp(&cfg, &logger, true)
        .map_err(|err| runtime_error("runtime.sync-rtp-failed", err))?;
    state
        .bridge
        .update_config(cfg.clone(), &logger)
        .map_err(|err| runtime_error("runtime.update-config-failed", err))?;
    Ok(cfg)
}

pub fn export_config(path: String, state: &AppState) -> Result<(), CommandError> {
    let cfg = state.config_store.load();
    let yaml = serde_yaml::to_string(&cfg)
        .map_err(|err| config_error("config.serialize-failed", err.to_string()))?;
    if let Some(parent) = PathBuf::from(&path).parent() {
        fs::create_dir_all(parent)
            .map_err(|err| io_to_error("config.export-failed", "config", err))?;
    }
    fs::write(&path, yaml).map_err(|err| io_to_error("config.export-failed", "config", err))
}

pub fn import_config(
    app: &AppHandle,
    window: &Window,
    path: String,
    state: &AppState,
) -> Result<Config, CommandError> {
    let raw = fs::read_to_string(&path)
        .map_err(|err| io_to_error("config.import-read-failed", "config", err))?;
    let cfg: Config = serde_yaml::from_str(&raw)
        .map_err(|err| config_error("config.import-parse-failed", err.to_string()))?;

    sync_runtime_logging(&cfg, &state.dev_logging);
    set_log_all_to_file(cfg.logging.log_all_to_file);
    set_logs_enabled(cfg.logging.enabled);
    state
        .config_store
        .save(&cfg)
        .map_err(|err| config_error("config.save-failed", err))?;
    sync_rtp_discovery(&cfg, state, app);
    let logger = frontend_logger(window, state);
    state
        .bridge
        .sync_rtp(&cfg, &logger, true)
        .map_err(|err| runtime_error("runtime.sync-rtp-failed", err))?;
    state
        .bridge
        .update_config(cfg.clone(), &logger)
        .map_err(|err| runtime_error("runtime.update-config-failed", err))?;

    if cfg.audio.enabled {
        let fallback_vst = fallback_vst_path(app);
        if let Err(err) = state.audio.start(
            audio_settings_from_config(&cfg),
            fallback_vst,
            logger.clone(),
        ) {
            logger.error(format!("Audio not restarted after import: {err}"));
        } else {
            persist_resolved_audio_device(&cfg, state);
        }
    } else {
        state.audio.stop(Some(app.clone()));
    }

    Ok(cfg)
}
