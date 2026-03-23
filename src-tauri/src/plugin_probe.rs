#![allow(deprecated)]

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rack::vst3::Vst3Scanner;
use rack::{PluginInfo as RackPluginInfo, PluginScanner as RackPluginScanner, PluginType};
use vst::api::Supported;
use vst::buffer::AudioBuffer;
use vst::host::{Host, PluginInstance, PluginLoader};
use vst::plugin::{CanDo, Category, Plugin};

use crate::types::VstPluginEntry;

pub const DEFAULT_VST3_PLUGIN_DIR: &str = "C:\\Program Files\\Common Files\\VST3";
pub const DEFAULT_VST2_PLUGIN_DIR: &str = "C:\\Program Files\\VSTPlugins";
pub const STEINBERG_VST2_PLUGIN_DIR: &str = "C:\\Program Files\\Steinberg\\VstPlugins";
pub const COMMON_VST2_PLUGIN_DIR: &str = "C:\\Program Files\\Common Files\\VST2";
pub const COMMON_STEINBERG_VST2_PLUGIN_DIR: &str =
    "C:\\Program Files\\Common Files\\Steinberg\\VST2";
pub const DEFAULT_VST3_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\Common Files\\VST3";
pub const DEFAULT_VST2_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\VSTPlugins";
pub const STEINBERG_VST2_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\Steinberg\\VstPlugins";
pub const COMMON_VST2_PLUGIN_DIR_X86: &str = "C:\\Program Files (x86)\\Common Files\\VST2";
pub const COMMON_STEINBERG_VST2_PLUGIN_DIR_X86: &str =
    "C:\\Program Files (x86)\\Common Files\\Steinberg\\VST2";

const VST2_PROBE_SAMPLE_RATE: f32 = 48_000.0;
const VST_PROBE_BLOCK_SIZE: usize = 128;

fn probe_trace(message: impl AsRef<str>) {
    if std::env::var_os("OSCMIDI_VST_PROBE_TRACE").is_some() {
        eprintln!("{}", message.as_ref());
    }
}

pub fn default_vst_scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for raw in [
        DEFAULT_VST3_PLUGIN_DIR,
        DEFAULT_VST2_PLUGIN_DIR,
        STEINBERG_VST2_PLUGIN_DIR,
        COMMON_VST2_PLUGIN_DIR,
        COMMON_STEINBERG_VST2_PLUGIN_DIR,
        DEFAULT_VST3_PLUGIN_DIR_X86,
        DEFAULT_VST2_PLUGIN_DIR_X86,
        STEINBERG_VST2_PLUGIN_DIR_X86,
        COMMON_VST2_PLUGIN_DIR_X86,
        COMMON_STEINBERG_VST2_PLUGIN_DIR_X86,
    ] {
        let path = PathBuf::from(raw);
        if path.exists() && !roots.iter().any(|existing| existing == &path) {
            roots.push(path);
        }
    }
    roots
}

pub fn is_vst2_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("dll"))
        .unwrap_or(false)
}

pub fn is_vst3_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("vst3"))
        .unwrap_or(false)
}

pub fn scan_vst_plugins_in_roots(roots: &[PathBuf]) -> Vec<VstPluginEntry> {
    let mut entries = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in roots {
        for path in scan_plugin_candidates(root) {
            let key = normalize_path(&path);
            if seen_paths.insert(key) {
                let entry = probe_plugin(&path);
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
    let entry = probe_plugin(path);
    if entry.supported {
        Ok(entry)
    } else {
        Err(entry
            .unsupported_reason
            .clone()
            .unwrap_or_else(|| "Plugin is not supported in this app".to_string()))
    }
}

pub fn probe_plugin(path: &Path) -> VstPluginEntry {
    if is_vst3_path(path) {
        probe_vst3_plugin(path)
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

pub fn detect_vst3_channels(
    plugin: &mut rack::vst3::Vst3Plugin,
    _info: &RackPluginInfo,
    _max_block_size: usize,
) -> Result<(usize, usize), String> {
    let input_channels = plugin.input_channels();
    let output_channels = plugin.output_channels();

    if output_channels == 0 {
        return Err("VST3 plugin has no output buses".to_string());
    }

    Ok((input_channels, output_channels))
}

pub fn smoke_vst2_instance(
    instance: &mut PluginInstance,
    frames: usize,
) -> Result<(usize, usize), String> {
    instance.init();
    instance.set_sample_rate(VST2_PROBE_SAMPLE_RATE);
    instance.set_block_size(frames as i64);
    instance.resume();

    let info = instance.get_info();
    if info.outputs <= 0 {
        return Err("VST2 plugin has no output buses".to_string());
    }

    let input_channels = info.inputs.max(0) as usize;
    let output_channels = info.outputs.max(1) as usize;
    let input_buffers = vec![vec![0.0f32; frames]; input_channels];
    let mut output_buffers = vec![vec![0.0f32; frames]; output_channels];
    let input_ptrs: Vec<*const f32> = input_buffers.iter().map(|buf| buf.as_ptr()).collect();
    let mut output_ptrs: Vec<*mut f32> = output_buffers
        .iter_mut()
        .map(|buf| buf.as_mut_ptr())
        .collect();
    let mut buffer = unsafe {
        AudioBuffer::from_raw(
            input_channels,
            output_channels,
            input_ptrs.as_ptr(),
            output_ptrs.as_mut_ptr(),
            frames,
        )
    };

    instance.process(&mut buffer);
    Ok((input_channels, output_channels))
}

fn scan_plugin_candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    scan_dir(root, &mut out);
    out
}

fn scan_dir(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read_dir) = fs::read_dir(root) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if is_vst3_path(&path) {
                out.push(path);
            } else {
                scan_dir(&path, out);
            }
            continue;
        }

        if is_vst2_path(&path) || is_vst3_path(&path) {
            out.push(path);
        }
    }
}

fn probe_vst2_plugin(path: &Path) -> VstPluginEntry {
    let architecture = detect_plugin_architecture(path);
    let name = plugin_name(path);
    let format = "VST2".to_string();

    if architecture == "x86" {
        return unsupported_entry(
            path,
            name,
            format,
            "instrument".to_string(),
            architecture,
            "32-bit plugin detected; bridge not implemented".to_string(),
        );
    }

    let host = Arc::new(Mutex::new(ScanHost));
    let mut loader = match PluginLoader::load(path, host) {
        Ok(loader) => loader,
        Err(err) => {
            return unsupported_entry(
                path,
                name,
                format,
                "other".to_string(),
                architecture,
                format!("VST2 load failed: {err}"),
            )
        }
    };

    let mut instance = match loader.instance() {
        Ok(instance) => instance,
        Err(err) => {
            return unsupported_entry(
                path,
                name,
                format,
                "other".to_string(),
                architecture,
                format!("VST2 instantiate failed: {err}"),
            )
        }
    };

    let info = instance.get_info();
    let plugin_kind = classify_vst2_kind(&instance, &info);
    let midi_compatible = vst2_receives_midi(&instance);
    let has_editor = instance.get_editor().is_some();
    let channel_layout = Some(format!(
        "{} in / {} out",
        info.inputs.max(0),
        info.outputs.max(0)
    ));

    if plugin_kind != "instrument" {
        return VstPluginEntry {
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            supported: false,
            unsupported_reason: Some("Effect plugin ignored".to_string()),
            midi_compatible: Some(midi_compatible),
            has_editor,
            channel_layout,
        };
    }

    if !midi_compatible {
        return VstPluginEntry {
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            supported: false,
            unsupported_reason: Some(
                "Instrument does not advertise MIDI input support".to_string(),
            ),
            midi_compatible: Some(false),
            has_editor,
            channel_layout,
        };
    }

    let smoke = smoke_vst2_instance(&mut instance, VST_PROBE_BLOCK_SIZE);
    match smoke {
        Ok((inputs, outputs)) => VstPluginEntry {
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            supported: true,
            unsupported_reason: None,
            midi_compatible: Some(true),
            has_editor,
            channel_layout: Some(format!("{inputs} in / {outputs} out")),
        },
        Err(err) => VstPluginEntry {
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            supported: false,
            unsupported_reason: Some(format!("VST2 smoke failed: {err}")),
            midi_compatible: Some(true),
            has_editor,
            channel_layout,
        },
    }
}

fn probe_vst3_plugin(path: &Path) -> VstPluginEntry {
    probe_trace(format!("probe_vst3_plugin: begin {}", path.display()));
    let architecture = detect_plugin_architecture(path);
    let format = "VST3".to_string();
    let fallback_name = plugin_name(path);

    if architecture == "x86" {
        return unsupported_entry(
            path,
            fallback_name,
            format,
            "instrument".to_string(),
            architecture,
            "32-bit plugin detected; bridge not implemented".to_string(),
        );
    }

    let scanner = match Vst3Scanner::new() {
        Ok(scanner) => scanner,
        Err(err) => {
            return unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                format!("VST3 scanner init failed: {err}"),
            )
        }
    };
    probe_trace("probe_vst3_plugin: scanner created");

    let scan_root = vst3_scan_root_for_path(path);
    probe_trace(format!(
        "probe_vst3_plugin: scan root {}",
        scan_root.display()
    ));
    let plugins = match scanner.scan_path(&scan_root) {
        Ok(plugins) => plugins,
        Err(err) => {
            return unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                format!("VST3 scan failed: {err}"),
            )
        }
    };
    probe_trace(format!(
        "probe_vst3_plugin: scan_path returned {}",
        plugins.len()
    ));
    for plugin in &plugins {
        probe_trace(format!(
            "probe_vst3_plugin: candidate {} {:?} {}",
            plugin.name,
            plugin.plugin_type,
            plugin.path.display()
        ));
    }

    let info = match select_vst3_plugin_info_for_path(&plugins, path) {
        Some(info) => info,
        None => {
            return unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                "VST3 plugin info not found for requested bundle".to_string(),
            )
        }
    };
    probe_trace(format!(
        "probe_vst3_plugin: matched info {} ({})",
        info.name,
        info.path.display()
    ));

    let kind = match info.plugin_type {
        PluginType::Instrument => "instrument",
        PluginType::Effect => "effect",
        _ => "other",
    }
    .to_string();
    let has_editor = kind == "instrument";
    let channel_layout = None;

    if kind != "instrument" {
        return VstPluginEntry {
            name: info.name,
            path: path.to_string_lossy().to_string(),
            format,
            kind,
            architecture,
            supported: false,
            unsupported_reason: Some("Effect plugin ignored".to_string()),
            midi_compatible: None,
            has_editor,
            channel_layout,
        };
    }

    VstPluginEntry {
        name: info.name,
        path: path.to_string_lossy().to_string(),
        format,
        kind,
        architecture,
        supported: true,
        unsupported_reason: None,
        midi_compatible: Some(true),
        has_editor,
        channel_layout,
    }
}

fn unsupported_entry(
    path: &Path,
    name: String,
    format: String,
    kind: String,
    architecture: String,
    reason: String,
) -> VstPluginEntry {
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
    }
}

fn plugin_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn classify_vst2_kind(instance: &PluginInstance, info: &vst::plugin::Info) -> String {
    if is_vst2_instrument_category(info.category) || is_vst2_midi_instrument_like(instance, info) {
        "instrument".to_string()
    } else if matches!(info.category, Category::Effect) {
        "effect".to_string()
    } else {
        "other".to_string()
    }
}

fn is_vst2_instrument_category(category: Category) -> bool {
    matches!(category, Category::Synth | Category::Generator)
}

fn is_vst2_midi_instrument_like(instance: &PluginInstance, info: &vst::plugin::Info) -> bool {
    if info.outputs <= 0 || info.inputs > 0 {
        return false;
    }
    vst2_receives_midi(instance)
}

fn vst2_receives_midi(instance: &PluginInstance) -> bool {
    matches!(
        instance.can_do(CanDo::ReceiveMidiEvent),
        Supported::Yes | Supported::Maybe
    ) || matches!(
        instance.can_do(CanDo::ReceiveEvents),
        Supported::Yes | Supported::Maybe
    )
}

fn normalize_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

pub fn vst3_scan_root_for_path(path: &Path) -> PathBuf {
    if is_vst3_path(path) {
        return path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if path.is_file() {
        return path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }
    path.to_path_buf()
}

pub fn select_vst3_plugin_info_for_path(
    plugins: &[RackPluginInfo],
    target_path: &Path,
) -> Option<RackPluginInfo> {
    let target = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.to_path_buf());

    plugins
        .iter()
        .filter_map(|plugin| {
            vst3_path_match_score(&target, &plugin.path).map(|path_score| {
                (
                    path_score,
                    vst3_plugin_type_rank(plugin.plugin_type),
                    plugin.name.to_lowercase(),
                    plugin.clone(),
                )
            })
        })
        .min_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        })
        .map(|(_, _, _, plugin)| plugin)
}

fn vst3_path_match_score(target: &Path, candidate: &Path) -> Option<u8> {
    let target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    let candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());

    if candidate == target {
        return Some(0);
    }
    if candidate.starts_with(&target) {
        return Some(1);
    }
    if target.starts_with(&candidate) {
        return Some(2);
    }

    let target_name = target.file_name().and_then(|value| value.to_str());
    let candidate_name = candidate.file_name().and_then(|value| value.to_str());
    if target_name
        .zip(candidate_name)
        .map(|(left, right)| left.eq_ignore_ascii_case(right))
        .unwrap_or(false)
    {
        return Some(3);
    }

    let normalized_target = normalize_path(&target);
    let normalized_candidate = normalize_path(&candidate);
    if normalized_candidate.contains(&normalized_target)
        || normalized_target.contains(&normalized_candidate)
    {
        return Some(4);
    }

    None
}

fn vst3_plugin_type_rank(plugin_type: PluginType) -> u8 {
    match plugin_type {
        PluginType::Instrument => 0,
        PluginType::Effect => 1,
        PluginType::Analyzer => 2,
        PluginType::Spatial => 3,
        _ => 4,
    }
}

fn detect_plugin_architecture(path: &Path) -> String {
    if is_vst2_path(path) {
        return read_pe_machine(path).unwrap_or("unknown".to_string());
    }
    if is_vst3_path(path) {
        return detect_vst3_architecture(path);
    }
    "unknown".to_string()
}

fn detect_vst3_architecture(path: &Path) -> String {
    if path.is_file() {
        return read_pe_machine(path).unwrap_or("unknown".to_string());
    }

    let x64_dir = path.join("Contents").join("x86_64-win");
    if x64_dir.exists() {
        return "x64".to_string();
    }
    let x86_dir = path.join("Contents").join("x86-win");
    if x86_dir.exists() {
        return "x86".to_string();
    }

    if let Some(binary) = find_first_vst3_binary(path) {
        return read_pe_machine(&binary).unwrap_or("unknown".to_string());
    }

    "unknown".to_string()
}

fn find_first_vst3_binary(path: &Path) -> Option<PathBuf> {
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }
            if is_vst3_path(&entry_path) {
                return Some(entry_path);
            }
        }
    }
    None
}

fn read_pe_machine(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut mz = [0u8; 2];
    file.read_exact(&mut mz).ok()?;
    if &mz != b"MZ" {
        return None;
    }

    file.seek(SeekFrom::Start(0x3c)).ok()?;
    let mut pe_offset = [0u8; 4];
    file.read_exact(&mut pe_offset).ok()?;
    let pe_offset = u32::from_le_bytes(pe_offset) as u64;

    file.seek(SeekFrom::Start(pe_offset)).ok()?;
    let mut signature = [0u8; 4];
    file.read_exact(&mut signature).ok()?;
    if &signature != b"PE\0\0" {
        return None;
    }

    let mut machine = [0u8; 2];
    file.read_exact(&mut machine).ok()?;
    Some(match u16::from_le_bytes(machine) {
        0x14c => "x86".to_string(),
        0x8664 => "x64".to_string(),
        value => format!("0x{value:04X}"),
    })
}

struct ScanHost;

impl Host for ScanHost {}

#[cfg(test)]
mod tests {
    use super::*;
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
        let pe = dir.path().join("test.dll");
        let mut bytes = vec![0u8; 512];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        bytes[0x3c] = 0x80;
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        fs::write(&pe, bytes).expect("write pe");

        assert_eq!(read_pe_machine(&pe).as_deref(), Some("x64"));
    }

    #[test]
    fn read_pe_machine_detects_x86() {
        let dir = tempdir().expect("temp dir");
        let pe = dir.path().join("test32.dll");
        let mut bytes = vec![0u8; 512];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        bytes[0x3c] = 0x80;
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        fs::write(&pe, bytes).expect("write pe");

        assert_eq!(read_pe_machine(&pe).as_deref(), Some("x86"));
        assert_eq!(detect_plugin_architecture(&pe), "x86");
    }

    #[test]
    fn detect_vst3_architecture_from_bundle_layout() {
        let dir = tempdir().expect("temp dir");
        let vst3 = dir.path().join("TestPiano.vst3");
        let bundle_binary = vst3
            .join("Contents")
            .join("x86_64-win")
            .join("TestPiano.vst3");
        fs::create_dir_all(bundle_binary.parent().expect("bundle dir")).expect("bundle");
        fs::write(&bundle_binary, b"dummy").expect("binary");

        assert_eq!(detect_plugin_architecture(&vst3), "x64");
    }

    #[test]
    fn matches_vst3_bundle_path_accepts_bundle_binary() {
        let dir = tempdir().expect("temp dir");
        let bundle = dir.path().join("TestPiano.vst3");
        let binary = bundle
            .join("Contents")
            .join("x86_64-win")
            .join("TestPiano.vst3");
        fs::create_dir_all(binary.parent().expect("binary dir")).expect("mkdirs");
        fs::write(&binary, b"dummy").expect("write");

        assert!(vst3_path_match_score(&bundle, &binary).is_some());
        assert!(vst3_path_match_score(&binary, &bundle).is_some());
    }

    #[test]
    fn select_vst3_plugin_info_prefers_instrument_for_same_bundle() {
        let dir = tempdir().expect("temp dir");
        let bundle = dir.path().join("TestPiano.vst3");
        fs::create_dir_all(&bundle).expect("bundle dir");

        let effect = RackPluginInfo::new(
            "TestPiano FX".to_string(),
            "Vendor".to_string(),
            1,
            PluginType::Effect,
            bundle.clone(),
            "FXUID".to_string(),
        );
        let instrument = RackPluginInfo::new(
            "TestPiano Instrument".to_string(),
            "Vendor".to_string(),
            1,
            PluginType::Instrument,
            bundle.clone(),
            "INSUID".to_string(),
        );

        let selected =
            select_vst3_plugin_info_for_path(&[effect, instrument], &bundle).expect("selection");
        assert_eq!(selected.plugin_type, PluginType::Instrument);
    }

    #[test]
    fn unknown_paths_return_unsupported_entry() {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("readme.txt");
        fs::write(&path, b"not a plugin").expect("write");

        let entry = probe_plugin(&path);
        assert!(!entry.supported);
        assert_eq!(entry.format, "Unknown");
        assert_eq!(entry.architecture, "unknown");
        assert!(entry.unsupported_reason.is_some());
    }
}
