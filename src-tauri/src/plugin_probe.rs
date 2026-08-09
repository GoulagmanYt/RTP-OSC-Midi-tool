#![allow(deprecated)]

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::types::VstPluginEntry;
use rack::PluginInfo as RackPluginInfo;
use std::path::PathBuf as StdPathBuf;

#[path = "plugin_probe_arch.rs"]
mod plugin_probe_arch;
#[path = "plugin_probe_paths.rs"]
mod plugin_probe_paths;
#[path = "plugin_probe_vst2.rs"]
pub(crate) mod plugin_probe_vst2;
#[path = "plugin_probe_vst3.rs"]
mod plugin_probe_vst3;

#[cfg(test)]
use plugin_probe_arch::{detect_vst3_architecture, read_pe_machine};
use plugin_probe_paths::{normalize_path, plugin_name, unique_existing_roots};
use plugin_probe_vst2::probe_vst2_plugin;

pub(crate) fn probe_trace(message: impl AsRef<str>) {
    if std::env::var_os("OSCMIDI_VST_PROBE_TRACE").is_some() {
        eprintln!("{}", message.as_ref());
    }
}

pub fn default_vst_scan_roots() -> Vec<PathBuf> {
    plugin_probe_paths::default_vst_scan_roots()
}

pub fn is_vst2_path(path: &Path) -> bool {
    plugin_probe_paths::is_vst2_path(path)
}

pub fn is_vst3_path(path: &Path) -> bool {
    plugin_probe_paths::is_vst3_path(path)
}

pub fn scan_plugin_candidates(root: &Path) -> Vec<PathBuf> {
    plugin_probe_paths::scan_plugin_candidates(root)
}

pub fn detect_vst3_channels(
    plugin: &mut rack::vst3::Vst3Plugin,
    info: &RackPluginInfo,
    max_block_size: usize,
) -> Result<(usize, usize), String> {
    plugin_probe_vst3::detect_vst3_channels(plugin, info, max_block_size)
}

pub(crate) fn select_vst3_plugin_info_for_path(
    plugins: &[RackPluginInfo],
    target_path: &Path,
) -> Option<RackPluginInfo> {
    plugin_probe_vst3::select_vst3_plugin_info_for_path(plugins, target_path)
}

pub(crate) fn vst3_scan_root_for_path(path: &Path) -> StdPathBuf {
    plugin_probe_vst3::vst3_scan_root_for_path(path)
}

pub fn scan_vst_plugins_in_roots(roots: &[PathBuf]) -> Vec<VstPluginEntry> {
    let mut entries = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in unique_existing_roots(roots) {
        for path in scan_plugin_candidates(&root) {
            let key = normalize_path(&path);
            if seen_paths.insert(key) {
                let entry = probe_plugin_isolated(&path, Duration::from_secs(10));
                if is_listable_instrument_entry(&entry) {
                    entries.push(entry);
                }
            }
        }
    }

    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    entries
}

fn is_listable_instrument_entry(entry: &VstPluginEntry) -> bool {
    entry.kind == "instrument" && entry.supported
}

pub fn ensure_supported_plugin_in_app(path: &Path) -> Result<VstPluginEntry, String> {
    let entry = probe_plugin_isolated(path, Duration::from_secs(10));
    if entry.supported {
        Ok(entry)
    } else {
        Err(entry
            .unsupported_reason
            .clone()
            .unwrap_or_else(|| "Plugin is not supported in this app".to_string()))
    }
}

pub fn smoke_vst2_instance(
    instance: &mut vst::host::PluginInstance,
    block_size: usize,
) -> Result<(usize, usize), String> {
    plugin_probe_vst2::smoke_vst2_instance(instance, block_size)
}

pub fn probe_plugin_isolated(path: &Path, timeout: Duration) -> VstPluginEntry {
    match run_probe_process(path, timeout) {
        Ok(entry) => entry,
        Err(reason) => unsupported_entry(
            path,
            plugin_name(path),
            if is_vst3_path(path) { "VST3" } else { "VST2" }.to_string(),
            "instrument".to_string(),
            "unknown".to_string(),
            reason,
        ),
    }
}

fn run_probe_process(path: &Path, timeout: Duration) -> Result<VstPluginEntry, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Could not locate probe executable: {error}"))?;
    let dedicated = current_exe.with_file_name(if cfg!(windows) {
        "vst_probe.exe"
    } else {
        "vst_probe"
    });
    let (program, use_current_exe) = if dedicated.exists() && dedicated != current_exe {
        (dedicated, false)
    } else {
        (current_exe, true)
    };
    let mut command = Command::new(program);
    if use_current_exe {
        command.arg("--vst-probe");
    }
    let mut child = command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to start isolated VST probe: {error}"))?;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "VST probe stdout pipe unavailable".to_string())?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "VST probe stderr pipe unavailable".to_string())?;
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!(
                    "VST probe timed out after {:.1}s",
                    timeout.as_secs_f32()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("Failed while waiting for VST probe: {error}"));
            }
        }
    }
    let stdout_bytes = stdout_reader
        .join()
        .map_err(|_| "VST probe stdout reader panicked".to_string())?;
    let stderr_bytes = stderr_reader
        .join()
        .map_err(|_| "VST probe stderr reader panicked".to_string())?;
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let json = stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("OSCMIDI_PROBE_JSON:"))
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            format!("VST probe exited without a result: {}", stderr.trim())
        })?;
    serde_json::from_str(json).map_err(|error| format!("Invalid VST probe result: {error}"))
}

pub fn probe_plugin(path: &Path) -> VstPluginEntry {
    if is_vst3_path(path) {
        plugin_probe_vst3::probe_vst3_plugin(path)
    } else if is_vst2_path(path) {
        probe_vst2_plugin(path)
    } else {
        unsupported_entry(
            path,
            plugin_name(path),
            "Unknown".to_string(),
            "other".to_string(),
            "unknown".to_string(),
            "Unsupported plugin path".to_string(),
        )
    }
}

pub(crate) fn unsupported_entry(
    path: &Path,
    name: String,
    format: String,
    kind: String,
    architecture: String,
    reason: String,
) -> VstPluginEntry {
    let (file_modified_ms, file_size) = plugin_file_metadata(path);
    VstPluginEntry {
        name,
        path: path.to_string_lossy().to_string(),
        format,
        kind,
        architecture,
        supported: false,
        unsupported_reason: Some(reason),
        midi_compatible: None,
        has_editor: false,
        channel_layout: None,
        class_uid: None,
        file_modified_ms,
        file_size,
        host_abi_version: 1,
    }
}

pub(crate) fn plugin_file_metadata(path: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return (None, None);
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64);
    (modified, Some(metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rack::{PluginInfo, PluginType};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scan_roots_include_existing_windows_folders_only() {
        let roots = default_vst_scan_roots();
        let set: HashSet<PathBuf> = roots.iter().cloned().collect();
        assert_eq!(set.len(), roots.len());
    }

    #[test]
    fn scan_candidates_collects_vst2_and_vst3() {
        let dir = tempdir().expect("temp dir");
        let vst2 = dir.path().join("Synth.dll");
        let vst3 = dir.path().join("Piano.vst3");
        let nested = dir.path().join("Nested");

        fs::write(&vst2, b"MZ").expect("write vst2");
        fs::create_dir_all(&vst3).expect("write vst3");
        fs::create_dir_all(&nested).expect("nested");
        fs::write(nested.join("Ignored.txt"), b"").expect("ignored");

        let entries = scan_plugin_candidates(dir.path());
        assert!(entries.iter().any(|p| p.ends_with("Synth.dll")));
        assert!(entries.iter().any(|p| p.ends_with("Piano.vst3")));
    }

    #[test]
    fn read_pe_machine_detects_x64() {
        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("plugin.dll");
        let mut bytes = vec![0u8; 0x90];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        bytes[0x3c..0x40].copy_from_slice(&(0x80u32).to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&(0x8664u16).to_le_bytes());
        fs::write(&file, bytes).expect("write");
        assert_eq!(read_pe_machine(&file).as_deref(), Some("x64"));
    }

    #[test]
    fn read_pe_machine_detects_x86() {
        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("plugin.dll");
        let mut bytes = vec![0u8; 0x90];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        bytes[0x3c..0x40].copy_from_slice(&(0x80u32).to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&(0x14cu16).to_le_bytes());
        fs::write(&file, bytes).expect("write");
        assert_eq!(read_pe_machine(&file).as_deref(), Some("x86"));
    }

    #[test]
    fn detect_vst3_architecture_from_bundle_layout() {
        let dir = tempdir().expect("temp dir");
        let bundle = dir.path().join("Plugin.vst3");
        fs::create_dir_all(bundle.join("Contents").join("x86_64-win")).expect("layout");
        assert_eq!(detect_vst3_architecture(&bundle), "x64");
    }

    #[test]
    fn matches_vst3_bundle_path_accepts_bundle_binary() {
        let bundle = PathBuf::from("C:/VST3/Plugin.vst3");
        let candidate = bundle
            .join("Contents")
            .join("x86_64-win")
            .join("Plugin.vst3");
        let plugins = vec![PluginInfo::new(
            "Plugin".into(),
            "Vendor".into(),
            1,
            PluginType::Instrument,
            candidate,
            "plugin".into(),
        )];
        let selected = select_vst3_plugin_info_for_path(&plugins, &bundle).expect("selected");
        assert_eq!(selected.name, "Plugin");
    }

    #[test]
    fn select_vst3_plugin_info_prefers_instrument_for_same_bundle() {
        let bundle = PathBuf::from("C:/VST3/Plugin.vst3");
        let plugins = vec![
            PluginInfo::new(
                "Plugin FX".into(),
                "Vendor".into(),
                1,
                PluginType::Effect,
                bundle.clone(),
                "effect".into(),
            ),
            PluginInfo::new(
                "Plugin Instrument".into(),
                "Vendor".into(),
                1,
                PluginType::Instrument,
                bundle.clone(),
                "instrument".into(),
            ),
        ];

        let selected = select_vst3_plugin_info_for_path(&plugins, &bundle).expect("selected");
        assert_eq!(selected.name, "Plugin Instrument");
    }

    #[test]
    fn unknown_paths_return_unsupported_entry() {
        let path = PathBuf::from("C:/Unknown/plugin.txt");
        let entry = probe_plugin(&path);
        assert!(!entry.supported);
        assert_eq!(entry.kind, "other");
        assert_eq!(
            entry.unsupported_reason.as_deref(),
            Some("Unsupported plugin path")
        );
    }
}
