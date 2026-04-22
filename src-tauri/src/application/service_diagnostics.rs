use std::{fs, path::PathBuf};

use crate::{error::CommandError, tauri::state::AppState};

use super::service_common::{build_diagnostic_export, io_to_error, runtime_error};

pub fn export_diagnostics(path: String, state: &AppState) -> Result<(), CommandError> {
    let payload = build_diagnostic_export(state);
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|err| runtime_error("runtime.diagnostics-serialize-failed", err.to_string()))?;
    if let Some(parent) = PathBuf::from(&path).parent() {
        fs::create_dir_all(parent)
            .map_err(|err| io_to_error("runtime.export-diagnostics-failed", "runtime", err))?;
    }
    fs::write(&path, json)
        .map_err(|err| io_to_error("runtime.export-diagnostics-failed", "runtime", err))
}
