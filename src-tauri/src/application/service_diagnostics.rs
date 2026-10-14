use std::path::Path;

use crate::{error::CommandError, tauri::state::AppState};

use super::service_common::{build_diagnostic_export, io_to_error, runtime_error};

pub fn export_diagnostics(path: String, state: &AppState) -> Result<(), CommandError> {
    let payload = build_diagnostic_export(state);
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|err| runtime_error("runtime.diagnostics-serialize-failed", err.to_string()))?;
    crate::atomic_file::write_atomically(Path::new(&path), json.as_bytes())
        .map_err(|err| io_to_error("runtime.export-diagnostics-failed", "runtime", err))
}
