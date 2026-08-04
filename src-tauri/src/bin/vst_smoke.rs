#![allow(deprecated)]

mod types {
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct VstPluginEntry {
        pub name: String,
        pub path: String,
        pub format: String,
        pub kind: String,
        pub architecture: String,
        pub supported: bool,
        pub unsupported_reason: Option<String>,
        pub midi_compatible: Option<bool>,
        pub has_editor: bool,
        pub channel_layout: Option<String>,
        pub class_uid: Option<String>,
        pub file_modified_ms: Option<u64>,
        pub file_size: Option<u64>,
        pub host_abi_version: u32,
    }
}
#[allow(dead_code)]
#[path = "../plugin_probe.rs"]
mod plugin_probe;

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rack::vst3::Vst3Scanner;
use rack::{
    prelude::{PluginScanner as RackPluginScanner, PluginType as RackPluginType},
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
        eprintln!("usage: vst_smoke --vst2 <path> | --vst3 <path>");
        std::process::exit(2);
    };
    let Some(path) = args.next() else {
        eprintln!("missing plugin path");
        std::process::exit(2);
    };

    let result = match mode.as_str() {
        "--vst2" => smoke_vst2(Path::new(&path)),
        "--vst3" => smoke_vst3(Path::new(&path)),
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
    let mut loader = PluginLoader::load(path, host)
        .map_err(|e| format!("VST2 smoke: failed to load plugin: {e}"))?;
    let mut instance = loader
        .instance()
        .map_err(|e| format!("VST2 smoke: failed to instantiate plugin: {e}"))?;
    plugin_probe::plugin_probe_vst2::smoke_vst2_instance(&mut instance, 128)?;
    println!("VST2 smoke ok: {}", path.display());
    Ok(())
}

fn smoke_vst3(path: &Path) -> Result<(), String> {
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
    let plugins = scanner
        .scan_path(path)
        .map_err(|e| format!("VST3 smoke: failed to scan plugin path: {e}"))?;
    let info = plugins
        .into_iter()
        .find(|plugin| {
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
    println!(
        "VST3 smoke ok: {} (inputs={}, outputs={})",
        path.display(),
        input_channels,
        output_channels
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
