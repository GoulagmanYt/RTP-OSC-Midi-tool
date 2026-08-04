#![allow(deprecated)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use vst::api::Supported;
use vst::buffer::AudioBuffer;
use vst::host::{Host, PluginInstance, PluginLoader};
use vst::plugin::{CanDo, Category, Plugin};

use crate::types::VstPluginEntry;

use super::plugin_probe_arch::detect_plugin_architecture;
use super::plugin_probe_paths::plugin_name;

const VST2_PROBE_SAMPLE_RATE: f32 = 48_000.0;
const VST_PROBE_BLOCK_SIZE: usize = 128;

pub struct ScanHost;

impl Host for ScanHost {}

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

pub fn probe_vst2_plugin(path: &Path) -> VstPluginEntry {
    let architecture = detect_plugin_architecture(path);
    let name = plugin_name(path);
    let format = "VST2".to_string();

    if architecture == "x86" {
        return super::unsupported_entry(
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
            return super::unsupported_entry(
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
            return super::unsupported_entry(
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
    let class_uid = Some(format!("vst2:{:08x}", info.unique_id));
    let (file_modified_ms, file_size) = super::plugin_file_metadata(path);

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
            class_uid: class_uid.clone(),
            file_modified_ms,
            file_size,
            host_abi_version: 1,
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
            class_uid: class_uid.clone(),
            file_modified_ms,
            file_size,
            host_abi_version: 1,
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
            class_uid: class_uid.clone(),
            file_modified_ms,
            file_size,
            host_abi_version: 1,
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
            class_uid,
            file_modified_ms,
            file_size,
            host_abi_version: 1,
        },
    }
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
