use std::path::{Path, PathBuf};

use rack::vst3::Vst3Scanner;
use rack::PluginScanner as _;
use rack::{PluginInfo as RackPluginInfo, PluginInstance as _, PluginType};

use crate::types::{PluginStatus, VstPluginEntry};

use super::plugin_probe_arch::detect_plugin_architecture;
use super::plugin_probe_paths::{is_vst3_path, normalize_path, plugin_name};

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

pub fn probe_vst3_plugins(path: &Path) -> Vec<VstPluginEntry> {
    super::probe_trace(format!("probe_vst3_plugin: begin {}", path.display()));
    let architecture = detect_plugin_architecture(path);
    let format = "VST3".to_string();
    let fallback_name = plugin_name(path);

    let scanner = match Vst3Scanner::new() {
        Ok(scanner) => scanner,
        Err(err) => {
            return vec![super::unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                format!("VST3 scanner init failed: {err}"),
            )]
        }
    };
    super::probe_trace("probe_vst3_plugin: scanner created");

    let scan_root = vst3_scan_root_for_path(path);
    super::probe_trace(format!(
        "probe_vst3_plugin: scan root {}",
        scan_root.display()
    ));
    let plugins: Vec<RackPluginInfo> = match scanner.scan_path(&scan_root) {
        Ok(plugins) => plugins,
        Err(err) => {
            return vec![super::unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                format!("VST3 scan failed: {err}"),
            )]
        }
    };
    super::probe_trace(format!(
        "probe_vst3_plugin: scan_path returned {}",
        plugins.len()
    ));
    for plugin in &plugins {
        super::probe_trace(format!(
            "probe_vst3_plugin: candidate {} {:?} {}",
            plugin.name,
            plugin.plugin_type,
            plugin.path.display()
        ));
    }

    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut matching = plugins
        .into_iter()
        .filter(|info| vst3_path_match_score(&target, &info.path).is_some())
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return vec![super::unsupported_entry(
            path,
            fallback_name,
            format,
            "other".to_string(),
            architecture,
            "VST3 plugin info not found for requested bundle".to_string(),
        )];
    }
    matching.sort_by(|a, b| {
        vst3_plugin_type_rank(a.plugin_type)
            .cmp(&vst3_plugin_type_rank(b.plugin_type))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let mut seen_classes = std::collections::HashSet::new();
    matching.retain(|info| seen_classes.insert(info.unique_id.clone()));
    matching
        .into_iter()
        .map(|info| {
            let probe_result = if info.plugin_type == PluginType::Instrument {
                probe_vst3_runtime(&scanner, &info, path)
            } else {
                Ok((0, 0, false))
            };
            let mut entry = vst3_entry_from_info(path, &architecture, info);
            if entry.kind == "instrument" {
                match probe_result {
                    Ok((inputs, outputs, has_editor)) => {
                        entry.status = PluginStatus::Compatible;
                        entry.supported = true;
                        entry.has_editor = has_editor;
                        entry.channel_layout = Some(format!("{inputs} in / {outputs} out"));
                        entry.failure_stage = None;
                        entry.last_error = None;
                        entry.unsupported_reason = None;
                    }
                    Err((stage, error)) => {
                        entry.status = PluginStatus::Failed;
                        entry.supported = false;
                        entry.failure_stage = Some(stage.to_string());
                        entry.last_error = Some(error.clone());
                        entry.unsupported_reason = Some(error);
                    }
                }
                entry.refresh_derived_fields();
            }
            entry
        })
        .collect()
}

fn probe_vst3_runtime(
    scanner: &Vst3Scanner,
    info: &RackPluginInfo,
    path: &Path,
) -> Result<(usize, usize, bool), (&'static str, String)> {
    super::probe_trace(format!("probe_vst3_plugin: instantiate {}", info.unique_id));
    let mut plugin = scanner
        .load(info)
        .map_err(|error| ("instantiation", error.to_string()))?;
    super::probe_trace("probe_vst3_plugin: initialize");
    plugin
        .initialize(48_000.0, 128)
        .map_err(|error| ("initialization", error.to_string()))?;
    let inputs = plugin.input_channels();
    let outputs = plugin.output_channels();
    if outputs == 0 {
        return Err((
            "bus",
            "VST3 instrument has no active main output".to_string(),
        ));
    }
    let input_storage = vec![vec![0.0f32; 128]; inputs];
    let mut output_storage = vec![vec![0.0f32; 128]; outputs];
    let input_refs = input_storage.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let mut output_refs = output_storage
        .iter_mut()
        .map(Vec::as_mut_slice)
        .collect::<Vec<_>>();
    super::probe_trace("probe_vst3_plugin: first process");
    plugin
        .process(&input_refs, &mut output_refs, 128)
        .map_err(|error| ("firstProcess", error.to_string()))?;
    super::probe_trace("probe_vst3_plugin: state and editor capabilities");
    let (_, file_size) = super::plugin_file_metadata(path);
    if super::vst3_native_state_requires_parameter_fallback(
        path,
        Some(info.unique_id.as_str()),
        file_size,
    ) {
        super::probe_trace(
            "probe_vst3_plugin: native state skipped by fingerprinted compatibility profile",
        );
    } else {
        let _ = plugin.get_state();
    }
    let has_editor = plugin.create_gui().is_ok();

    if crate::audio::engine::requires_vst3_destructor_quarantine(
        path,
        Some(info.unique_id.as_str()),
    ) {
        super::probe_trace("probe_vst3_plugin: quarantining known destructor");
        std::mem::forget(plugin);
    }
    Ok((inputs, outputs, has_editor))
}

fn vst3_entry_from_info(path: &Path, architecture: &str, info: RackPluginInfo) -> VstPluginEntry {
    let format = "VST3".to_string();

    let kind = match info.plugin_type {
        PluginType::Instrument => "instrument",
        PluginType::Effect => "effect",
        _ => "other",
    }
    .to_string();
    let has_editor = kind == "instrument";
    let channel_layout = None;
    let class_uid = Some(info.unique_id.clone());
    let (file_modified_ms, file_size) = super::plugin_file_metadata(path);

    if kind != "instrument" {
        return super::finalize_entry(VstPluginEntry {
            id: String::new(),
            name: info.name,
            path: path.to_string_lossy().to_string(),
            format,
            kind,
            architecture: architecture.to_string(),
            available_architectures: Vec::new(),
            vendor: if info.manufacturer.is_empty() {
                None
            } else {
                Some(info.manufacturer.clone())
            },
            plugin_version: Some(info.version.to_string()),
            status: PluginStatus::OutOfScope,
            supported: false,
            unsupported_reason: Some("Effect plugin ignored".to_string()),
            failure_stage: Some("classification".to_string()),
            last_error: Some("Effect plugin ignored".to_string()),
            last_probed_ms: None,
            midi_compatible: None,
            has_editor,
            channel_layout,
            class_uid,
            sub_plugin_id: None,
            hosting_mode: None,
            file_modified_ms,
            file_size,
            host_abi_version: crate::types::VST_HOST_ABI_VERSION,
        });
    }

    super::finalize_entry(VstPluginEntry {
        id: String::new(),
        name: info.name,
        path: path.to_string_lossy().to_string(),
        format,
        kind,
        architecture: architecture.to_string(),
        available_architectures: Vec::new(),
        vendor: if info.manufacturer.is_empty() {
            None
        } else {
            Some(info.manufacturer.clone())
        },
        plugin_version: Some(info.version.to_string()),
        status: PluginStatus::Compatible,
        supported: true,
        unsupported_reason: None,
        failure_stage: None,
        last_error: None,
        last_probed_ms: None,
        midi_compatible: Some(true),
        has_editor,
        channel_layout,
        class_uid,
        sub_plugin_id: None,
        hosting_mode: None,
        file_modified_ms,
        file_size,
        host_abi_version: crate::types::VST_HOST_ABI_VERSION,
    })
}

pub fn vst3_scan_root_for_path(path: &Path) -> PathBuf {
    let native_path = super::plugin_probe_paths::native_plugin_load_path(path);
    let path = native_path.as_path();
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
