#![allow(deprecated)]

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::types::{legacy_path_plugin_id, PluginStatus, VstPluginEntry, VST_HOST_ABI_VERSION};
use rack::PluginInfo as RackPluginInfo;
use std::path::PathBuf as StdPathBuf;

const UPRIGHT_PIANO_VST3_CLASS_UID: &str = "5653545F474B64757072696768742070";
const UPRIGHT_PIANO_UNSAFE_STATE_FILE_SIZE: u64 = 4_173_312;
const MAX_PROBE_STREAM_BYTES: u64 = 256 * 1024;

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
use plugin_probe_vst2::probe_vst2_plugins;

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

pub fn native_plugin_load_path(path: &Path) -> StdPathBuf {
    plugin_probe_paths::native_plugin_load_path(path)
}

/// Compatibility profile for plug-in builds whose native state callback is
/// known to crash inside the isolated host. The class ID and file fingerprint
/// keep this exception from silently applying to a future fixed Upright build.
pub fn vst3_native_state_requires_parameter_fallback(
    path: &Path,
    class_uid: Option<&str>,
    file_size: Option<u64>,
) -> bool {
    let normalized = path.to_string_lossy().to_lowercase();
    if normalized.ends_with("sforzando.vst3") || normalized.contains("\\sforzando.vst3") {
        return true;
    }

    class_uid.is_some_and(|uid| uid.eq_ignore_ascii_case(UPRIGHT_PIANO_VST3_CLASS_UID))
        && file_size.is_some_and(|size| size == UPRIGHT_PIANO_UNSAFE_STATE_FILE_SIZE)
}

pub(crate) fn requires_vst2_destructor_quarantine(unique_id: i32) -> bool {
    plugin_probe_vst2::requires_vst2_destructor_quarantine(unique_id)
}

pub fn detect_vst3_channels(
    plugin: &mut rack::vst3::Vst3Plugin,
    info: &RackPluginInfo,
    max_block_size: usize,
) -> Result<(usize, usize), String> {
    plugin_probe_vst3::detect_vst3_channels(plugin, info, max_block_size)
}

pub fn select_vst3_plugin_info_for_path(
    plugins: &[RackPluginInfo],
    target_path: &Path,
) -> Option<RackPluginInfo> {
    plugin_probe_vst3::select_vst3_plugin_info_for_path(plugins, target_path)
}

pub fn vst3_scan_root_for_path(path: &Path) -> StdPathBuf {
    plugin_probe_vst3::vst3_scan_root_for_path(path)
}

pub fn scan_vst_plugins_in_roots(roots: &[PathBuf]) -> Vec<VstPluginEntry> {
    scan_vst_plugins_in_roots_cached(roots, &[], false)
}

pub fn scan_vst_plugins_in_roots_cached(
    roots: &[PathBuf],
    cached: &[VstPluginEntry],
    force_retest: bool,
) -> Vec<VstPluginEntry> {
    let mut entries = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut cached_by_path: HashMap<String, Vec<VstPluginEntry>> = HashMap::new();
    for entry in cached {
        cached_by_path
            .entry(normalize_path(Path::new(&entry.path)))
            .or_default()
            .push(entry.clone());
    }
    let mut x86_candidates = Vec::new();
    let mut x64_candidates = Vec::new();

    for root in unique_existing_roots(roots) {
        for path in scan_plugin_candidates(&root) {
            let key = normalize_path(&path);
            if seen_paths.insert(key) {
                let (modified, size) = plugin_file_metadata(&path);
                let cache_key = normalize_path(&path);
                if !force_retest {
                    if let Some(hit) = cached_by_path.get(&cache_key).filter(|variants| {
                        !variants.is_empty()
                            && variants.iter().all(|entry| {
                                entry.file_modified_ms == modified
                                    && entry.file_size == size
                                    && entry.host_abi_version == VST_HOST_ABI_VERSION
                            })
                    }) {
                        entries.extend(hit.iter().cloned());
                        continue;
                    }
                }
                let architectures = plugin_probe_arch::available_plugin_architectures(&path);
                if architectures
                    .iter()
                    .any(|architecture| architecture == "x86")
                {
                    x86_candidates.push(path.clone());
                }
                if architectures
                    .iter()
                    .any(|architecture| architecture != "x86")
                {
                    x64_candidates.push(path);
                }
            }
        }
    }

    // Each lane is deliberately sequential: at most one x86 probe and one x64
    // probe execute concurrently, limiting the blast radius of hostile DLLs.
    let timeout = if force_retest {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(30)
    };
    let x86 = std::thread::spawn(move || {
        x86_candidates
            .into_iter()
            .flat_map(|path| probe_plugins_isolated(&path, timeout))
            .collect::<Vec<_>>()
    });
    let x64 = std::thread::spawn(move || {
        x64_candidates
            .into_iter()
            .flat_map(|path| probe_plugins_isolated(&path, timeout))
            .collect::<Vec<_>>()
    });
    entries.extend(x86.join().unwrap_or_default());
    entries.extend(x64.join().unwrap_or_default());

    for index in 0..entries.len() {
        let mut architectures = entries
            .iter()
            .filter(|candidate| {
                candidate
                    .format
                    .eq_ignore_ascii_case(&entries[index].format)
                    && candidate.name.eq_ignore_ascii_case(&entries[index].name)
                    && candidate.vendor == entries[index].vendor
            })
            .map(|candidate| candidate.architecture.clone())
            .collect::<Vec<_>>();
        architectures.sort_by_key(|architecture| if architecture == "x64" { 0 } else { 1 });
        architectures.dedup();
        entries[index].available_architectures = architectures;
    }

    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| {
                let rank = |architecture: &str| if architecture == "x64" { 0 } else { 1 };
                rank(&a.architecture).cmp(&rank(&b.architecture))
            })
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    entries
}

pub fn ensure_supported_plugin_in_app(path: &Path) -> Result<VstPluginEntry, String> {
    ensure_plugin_reference_in_app(path, None)
}

pub fn ensure_plugin_reference_in_app(
    path: &Path,
    plugin_id: Option<&str>,
) -> Result<VstPluginEntry, String> {
    let entries = probe_plugins_isolated(path, Duration::from_secs(30));
    let entry = plugin_id
        .and_then(|id| {
            entries.iter().find(|entry| {
                entry.id == id
                    || legacy_path_plugin_id(
                        &entry.format,
                        &entry.path,
                        entry.class_uid.as_deref(),
                        entry.sub_plugin_id,
                        &entry.architecture,
                    ) == id
            })
        })
        .or_else(|| entries.iter().find(|entry| entry.supported))
        .or_else(|| entries.first())
        .cloned()
        .ok_or_else(|| "Plugin probe returned no classes".to_string())?;
    if entry.supported {
        Ok(entry)
    } else {
        Err(entry
            .unsupported_reason
            .clone()
            .unwrap_or_else(|| "Plugin is not supported in this app".to_string()))
    }
}

/// Handles the private subprocess mode used to inspect a plug-in without loading it into the
/// long-lived application or audio worker process.
pub fn probe_cli_exit_code() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new("--vst-probe")) {
        return None;
    }
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("Missing plug-in path for --vst-probe");
        return Some(2);
    };
    let entries = probe_plugins(&path);
    match serde_json::to_string(&entries) {
        Ok(json) => {
            println!("OSCMIDI_PROBE_JSON:{json}");
            Some(0)
        }
        Err(error) => {
            eprintln!("Failed to serialize VST probe result: {error}");
            Some(1)
        }
    }
}

pub fn smoke_vst2_instance(
    instance: &mut vst::host::PluginInstance,
    block_size: usize,
) -> Result<(usize, usize), String> {
    plugin_probe_vst2::smoke_vst2_instance(instance, block_size)
}

pub fn probe_plugin_isolated(path: &Path, timeout: Duration) -> VstPluginEntry {
    let entries = probe_plugins_isolated(path, timeout);
    entries
        .iter()
        .find(|entry| entry.supported)
        .cloned()
        .or_else(|| entries.into_iter().next())
        .unwrap_or_else(|| {
            unsupported_entry(
                path,
                plugin_name(path),
                if is_vst3_path(path) { "VST3" } else { "VST2" }.to_string(),
                "instrument".to_string(),
                "unknown".to_string(),
                "Probe returned no plugin classes".to_string(),
            )
        })
}

pub fn probe_plugins_isolated(path: &Path, timeout: Duration) -> Vec<VstPluginEntry> {
    match run_probe_process(path, timeout) {
        Ok(entries) => entries,
        Err(reason) => vec![unsupported_entry(
            path,
            plugin_name(path),
            if is_vst3_path(path) { "VST3" } else { "VST2" }.to_string(),
            "instrument".to_string(),
            "unknown".to_string(),
            reason,
        )],
    }
}

fn run_probe_process(path: &Path, timeout: Duration) -> Result<Vec<VstPluginEntry>, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Could not locate probe executable: {error}"))?;
    let architecture = plugin_probe_arch::detect_plugin_architecture(path);
    let dedicated_name = if cfg!(windows) {
        if architecture == "x86" {
            "vst-host-worker-x86.exe"
        } else {
            "vst-host-worker-x64.exe"
        }
    } else {
        "vst_probe"
    };
    let worker_override = if architecture == "x86" {
        std::env::var_os("OSCMIDI_VST_WORKER_X86_PATH")
    } else {
        std::env::var_os("OSCMIDI_VST_WORKER_X64_PATH")
    }
    .map(PathBuf::from);
    let dedicated = worker_override.unwrap_or_else(|| current_exe.with_file_name(dedicated_name));
    let (program, use_current_exe) = if dedicated.exists() && dedicated != current_exe {
        (dedicated, false)
    } else {
        (current_exe, true)
    };
    let program_label = program.display().to_string();
    let mut command = Command::new(program);
    command.env("OSCMIDI_VST_PROBE_TRACE", "1");
    if use_current_exe || dedicated_name.starts_with("vst-host-worker") {
        command.arg("--vst-probe");
    }
    // The probe runs whenever a VST is selected or the bridge starts. It has
    // captured stdio, so a console window would only be a distracting flash.
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to start isolated VST probe: {error}"))?;
    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "VST probe stdout pipe unavailable".to_string())?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "VST probe stderr pipe unavailable".to_string())?;
    let stdout_reader = std::thread::spawn(move || read_bounded_probe_stream(stdout_pipe));
    let stderr_reader = std::thread::spawn(move || read_bounded_probe_stream(stderr_pipe));
    let started = Instant::now();
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
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
    };
    let (stdout_bytes, stdout_exceeded) = stdout_reader
        .join()
        .map_err(|_| "VST probe stdout reader panicked".to_string())?;
    let (stderr_bytes, stderr_exceeded) = stderr_reader
        .join()
        .map_err(|_| "VST probe stderr reader panicked".to_string())?;
    if stdout_exceeded || stderr_exceeded {
        return Err(format!(
            "VST probe output exceeded the {} KiB per-stream safety limit",
            MAX_PROBE_STREAM_BYTES / 1024
        ));
    }
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let json = stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("OSCMIDI_PROBE_JSON:"))
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            let stderr = output_excerpt(&stderr);
            let stdout = output_excerpt(&stdout);
            let exit = exit_status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal or exception".to_string());
            format!(
                "VST probe exited without a result (worker: {program_label}, architecture: {architecture}, exit: {exit}); stderr: {stderr}; stdout: {stdout}"
            )
        })?;
    serde_json::from_str::<Vec<VstPluginEntry>>(json)
        .or_else(|_| serde_json::from_str::<VstPluginEntry>(json).map(|entry| vec![entry]))
        .map_err(|error| format!("Invalid VST probe result: {error}"))
}

fn read_bounded_probe_stream(reader: impl Read) -> (Vec<u8>, bool) {
    let mut bytes = Vec::new();
    let _ = reader
        .take(MAX_PROBE_STREAM_BYTES + 1)
        .read_to_end(&mut bytes);
    let exceeded = bytes.len() as u64 > MAX_PROBE_STREAM_BYTES;
    bytes.truncate(MAX_PROBE_STREAM_BYTES as usize);
    (bytes, exceeded)
}

fn output_excerpt(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    let excerpt = trimmed.chars().take(512).collect::<String>();
    if trimmed.chars().count() > 512 {
        format!("{excerpt}…")
    } else {
        excerpt
    }
}

pub fn probe_plugin(path: &Path) -> VstPluginEntry {
    probe_plugins(path).into_iter().next().unwrap_or_else(|| {
        unsupported_entry(
            path,
            plugin_name(path),
            "Unknown".to_string(),
            "other".to_string(),
            "unknown".to_string(),
            "Probe returned no plugin classes".to_string(),
        )
    })
}

pub fn probe_plugins(path: &Path) -> Vec<VstPluginEntry> {
    if is_vst3_path(path) {
        plugin_probe_vst3::probe_vst3_plugins(path)
    } else if is_vst2_path(path) {
        probe_vst2_plugins(path)
    } else {
        vec![unsupported_entry(
            path,
            plugin_name(path),
            "Unknown".to_string(),
            "other".to_string(),
            "unknown".to_string(),
            "Unsupported plugin path".to_string(),
        )]
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
    let status = if reason.contains("timed out") || reason.contains("without a result") {
        PluginStatus::Quarantined
    } else if format == "Unknown" {
        PluginStatus::Unsupported
    } else {
        PluginStatus::Failed
    };
    let failure_stage = if reason.contains("timed out") {
        "timeout"
    } else if reason.contains("architecture") || reason.contains("PE") {
        "architecture"
    } else if reason.contains("load failed") || reason.contains("not a VST") {
        "loading"
    } else if reason.contains("instantiate") || reason.contains("Instance") {
        "instantiation"
    } else if reason.contains("initial") {
        "initialization"
    } else if reason.contains("bus") || reason.contains("output") {
        "bus"
    } else if reason.contains("process") || reason.contains("smoke") {
        "firstProcess"
    } else if reason.contains("editor") {
        "editor"
    } else {
        "probe"
    };
    finalize_entry(VstPluginEntry {
        id: String::new(),
        name,
        path: path.to_string_lossy().to_string(),
        format,
        kind,
        architecture,
        available_architectures: Vec::new(),
        vendor: None,
        plugin_version: None,
        status,
        supported: false,
        unsupported_reason: Some(reason.clone()),
        failure_stage: Some(failure_stage.to_string()),
        last_error: Some(reason),
        last_probed_ms: Some(current_unix_ms()),
        midi_compatible: None,
        has_editor: false,
        channel_layout: None,
        class_uid: None,
        sub_plugin_id: None,
        hosting_mode: None,
        file_modified_ms,
        file_size,
        host_abi_version: VST_HOST_ABI_VERSION,
    })
}

pub(crate) fn finalize_entry(mut entry: VstPluginEntry) -> VstPluginEntry {
    if entry.last_probed_ms.is_none() {
        entry.last_probed_ms = Some(current_unix_ms());
    }
    entry.refresh_derived_fields();
    entry
}

pub(crate) fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
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
    fn probe_stream_reader_never_retains_more_than_the_limit() {
        let payload = vec![0x41; MAX_PROBE_STREAM_BYTES as usize + 32];
        let (bytes, exceeded) = read_bounded_probe_stream(std::io::Cursor::new(payload));

        assert!(exceeded);
        assert_eq!(bytes.len(), MAX_PROBE_STREAM_BYTES as usize);
    }

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

    #[test]
    fn stable_identity_changes_by_class_and_architecture() {
        let path = "C:/VST/Multi.vst3";
        let x64_a = crate::types::stable_plugin_id("VST3", path, Some("class-a"), None, "x64");
        let x64_a_again =
            crate::types::stable_plugin_id("vst3", path, Some("class-a"), None, "x64");
        let x64_b = crate::types::stable_plugin_id("VST3", path, Some("class-b"), None, "x64");
        let x86_a = crate::types::stable_plugin_id("VST3", path, Some("class-a"), None, "x86");
        assert_eq!(x64_a, x64_a_again);
        assert_ne!(x64_a, x64_b);
        assert_ne!(x64_a, x86_a);
    }

    #[test]
    fn stable_identity_survives_moves_when_the_plugin_has_an_intrinsic_id() {
        let original = crate::types::stable_plugin_id(
            "VST3",
            "C:/VST/Original/Multi.vst3",
            Some("class-a"),
            None,
            "x64",
        );
        let moved = crate::types::stable_plugin_id(
            "VST3",
            "D:/Audio/Moved/Multi.vst3",
            Some("class-a"),
            None,
            "x64",
        );
        let fallback_a =
            crate::types::stable_plugin_id("VST3", "C:/VST/Original/Multi.vst3", None, None, "x64");
        let fallback_b =
            crate::types::stable_plugin_id("VST3", "D:/Audio/Moved/Multi.vst3", None, None, "x64");
        assert_eq!(original, moved);
        assert_ne!(fallback_a, fallback_b);
        assert_ne!(
            original,
            crate::types::legacy_path_plugin_id(
                "VST3",
                "C:/VST/Original/Multi.vst3",
                Some("class-a"),
                None,
                "x64",
            )
        );
    }

    #[test]
    fn upright_state_fallback_is_limited_to_the_crashing_fingerprint() {
        let path = Path::new(r"C:\VST3\Upright Piano.vst3");
        assert!(vst3_native_state_requires_parameter_fallback(
            path,
            Some(UPRIGHT_PIANO_VST3_CLASS_UID),
            Some(UPRIGHT_PIANO_UNSAFE_STATE_FILE_SIZE),
        ));
        assert!(!vst3_native_state_requires_parameter_fallback(
            path,
            Some(UPRIGHT_PIANO_VST3_CLASS_UID),
            Some(UPRIGHT_PIANO_UNSAFE_STATE_FILE_SIZE + 1),
        ));
        assert!(!vst3_native_state_requires_parameter_fallback(
            path,
            Some("different-class"),
            Some(UPRIGHT_PIANO_UNSAFE_STATE_FILE_SIZE),
        ));
    }
}
