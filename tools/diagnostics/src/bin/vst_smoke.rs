#![allow(deprecated)]

//! Isolated smoke runner for validating third-party VST2 and VST3 binaries.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use osc_midi_bridge::plugin_probe;
use rack::vst3::Vst3Scanner;
use rack::{
    prelude::{
        MidiEvent as RackMidiEvent, PluginScanner as RackPluginScanner,
        PluginType as RackPluginType,
    },
    PluginInstance as RackPluginInstance,
};
use vst::host::{Host, PluginLoader};

struct SimpleHost;

impl Host for SimpleHost {}

#[cfg(target_os = "windows")]
#[link(name = "ole32")]
unsafe extern "system" {}

fn trace(message: &str) {
    if std::env::var_os("VST_SMOKE_TRACE").is_some() {
        eprintln!("{message}");
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(mode) = args.next() else {
        eprintln!("usage: vst_smoke --vst2 <path> | --vst3 <path> [--state <state-file>]");
        std::process::exit(2);
    };
    let Some(path) = args.next() else {
        eprintln!("missing plugin path");
        std::process::exit(2);
    };

    let mut state_path = None;
    while let Some(flag) = args.next() {
        if flag == "--state" {
            state_path = args.next();
        }
    }
    let result = match mode.as_str() {
        "--vst2" => smoke_vst2(Path::new(&path)),
        "--vst3" => smoke_vst3(Path::new(&path), state_path.as_deref().map(Path::new)),
        _ => {
            eprintln!("unknown mode: {}", mode);
            std::process::exit(2);
        }
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn smoke_vst2(path: &Path) -> Result<(), String> {
    trace(&format!("vst_smoke: probe VST2 {}", path.display()));
    let entry = plugin_probe::ensure_supported_plugin_in_app(path)?;
    let _ = (
        &entry.name,
        &entry.path,
        &entry.kind,
        &entry.architecture,
        &entry.midi_compatible,
        entry.has_editor,
        &entry.channel_layout,
    );
    if entry.format != "VST2" {
        return Err(format!(
            "VST2 smoke: expected a VST2 plugin, got {}",
            entry.format
        ));
    }

    let host = Arc::new(Mutex::new(SimpleHost));
    let load_path = plugin_probe::native_plugin_load_path(path);
    let mut loader = PluginLoader::load(&load_path, host)
        .map_err(|e| format!("VST2 smoke: failed to load plugin: {e}"))?;
    let mut instance = loader
        .instance()
        .map_err(|e| format!("VST2 smoke: failed to instantiate plugin: {e}"))?;
    plugin_probe::smoke_vst2_instance(&mut instance, 128)?;
    println!("VST2 smoke ok: {}", path.display());
    Ok(())
}

fn smoke_vst3(path: &Path, state_path: Option<&Path>) -> Result<(), String> {
    trace(&format!("vst_smoke: probe VST3 {}", path.display()));
    let entry = plugin_probe::ensure_supported_plugin_in_app(path)?;
    let _ = (
        &entry.name,
        &entry.path,
        &entry.kind,
        &entry.architecture,
        &entry.midi_compatible,
        entry.has_editor,
        &entry.channel_layout,
    );
    if entry.format != "VST3" {
        return Err(format!(
            "VST3 smoke: expected a VST3 plugin, got {}",
            entry.format
        ));
    }

    trace("vst_smoke: create VST3 scanner");
    let scanner =
        Vst3Scanner::new().map_err(|e| format!("VST3 smoke: failed to create scanner: {e}"))?;
    trace("vst_smoke: scan VST3 path");
    let scan_root = plugin_probe::vst3_scan_root_for_path(path);
    let plugins = scanner
        .scan_path(&scan_root)
        .map_err(|e| format!("VST3 smoke: failed to scan plugin path: {e}"))?;
    let info = plugin_probe::select_vst3_plugin_info_for_path(&plugins, path)
        .filter(|plugin| {
            matches!(
                plugin.plugin_type,
                RackPluginType::Instrument | RackPluginType::Effect
            )
        })
        .ok_or_else(|| "VST3 smoke: plugin info not found".to_string())?;
    trace("vst_smoke: load VST3 plugin");
    let mut plugin = scanner
        .load(&info)
        .map_err(|e| format!("VST3 smoke: failed to load plugin: {e}"))?;
    trace("vst_smoke: initialize VST3 plugin");
    plugin
        .initialize(48_000.0, 128)
        .map_err(|e| format!("VST3 smoke: initialize failed: {e}"))?;

    trace("vst_smoke: detect VST3 channels");
    let (input_channels, output_channels) =
        plugin_probe::detect_vst3_channels(&mut plugin, &info, 128)
            .map_err(|e| format!("VST3 smoke: channel probe failed: {e}"))?;
    let native_state_bytes = if plugin_probe::vst3_native_state_requires_parameter_fallback(
        Path::new(&entry.path),
        entry.class_uid.as_deref(),
        entry.file_size,
    ) {
        trace("vst_smoke: native state skipped by fingerprinted compatibility profile");
        0
    } else {
        match plugin.get_state() {
            Ok(state) => {
                plugin
                    .set_state(&state)
                    .map_err(|error| format!("VST3 smoke: native state restore failed: {error}"))?;
                state.len()
            }
            Err(error) => {
                trace(&format!(
                    "vst_smoke: native VST3 state unavailable; parameter fallback required: {error}"
                ));
                0
            }
        }
    };
    let peak = render_vst3_note(&mut plugin, input_channels, output_channels, 128)?;
    let mut restored_peak = None;
    let mut restored_parameters = 0usize;
    let mut state_rejected = None;
    if let Some(state_path) = state_path {
        plugin
            .reset()
            .map_err(|error| format!("VST3 smoke: reset failed: {error}"))?;
        match osc_midi_bridge::audio::restore_vst3_state_file_for_diagnostic(
            &mut plugin,
            &entry,
            state_path,
        ) {
            Ok(count) => {
                restored_parameters = count;
                restored_peak = Some(render_vst3_note(
                    &mut plugin,
                    input_channels,
                    output_channels,
                    128,
                )?);
            }
            Err(error) => {
                state_rejected = Some(error);
                restored_peak = Some(render_vst3_note(
                    &mut plugin,
                    input_channels,
                    output_channels,
                    128,
                )?);
            }
        }
    }
    println!(
        "VST3 smoke ok: {} (inputs={}, outputs={}, native_state_bytes={}, note_peak={:.6}, restored_note_peak={:?}, restored_parameters={}, state_rejected={:?})",
        path.display(),
        input_channels,
        output_channels,
        native_state_bytes,
        peak,
        restored_peak,
        restored_parameters,
        state_rejected,
    );
    if std::env::var_os("VST_SMOKE_FORGET_PLUGIN").is_some() {
        std::mem::forget(plugin);
        if std::env::var_os("VST_SMOKE_TRACE").is_some() {
            eprintln!("vst_smoke: forgot VST3 plugin");
        }
    } else {
        drop(plugin);
        if std::env::var_os("VST_SMOKE_TRACE").is_some() {
            eprintln!("vst_smoke: dropped VST3 plugin");
        }
    }
    drop(scanner);
    if std::env::var_os("VST_SMOKE_TRACE").is_some() {
        eprintln!("vst_smoke: dropped VST3 scanner");
    }
    Ok(())
}

fn render_vst3_note(
    plugin: &mut rack::vst3::Vst3Plugin,
    input_channels: usize,
    output_channels: usize,
    frames: usize,
) -> Result<f32, String> {
    plugin
        .send_midi(&[RackMidiEvent::note_on(60, 100, 0, 0)])
        .map_err(|error| format!("VST3 smoke: note-on failed: {error}"))?;

    let inputs = vec![vec![0.0f32; frames]; input_channels];
    let mut outputs = vec![vec![0.0f32; frames]; output_channels];
    let mut peak = 0.0f32;
    for _ in 0..256 {
        for output in &mut outputs {
            output.fill(0.0);
        }
        let input_slices = inputs.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let mut output_slices = outputs
            .iter_mut()
            .map(Vec::as_mut_slice)
            .collect::<Vec<_>>();
        plugin
            .process(&input_slices, &mut output_slices, frames)
            .map_err(|error| format!("VST3 smoke: note render failed: {error}"))?;
        peak = outputs
            .iter()
            .flatten()
            .fold(peak, |current, sample| current.max(sample.abs()));
    }
    plugin
        .send_midi(&[RackMidiEvent::note_off(60, 0, 0, 0)])
        .map_err(|error| format!("VST3 smoke: note-off failed: {error}"))?;
    Ok(peak)
}
