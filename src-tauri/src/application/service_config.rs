use std::{fs, path::Path};

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
    let audio_was_running = state.audio.is_running();
    let bridge_running = state.bridge.status(&previous).running;
    let logger = frontend_logger(window, state);
    let mut audio_restart_attempted = false;
    let mut applied_gain_db = previous.audio.gain_db;
    let mut applied_limiter = previous.audio.limiter_enabled;
    let prev_eq_key = previous
        .audio
        .vst_plugin_id
        .as_deref()
        .unwrap_or(crate::config::BUNDLED_VST_EQ_KEY);
    let mut applied_vst_eq = previous
        .audio
        .vst_eq_by_plugin
        .get(prev_eq_key)
        .cloned()
        .unwrap_or_default();
    commit_config_change(
        &previous,
        config,
        force_restart,
        |cfg, force| {
            if state.audio.is_running() {
                state
                    .audio
                    .set_game_mode(cfg.audio.game_mode, window.app_handle().clone())
                    .map_err(|err| runtime_error("audio.game-mode-failed", err.to_string()))?;
            }
            // Bind the replacement OSC listener before restarting RTP.
            state
                .bridge
                .update_config(cfg.clone(), &logger)
                .map_err(|err| runtime_error("runtime.update-config-failed", err))?;
            state
                .bridge
                .sync_rtp(cfg, &logger, force)
                .map_err(|err| runtime_error("runtime.sync-rtp-failed", err))?;
            let should_run = desired_audio_running(
                previous.audio.enabled,
                cfg.audio.enabled,
                audio_was_running,
                bridge_running,
            );
            if should_run && (force || audio_restart_attempted || !state.audio.is_running()) {
                // Also restart the previous settings if application or persistence
                // fails after replacing (or attempting to replace) the worker.
                audio_restart_attempted = true;
                state
                    .audio
                    .start(
                        audio_settings_from_config(cfg),
                        fallback_vst_path(window.app_handle()),
                        logger.clone(),
                    )
                    .map_err(|err| runtime_error("audio.start-failed", err.to_string()))?;
                applied_gain_db = cfg.audio.gain_db;
                applied_limiter = cfg.audio.limiter_enabled;
                let eq_key = cfg
                    .audio
                    .vst_plugin_id
                    .as_deref()
                    .unwrap_or(crate::config::BUNDLED_VST_EQ_KEY);
                applied_vst_eq = cfg
                    .audio
                    .vst_eq_by_plugin
                    .get(eq_key)
                    .cloned()
                    .unwrap_or_default();
            } else if !should_run && (audio_restart_attempted || state.audio.is_running()) {
                state.audio.stop(Some(window.app_handle().clone()));
            } else if should_run {
                apply_gain_change(&mut applied_gain_db, cfg.audio.gain_db, |gain_db| {
                    state
                        .audio
                        .set_gain(gain_db)
                        .map_err(|err| runtime_error("audio.set-gain-failed", err.to_string()))
                })?;
                apply_limiter_change(&mut applied_limiter, cfg.audio.limiter_enabled, |limiter| {
                    state
                        .audio
                        .set_limiter_enabled(limiter)
                        .map_err(|err| runtime_error("audio.set-limiter-failed", err.to_string()))
                })?;
                let eq_key = cfg
                    .audio
                    .vst_plugin_id
                    .as_deref()
                    .unwrap_or(crate::config::BUNDLED_VST_EQ_KEY);
                let current_eq = cfg
                    .audio
                    .vst_eq_by_plugin
                    .get(eq_key)
                    .cloned()
                    .unwrap_or_default();
                apply_vst_eq_change(&mut applied_vst_eq, current_eq, |eq| {
                    state
                        .audio
                        .set_vst_eq(eq)
                        .map_err(|err| runtime_error("audio.set-vst-eq-failed", err.to_string()))
                })?;
            }
            Ok(())
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

// Track admitted changes so a failed disk write restores the active gain as
// well as the saved configuration. Unrelated edits do not send audio controls.
fn apply_gain_change(
    applied: &mut f32,
    requested: f32,
    set_gain: impl FnOnce(f32) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    if *applied != requested {
        set_gain(requested)?;
        *applied = requested;
    }
    Ok(())
}

fn apply_limiter_change(
    applied: &mut bool,
    requested: bool,
    set_limiter: impl FnOnce(bool) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    if *applied != requested {
        set_limiter(requested)?;
        *applied = requested;
    }
    Ok(())
}

fn apply_vst_eq_change(
    applied: &mut crate::config::VstEqSettings,
    requested: crate::config::VstEqSettings,
    set_vst_eq: impl FnOnce(crate::config::VstEqSettings) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    if *applied != requested {
        set_vst_eq(requested.clone())?;
        *applied = requested;
    }
    Ok(())
}

// Preserve the actual runtime for unrelated edits and restore it on rollback.
// Enabling audio while the bridge is stopped only changes the next startup.
fn desired_audio_running(
    previous_enabled: bool,
    next_enabled: bool,
    initially_running: bool,
    bridge_running: bool,
) -> bool {
    if previous_enabled == next_enabled {
        initially_running
    } else {
        next_enabled && (bridge_running || initially_running)
    }
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
    crate::atomic_file::write_atomically(Path::new(&path), yaml.as_bytes())
        .map_err(|err| io_to_error("config.export-failed", "config", err))
}

pub fn import_config(
    _app: &AppHandle,
    window: &Window,
    path: String,
    state: &AppState,
) -> Result<Config, CommandError> {
    let raw = fs::read_to_string(&path)
        .map_err(|err| io_to_error("config.import-read-failed", "config", err))?;
    let cfg =
        Config::parse_yaml(&raw).map_err(|err| config_error("config.import-parse-failed", err))?;

    apply_and_save_config(window, &cfg, state, true)?;
    persist_resolved_audio_device(&cfg, state);
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn saved_gain_changes_reach_the_active_engine_and_roll_back_on_disk_failure() {
        for fail_save in [false, true] {
            let previous = Config::default();
            let mut next = previous.clone();
            next.audio.gain_db = -12.0;
            let active_gain = Cell::new(previous.audio.gain_db);
            let mut applied = previous.audio.gain_db;
            let result = commit_config_change(
                &previous,
                &next,
                false,
                |cfg, _| {
                    apply_gain_change(&mut applied, cfg.audio.gain_db, |gain| {
                        active_gain.set(gain);
                        Ok(())
                    })
                },
                |_| {
                    if fail_save {
                        Err(config_error("config.save-failed", "disk failure"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(result.is_err(), fail_save);
            assert_eq!(active_gain.get(), if fail_save { 0.0 } else { -12.0 });
        }
    }

    #[test]
    fn rejected_gain_is_not_persisted_and_unrelated_edits_send_no_gain_control() {
        let previous = Config::default();
        let mut next = previous.clone();
        next.audio.gain_db = -12.0;
        let mut applied = previous.audio.gain_db;
        let persisted = Cell::new(false);
        let calls = Cell::new(0);
        let result = commit_config_change(
            &previous,
            &next,
            false,
            |cfg, _| {
                apply_gain_change(&mut applied, cfg.audio.gain_db, |_| {
                    calls.set(calls.get() + 1);
                    Err(runtime_error("audio.set-gain-failed", "worker unavailable"))
                })
            },
            |_| {
                persisted.set(true);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().code, "audio.set-gain-failed");
        assert!(!persisted.get());
        assert_eq!(applied, previous.audio.gain_db);
        assert_eq!(calls.get(), 1);
        apply_gain_change(&mut applied, previous.audio.gain_db, |_| {
            panic!("unchanged gain must not send a command")
        })
        .unwrap();
    }

    #[test]
    fn rejected_import_restart_or_persistence_restores_the_previous_audio_settings() {
        for fail_restart in [false, true] {
            let previous = Config::default();
            let mut next = previous.clone();
            next.audio.vst_path = Some("replacement.dll".into());
            let active = std::cell::RefCell::new(previous.audio.clone());
            let persisted = Cell::new(false);
            let error = commit_config_change(
                &previous,
                &next,
                true,
                |cfg, force| {
                    *active.borrow_mut() = cfg.audio.clone();
                    if force && fail_restart {
                        Err(runtime_error(
                            "audio.start-failed",
                            "replacement worker rejected",
                        ))
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    persisted.set(true);
                    Err(config_error("config.save-failed", "disk failure"))
                },
            )
            .unwrap_err();
            assert_eq!(*active.borrow(), previous.audio);
            assert_eq!(persisted.get(), !fail_restart);
            assert_eq!(
                error.code,
                if fail_restart {
                    "audio.start-failed"
                } else {
                    "config.save-failed"
                }
            );
        }
    }

    #[test]
    fn audio_enable_changes_follow_bridge_lifecycle_and_preserve_unrelated_edits() {
        assert!(desired_audio_running(false, true, false, true));
        assert!(!desired_audio_running(true, false, true, true));
        assert!(!desired_audio_running(false, true, false, false));
        assert!(!desired_audio_running(true, true, false, true));
        assert!(desired_audio_running(true, true, true, false));
    }

    #[test]
    fn rejected_audio_enable_change_restores_initial_runtime() {
        for initially_running in [false, true] {
            let mut previous = Config::default();
            previous.audio.enabled = initially_running;
            let mut next = previous.clone();
            next.audio.enabled = !initially_running;
            let running = Cell::new(initially_running);
            let result = commit_config_change(
                &previous,
                &next,
                false,
                |cfg, _| {
                    running.set(desired_audio_running(
                        previous.audio.enabled,
                        cfg.audio.enabled,
                        initially_running,
                        true,
                    ));
                    Ok(())
                },
                |_| Err(config_error("write", "simulated disk failure")),
            );
            assert!(result.is_err());
            assert_eq!(running.get(), initially_running);
        }
    }

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

    #[test]
    fn saved_limiter_changes_reach_the_active_engine_and_roll_back_on_disk_failure() {
        for fail_save in [false, true] {
            let previous = Config::default();
            let mut next = previous.clone();
            next.audio.limiter_enabled = !previous.audio.limiter_enabled;
            let active_limiter = Cell::new(previous.audio.limiter_enabled);
            let mut applied = previous.audio.limiter_enabled;
            let result = commit_config_change(
                &previous,
                &next,
                false,
                |cfg, _| {
                    apply_limiter_change(&mut applied, cfg.audio.limiter_enabled, |limiter| {
                        active_limiter.set(limiter);
                        Ok(())
                    })
                },
                |_| {
                    if fail_save {
                        Err(config_error("config.save-failed", "disk failure"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(result.is_err(), fail_save);
            assert_eq!(
                active_limiter.get(),
                if fail_save {
                    previous.audio.limiter_enabled
                } else {
                    next.audio.limiter_enabled
                }
            );
        }
    }

    #[test]
    fn saved_vst_eq_changes_reach_the_active_engine_and_roll_back_on_disk_failure() {
        for fail_save in [false, true] {
            let previous = Config::default();
            let modified_eq = crate::config::VstEqSettings {
                enabled: true,
                low_shelf: crate::config::EqShelfSettings {
                    frequency_hz: 120.0,
                    gain_db: 4.5,
                },
                ..Default::default()
            };
            let mut next = previous.clone();
            next.audio.vst_eq_by_plugin.insert(
                crate::config::BUNDLED_VST_EQ_KEY.to_string(),
                modified_eq.clone(),
            );
            let active_eq = std::cell::RefCell::new(crate::config::VstEqSettings::default());
            let mut applied = crate::config::VstEqSettings::default();
            let result = commit_config_change(
                &previous,
                &next,
                false,
                |cfg, _| {
                    let eq = cfg
                        .audio
                        .vst_eq_by_plugin
                        .get(crate::config::BUNDLED_VST_EQ_KEY)
                        .cloned()
                        .unwrap_or_default();
                    apply_vst_eq_change(&mut applied, eq, |settings| {
                        *active_eq.borrow_mut() = settings;
                        Ok(())
                    })
                },
                |_| {
                    if fail_save {
                        Err(config_error("config.save-failed", "disk failure"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(result.is_err(), fail_save);
            assert_eq!(
                *active_eq.borrow(),
                if fail_save {
                    crate::config::VstEqSettings::default()
                } else {
                    modified_eq
                }
            );
        }
    }
}
