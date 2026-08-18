#![allow(deprecated)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use vst::api::Supported;
use vst::buffer::AudioBuffer;
use vst::host::{Host, PluginInstance, PluginLoader};
use vst::plugin::{CanDo, Category, Plugin};

use crate::types::{PluginStatus, VstPluginEntry};

use super::plugin_probe_arch::detect_plugin_architecture;
use super::plugin_probe_paths::plugin_name;

const VST2_PROBE_SAMPLE_RATE: f32 = 48_000.0;
const VST_PROBE_BLOCK_SIZE: usize = 128;
const CHURCH_ORGAN_VST2_UNIQUE_ID: i32 = 0x3576_5a5b;

pub(crate) fn requires_vst2_destructor_quarantine(unique_id: i32) -> bool {
    unique_id == CHURCH_ORGAN_VST2_UNIQUE_ID
}

pub struct ScanHost {
    plugin_id: i32,
}

impl Host for ScanHost {
    fn get_plugin_id(&self) -> i32 {
        self.plugin_id
    }
}

pub fn smoke_vst2_instance(
    instance: &mut PluginInstance,
    frames: usize,
) -> Result<(usize, usize), String> {
    instance.init();
    instance.set_sample_rate(VST2_PROBE_SAMPLE_RATE);
    instance.set_block_size(frames as i64);
    instance.resume();
    instance.start_process();

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
    instance.stop_process();
    instance.suspend();
    Ok((input_channels, output_channels))
}

pub fn probe_vst2_plugins(path: &Path) -> Vec<VstPluginEntry> {
    let primary = probe_vst2_plugin(path);
    if primary.kind != "shell" {
        return vec![primary];
    }
    enumerate_vst2_shell(path, &primary)
}

pub fn probe_vst2_plugin(path: &Path) -> VstPluginEntry {
    super::probe_trace(format!("probe_vst2_plugin: begin {}", path.display()));
    let architecture = detect_plugin_architecture(path);
    let name = plugin_name(path);
    let format = "VST2".to_string();

    let host = Arc::new(Mutex::new(ScanHost { plugin_id: 0 }));
    let load_path = super::plugin_probe_paths::native_plugin_load_path(path);
    let mut loader = match PluginLoader::load(&load_path, host) {
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
    super::probe_trace("probe_vst2_plugin: module loaded");

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
    super::probe_trace("probe_vst2_plugin: instance created");

    let info = instance.get_info();
    super::probe_trace(format!(
        "probe_vst2_plugin: info id={:08x} inputs={} outputs={}",
        info.unique_id, info.inputs, info.outputs
    ));
    if matches!(info.category, Category::Shell) {
        let (file_modified_ms, file_size) = super::plugin_file_metadata(path);
        return super::finalize_entry(VstPluginEntry {
            id: String::new(),
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: "shell".to_string(),
            architecture,
            available_architectures: Vec::new(),
            vendor: (!info.vendor.is_empty()).then(|| info.vendor.clone()),
            plugin_version: Some(info.version.to_string()),
            status: PluginStatus::OutOfScope,
            supported: false,
            unsupported_reason: Some("VST2 Shell container; select one of its classes".to_string()),
            failure_stage: Some("classification".to_string()),
            last_error: None,
            last_probed_ms: None,
            midi_compatible: None,
            has_editor: false,
            channel_layout: None,
            class_uid: None,
            sub_plugin_id: None,
            hosting_mode: None,
            file_modified_ms,
            file_size,
            host_abi_version: crate::types::VST_HOST_ABI_VERSION,
        });
    }
    let plugin_kind = classify_vst2_kind(&instance, &info);
    let midi_compatible = vst2_receives_midi(&instance);
    let editor = instance.get_editor();
    let has_editor = editor.is_some();
    if requires_vst2_destructor_quarantine(info.unique_id) {
        // The probe process exits immediately after serialization. Keep this
        // legacy editor wrapper alive until process teardown because its native
        // release path can raise an access violation.
        std::mem::forget(editor);
    }
    let channel_layout = Some(format!(
        "{} in / {} out",
        info.inputs.max(0),
        info.outputs.max(0)
    ));
    let class_uid = Some(format!("vst2:{:08x}", info.unique_id));
    let (file_modified_ms, file_size) = super::plugin_file_metadata(path);

    if plugin_kind != "instrument" {
        return super::finalize_entry(VstPluginEntry {
            id: String::new(),
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            available_architectures: Vec::new(),
            vendor: if info.vendor.is_empty() {
                None
            } else {
                Some(info.vendor.clone())
            },
            plugin_version: Some(info.version.to_string()),
            status: PluginStatus::OutOfScope,
            supported: false,
            unsupported_reason: Some("Effect plugin ignored".to_string()),
            failure_stage: Some("classification".to_string()),
            last_error: Some("Effect plugin ignored".to_string()),
            last_probed_ms: None,
            midi_compatible: Some(midi_compatible),
            has_editor,
            channel_layout,
            class_uid: class_uid.clone(),
            sub_plugin_id: None,
            hosting_mode: None,
            file_modified_ms,
            file_size,
            host_abi_version: crate::types::VST_HOST_ABI_VERSION,
        });
    }

    super::probe_trace("probe_vst2_plugin: first process begin");
    let smoke = smoke_vst2_instance(&mut instance, VST_PROBE_BLOCK_SIZE);
    super::probe_trace("probe_vst2_plugin: first process complete");
    let entry = match smoke {
        Ok((inputs, outputs)) => super::finalize_entry(VstPluginEntry {
            id: String::new(),
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            available_architectures: Vec::new(),
            vendor: if info.vendor.is_empty() {
                None
            } else {
                Some(info.vendor.clone())
            },
            plugin_version: Some(info.version.to_string()),
            status: if midi_compatible {
                PluginStatus::Compatible
            } else {
                PluginStatus::Unverified
            },
            supported: true,
            unsupported_reason: (!midi_compatible).then(|| {
                "Instrument does not advertise MIDI input support; processing probe succeeded"
                    .to_string()
            }),
            failure_stage: (!midi_compatible).then(|| "midiCapability".to_string()),
            last_error: None,
            last_probed_ms: None,
            midi_compatible: Some(midi_compatible),
            has_editor,
            channel_layout: Some(format!("{inputs} in / {outputs} out")),
            class_uid: class_uid.clone(),
            sub_plugin_id: None,
            hosting_mode: None,
            file_modified_ms,
            file_size,
            host_abi_version: crate::types::VST_HOST_ABI_VERSION,
        }),
        Err(err) => super::finalize_entry(VstPluginEntry {
            id: String::new(),
            name: if info.name.is_empty() {
                name
            } else {
                info.name.clone()
            },
            path: path.to_string_lossy().to_string(),
            format,
            kind: plugin_kind,
            architecture,
            available_architectures: Vec::new(),
            vendor: if info.vendor.is_empty() {
                None
            } else {
                Some(info.vendor.clone())
            },
            plugin_version: Some(info.version.to_string()),
            status: PluginStatus::Failed,
            supported: false,
            unsupported_reason: Some(format!("VST2 smoke failed: {err}")),
            failure_stage: Some("firstProcess".to_string()),
            last_error: Some(err),
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
        }),
    };

    if requires_vst2_destructor_quarantine(info.unique_id) {
        super::probe_trace("probe_vst2_plugin: quarantining Church Organ probe destructor");
        std::mem::forget(instance);
        std::mem::forget(loader);
    }
    super::probe_trace("probe_vst2_plugin: complete");
    entry
}

fn enumerate_vst2_shell(path: &Path, container: &VstPluginEntry) -> Vec<VstPluginEntry> {
    let host = Arc::new(Mutex::new(ScanHost { plugin_id: 0 }));
    let load_path = super::plugin_probe_paths::native_plugin_load_path(path);
    let Ok(mut loader) = PluginLoader::load(&load_path, host) else {
        return vec![container.clone()];
    };
    let Ok(instance) = loader.instance() else {
        return vec![container.clone()];
    };
    let mut entries = Vec::new();
    while let Some((plugin_id, name)) = instance.shell_next_plugin() {
        let mut entry = container.clone();
        entry.id.clear();
        entry.name = if name.is_empty() {
            format!("VST2 Shell {plugin_id}")
        } else {
            name
        };
        entry.kind = "instrument".to_string();
        entry.status = PluginStatus::Unverified;
        entry.supported = true;
        entry.unsupported_reason =
            Some("VST2 Shell class; final MIDI capability is verified at load".to_string());
        entry.failure_stage = Some("shellEnumeration".to_string());
        entry.class_uid = Some(format!("vst2shell:{plugin_id:08x}"));
        entry.sub_plugin_id = Some(i64::from(plugin_id));
        entry.has_editor = true;
        entry.last_probed_ms = None;
        entries.push(super::finalize_entry(entry));
    }
    if entries.is_empty() {
        vec![container.clone()]
    } else {
        entries
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantines_only_the_known_church_organ_probe_destructor() {
        assert!(requires_vst2_destructor_quarantine(0x3576_5a5b));
        assert!(!requires_vst2_destructor_quarantine(0x4b7a_3634));
    }
}
