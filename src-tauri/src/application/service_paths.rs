use crate::{
    error::CommandError,
    tauri::utils::{config_dir_path, log_file_path, open_folder_in_explorer},
    types::AppPaths,
};

use super::service_common::{clear_log_file as clear_log_file_impl, paths_error};

pub fn get_app_paths() -> Result<AppPaths, CommandError> {
    AppPaths::new().map_err(|err| paths_error("paths.resolve-failed", err))
}

pub fn open_app_dir(target: String) -> Result<(), CommandError> {
    let config_dir =
        config_dir_path().map_err(|err| paths_error("paths.config-dir-failed", err))?;
    let log_dir = log_file_path()
        .map_err(|err| paths_error("paths.log-file-failed", err))?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| {
            paths_error(
                "paths.log-dir-failed",
                "Impossible de determiner le dossier des logs",
            )
        })?;

    match target.as_str() {
        "config" => open_folder_in_explorer(&config_dir)
            .map_err(|err| paths_error("paths.open-config-dir-failed", err)),
        "logs" => open_folder_in_explorer(&log_dir)
            .map_err(|err| paths_error("paths.open-log-dir-failed", err)),
        _ => Err(paths_error(
            "paths.unknown-target",
            "Dossier inconnu (config|logs)",
        )),
    }
}

pub fn clear_log_file() -> Result<(), CommandError> {
    clear_log_file_impl()
}
