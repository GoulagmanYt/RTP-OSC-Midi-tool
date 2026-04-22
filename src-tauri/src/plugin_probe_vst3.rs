use std::path::{Path, PathBuf};

use rack::vst3::Vst3Scanner;
use rack::PluginScanner as _;
use rack::{PluginInfo as RackPluginInfo, PluginType};

use crate::types::VstPluginEntry;

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

pub fn probe_vst3_plugin(path: &Path) -> VstPluginEntry {
    super::probe_trace(format!("probe_vst3_plugin: begin {}", path.display()));
    let architecture = detect_plugin_architecture(path);
    let format = "VST3".to_string();
    let fallback_name = plugin_name(path);

    if architecture == "x86" {
        return super::unsupported_entry(
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
            return super::unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                format!("VST3 scanner init failed: {err}"),
            )
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
            return super::unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                format!("VST3 scan failed: {err}"),
            )
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

    let info = match select_vst3_plugin_info_for_path(&plugins, path) {
        Some(info) => info,
        None => {
            return super::unsupported_entry(
                path,
                fallback_name,
                format,
                "other".to_string(),
                architecture,
                "VST3 plugin info not found for requested bundle".to_string(),
            )
        }
    };
    super::probe_trace(format!(
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
