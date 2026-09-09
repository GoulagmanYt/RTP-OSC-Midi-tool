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

/// Commit only after the runtime accepts the configuration. Roll back runtime
/// changes if binding or persistence fails; preserve the original error and
/// report a rollback failure separately.
fn commit_config_change(
    previous: &Config,
    next: &Config,
    force_restart: bool,
    mut apply: impl FnMut(&Config, bool) -> Result<(), CommandError>,
    persist: impl FnOnce(&Config) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let result = apply(next, force_restart).and_then(|()| persist(next));
    if let Err(mut error) = result {
        if let Err(rollback) = apply(previous, false) {
            error
                .details
                .insert("rollbackError".into(), rollback.to_string());
        }
        return Err(error);
    }
    Ok(())
}

fn apply_and_save_config(
    window: &Window,
    config: &Config,
    state: &AppState,
    force_restart: bool,
) -> Result<(), CommandError> {
    let _change = state.config_change.lock();
    let previous = state.config_store.load();
    let logger = frontend_logger(window, state);
    commit_config_change(
        &previous,
        config,
        force_restart,
        |cfg, force| {
            // Bind the replacement OSC listener before restarting RTP.
            state
                .bridge
                .update_config(cfg.clone(), &logger)
                .map_err(|err| runtime_error("runtime.update-config-failed", err))?;
            state
                .bridge
                .sync_rtp(cfg, &logger, force)
                .map_err(|err| runtime_error("runtime.sync-rtp-failed", err))
        },
        |cfg| {
            state
                .config_store
                .save(cfg)
                .map_err(|err| config_error("config.save-failed", err))
        },
    )?;
    sync_runtime_logging(config, &state.dev_logging);
    set_log_all_to_file(config.logging.log_all_to_file);
    set_logs_enabled(config.logging.enabled);
    sync_rtp_discovery(config, state, window.app_handle());
    Ok(())
}

pub fn save_config(window: &Window, config: Config, state: &AppState) -> Result<(), CommandError> {
    apply_and_save_config(window, &config, state, false)
}

pub fn reset_config_defaults(window: &Window, state: &AppState) -> Result<Config, CommandError> {
    let cfg = Config::default();
    apply_and_save_config(window, &cfg, state, true)?;
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

    apply_and_save_config(window, &cfg, state, true)?;
    let logger = frontend_logger(window, state);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn failed_bind_or_save_preserves_disk_and_restores_runtime_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.yaml");
        let store = crate::config::ConfigStore::with_path(path.clone());
        let previous = store.load();
        let original = fs::read(&path).unwrap();
        let occupied = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut next = previous.clone();
        next.osc.listen_port = occupied.local_addr().unwrap().port();
        let active_port = Cell::new(previous.osc.listen_port);
        let error = commit_config_change(
            &previous,
            &next,
            false,
            |cfg, _| {
                active_port.set(cfg.osc.listen_port);
                if cfg.osc.listen_port == next.osc.listen_port {
                    std::net::UdpSocket::bind(("127.0.0.1", cfg.osc.listen_port))
                        .map_err(|error| runtime_error("bind", error.to_string()))?;
                }
                Ok(())
            },
            |cfg| store.save(cfg).map_err(CommandError::from),
        )
        .unwrap_err();
        assert_eq!(error.code, "bind");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(active_port.get(), previous.osc.listen_port);

        let error = commit_config_change(
            &previous,
            &next,
            false,
            |cfg, _| {
                active_port.set(cfg.osc.listen_port);
                Ok(())
            },
            |_| Err(config_error("write", "simulated disk failure")),
        )
        .unwrap_err();
        assert_eq!(error.code, "write");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(active_port.get(), previous.osc.listen_port);

        commit_config_change(
            &previous,
            &next,
            false,
            |cfg, _| {
                active_port.set(cfg.osc.listen_port);
                Ok(())
            },
            |cfg| store.save(cfg).map_err(CommandError::from),
        )
        .unwrap();
        assert_eq!(store.load().osc.listen_port, next.osc.listen_port);
        assert_eq!(active_port.get(), next.osc.listen_port);
    }

    #[test]
    fn rollback_failure_is_reported_without_hiding_the_original_error() {
        let previous = Config::default();
        let error = commit_config_change(
            &previous,
            &previous,
            true,
            |_, force| {
                Err(runtime_error(
                    if force { "apply" } else { "rollback" },
                    "failed",
                ))
            },
            |_| panic!("failed application must not persist"),
        )
        .unwrap_err();
        assert_eq!(error.code, "apply");
        assert!(error.details["rollbackError"].contains("rollback"));
    }
}
