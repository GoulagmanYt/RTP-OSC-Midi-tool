#![allow(deprecated)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use directories::ProjectDirs;
use parking_lot::Mutex;
use rack::PluginInstance as _;
use vst::{host::PluginInstance, plugin::Plugin};

use crate::{
    logger::{background_log, FrontendLogger},
    types::VstPluginEntry,
};

use super::runtime_state::PluginBackend;

const STATE_MAGIC: &[u8; 4] = b"OSVS";
const STATE_VERSION_V1: u8 = 1;
const STATE_VERSION_V2: u8 = 2;
const STATE_VERSION_V3: u8 = 3;
const STATE_KIND_CHUNK: u8 = 0;
const STATE_KIND_PARAMS: u8 = 1;
const FORMAT_VST2: u8 = 2;
const FORMAT_VST3: u8 = 3;
const ARCH_X86: u8 = 1;
const ARCH_X64: u8 = 2;
const VST3_PARAMETER_IS_READ_ONLY: u32 = 1 << 1;
const MAX_STATE_BYTES: usize = 256 * 1024 * 1024;
const MAX_PARAMETERS: usize = 65_536;
const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
enum StatePolicy {
    #[default]
    NativePreferred = 0,
    TrackedParametersOnly = 1,
    StateUnsupported = 2,
    LegacyFullSnapshot = 3,
}

impl StatePolicy {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::NativePreferred),
            1 => Some(Self::TrackedParametersOnly),
            2 => Some(Self::StateUnsupported),
            3 => Some(Self::LegacyFullSnapshot),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(super) enum SavedState {
    Raw(Vec<u8>),
    Chunk(Vec<u8>),
    Params(Vec<f32>),
    V2(StateDocument),
}

#[derive(Clone, Debug, Default)]
pub(super) struct StateParameter {
    pub(super) id: u32,
    pub(super) index: u32,
    pub(super) flags: u32,
    pub(super) name: String,
    pub(super) value: f32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct StateDocument {
    state_version: u8,
    state_policy: StatePolicy,
    pub(super) plugin_id: String,
    pub(super) class_uid: String,
    pub(super) format: u8,
    pub(super) architecture: u8,
    pub(super) file_modified_ms: u64,
    pub(super) file_size: u64,
    pub(super) program: i32,
    pub(super) vst2_bank: Vec<u8>,
    pub(super) vst2_preset: Vec<u8>,
    pub(super) vst3_state: Vec<u8>,
    pub(super) parameters: Vec<StateParameter>,
}

fn state_dir() -> Option<PathBuf> {
    ProjectDirs::from("com", "OSCMIDI", "OSCMIDI").map(|dirs| dirs.config_dir().join("vst_state"))
}

fn legacy_state_path_for_plugin(vst_path: &Path) -> Option<PathBuf> {
    let file_name = vst_path.file_name()?.to_string_lossy();
    Some(state_dir()?.join(format!("{file_name}.state")))
}

fn old_stable_identity(vst_path: &Path, uid: Option<&str>) -> String {
    let canonical = vst_path
        .canonicalize()
        .unwrap_or_else(|_| vst_path.to_path_buf());
    let format = if crate::vst_scan::is_vst3_path(vst_path) {
        "vst3"
    } else {
        "vst2"
    };
    let base = format!("{}:{}", format, canonical.to_string_lossy().to_lowercase());
    uid.map(|uid| format!("{base}:{uid}")).unwrap_or(base)
}

fn old_state_path(vst_path: &Path, uid: Option<&str>) -> Option<PathBuf> {
    let file_name = vst_path.file_name()?.to_string_lossy();
    let hash = fnv1a64(old_stable_identity(vst_path, uid).as_bytes());
    Some(state_dir()?.join(format!("{file_name}-{hash:016x}.state")))
}

pub(super) fn state_path_for_plugin(entry: &VstPluginEntry) -> Option<PathBuf> {
    let file_name = Path::new(&entry.path).file_name()?.to_string_lossy();
    let hash = fnv1a64(entry.id.as_bytes());
    Some(state_dir()?.join(format!("{file_name}-{hash:016x}.state")))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
pub(super) fn encode_state_chunk(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + data.len());
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION_V1);
    out.push(STATE_KIND_CHUNK);
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
pub(super) fn encode_state_params(params: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + params.len() * 4);
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION_V1);
    out.push(STATE_KIND_PARAMS);
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for value in params {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_blob(out: &mut Vec<u8>, value: &[u8]) {
    push_u32(out, value.len().min(u32::MAX as usize) as u32);
    out.extend_from_slice(value);
}

fn push_text(out: &mut Vec<u8>, value: &str) {
    push_blob(out, value.as_bytes());
}

pub(super) fn encode_state_document(document: &StateDocument) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(document.format);
    body.push(document.architecture);
    push_u16(&mut body, document.state_policy as u16);
    push_u64(&mut body, document.file_modified_ms);
    push_u64(&mut body, document.file_size);
    body.extend_from_slice(&document.program.to_le_bytes());
    push_text(&mut body, &document.plugin_id);
    push_text(&mut body, &document.class_uid);
    push_blob(&mut body, &document.vst2_bank);
    push_blob(&mut body, &document.vst2_preset);
    push_blob(&mut body, &document.vst3_state);
    push_u32(
        &mut body,
        document.parameters.len().min(u32::MAX as usize) as u32,
    );
    for parameter in &document.parameters {
        push_u32(&mut body, parameter.id);
        push_u32(&mut body, parameter.index);
        push_u32(&mut body, parameter.flags);
        body.extend_from_slice(&parameter.value.to_le_bytes());
        let name = parameter.name.as_bytes();
        push_u16(&mut body, name.len().min(u16::MAX as usize) as u16);
        out_name(&mut body, name);
    }

    let mut out = Vec::with_capacity(9 + body.len() + 8);
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION_V3);
    push_u32(&mut out, body.len().min(u32::MAX as usize) as u32);
    out.extend_from_slice(&body);
    push_u64(&mut out, fnv1a64(&body));
    out
}

fn out_name(out: &mut Vec<u8>, name: &[u8]) {
    out.extend_from_slice(&name[..name.len().min(u16::MAX as usize)]);
}

struct StateReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> StateReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(count)?;
        let value = self.data.get(self.offset..end)?;
        self.offset = end;
        Some(value)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(*self.take(1)?.first()?)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn blob(&mut self, limit: usize) -> Option<Vec<u8>> {
        let len = self.u32()? as usize;
        (len <= limit).then_some(())?;
        Some(self.take(len)?.to_vec())
    }

    fn text(&mut self) -> Option<String> {
        String::from_utf8(self.blob(MAX_TEXT_BYTES)?).ok()
    }
}

fn decode_document(data: &[u8]) -> Option<StateDocument> {
    if data.len() < 17
        || data.get(..4)? != STATE_MAGIC
        || !matches!(data[4], STATE_VERSION_V2 | STATE_VERSION_V3)
    {
        return None;
    }
    let state_version = data[4];
    let body_len = u32::from_le_bytes(data.get(5..9)?.try_into().ok()?) as usize;
    if body_len > MAX_STATE_BYTES || data.len() != 9usize.checked_add(body_len)?.checked_add(8)? {
        return None;
    }
    let body = data.get(9..9 + body_len)?;
    let checksum = u64::from_le_bytes(data.get(9 + body_len..)?.try_into().ok()?);
    if fnv1a64(body) != checksum {
        return None;
    }
    let mut reader = StateReader::new(body);
    let format = reader.u8()?;
    let architecture = reader.u8()?;
    let encoded_policy = reader.u16()?;
    let state_policy = if state_version == STATE_VERSION_V2 {
        StatePolicy::LegacyFullSnapshot
    } else {
        StatePolicy::from_u16(encoded_policy)?
    };
    let file_modified_ms = reader.u64()?;
    let file_size = reader.u64()?;
    let program = reader.i32()?;
    let plugin_id = reader.text()?;
    let class_uid = reader.text()?;
    let vst2_bank = reader.blob(MAX_STATE_BYTES)?;
    let vst2_preset = reader.blob(MAX_STATE_BYTES)?;
    let vst3_state = reader.blob(MAX_STATE_BYTES)?;
    let parameter_count = reader.u32()? as usize;
    if parameter_count > MAX_PARAMETERS {
        return None;
    }
    let mut parameters = Vec::with_capacity(parameter_count);
    for _ in 0..parameter_count {
        let id = reader.u32()?;
        let index = reader.u32()?;
        let flags = reader.u32()?;
        let value = reader.f32()?;
        if !value.is_finite() {
            return None;
        }
        let name_len = reader.u16()? as usize;
        let name = String::from_utf8(reader.take(name_len)?.to_vec()).ok()?;
        parameters.push(StateParameter {
            id,
            index,
            flags,
            name,
            value,
        });
    }
    if reader.offset != body.len() {
        return None;
    }
    Some(StateDocument {
        state_version,
        state_policy,
        plugin_id,
        class_uid,
        format,
        architecture,
        file_modified_ms,
        file_size,
        program,
        vst2_bank,
        vst2_preset,
        vst3_state,
        parameters,
    })
}

pub(super) fn decode_state(data: &[u8]) -> SavedState {
    if let Some(document) = decode_document(data) {
        return SavedState::V2(document);
    }
    if data.len() < 6 || data[..4] != *STATE_MAGIC || data[4] != STATE_VERSION_V1 {
        return SavedState::Raw(data.to_vec());
    }
    match data[5] {
        STATE_KIND_CHUNK => SavedState::Chunk(data[6..].to_vec()),
        STATE_KIND_PARAMS => {
            if data.len() < 10 {
                return SavedState::Raw(data.to_vec());
            }
            let count = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
            let expected = 10usize.saturating_add(count.saturating_mul(4));
            if count > MAX_PARAMETERS || data.len() != expected {
                return SavedState::Raw(data.to_vec());
            }
            SavedState::Params(
                data[10..]
                    .chunks_exact(4)
                    .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
                    .collect(),
            )
        }
        _ => SavedState::Raw(data.to_vec()),
    }
}

fn capture_vst2_state_parameters(instance: &mut PluginInstance) -> Vec<StateParameter> {
    let count = instance.get_info().parameters.max(0) as usize;
    let params = instance.get_parameter_object();
    (0..count)
        .filter_map(|index| {
            let value = params.get_parameter(index as i32);
            value.is_finite().then(|| StateParameter {
                id: index as u32,
                index: index as u32,
                flags: 0,
                name: params.get_parameter_name(index as i32),
                value: value.clamp(0.0, 1.0),
            })
        })
        .collect()
}

pub(super) fn apply_vst2_parameters(instance: &mut PluginInstance, values: &[f32]) {
    let parameters = values
        .iter()
        .enumerate()
        .map(|(index, value)| StateParameter {
            id: index as u32,
            index: index as u32,
            flags: 0,
            name: String::new(),
            value: *value,
        })
        .collect::<Vec<_>>();
    apply_vst2_state_parameters(instance, &parameters);
}

fn apply_vst2_state_parameters(instance: &mut PluginInstance, values: &[StateParameter]) {
    let count = instance.get_info().parameters.max(0) as usize;
    let params = instance.get_parameter_object();
    for saved in values {
        let index = saved.index as usize;
        if index >= count || !saved.value.is_finite() {
            continue;
        }
        if !saved.name.is_empty() {
            let current_name = params.get_parameter_name(index as i32);
            if !current_name.is_empty() && !current_name.eq_ignore_ascii_case(&saved.name) {
                continue;
            }
        }
        params.set_parameter(index as i32, saved.value.clamp(0.0, 1.0));
    }
}

fn capture_document(plugin: &mut PluginBackend, entry: &VstPluginEntry) -> Option<StateDocument> {
    let mut document = StateDocument {
        state_version: STATE_VERSION_V3,
        state_policy: state_policy_for_entry(entry),
        plugin_id: entry.id.clone(),
        class_uid: entry.class_uid.clone().unwrap_or_default(),
        format: if entry.format.eq_ignore_ascii_case("VST3") {
            FORMAT_VST3
        } else {
            FORMAT_VST2
        },
        architecture: if entry.architecture.eq_ignore_ascii_case("x86") {
            ARCH_X86
        } else {
            ARCH_X64
        },
        file_modified_ms: entry.file_modified_ms.unwrap_or_default(),
        file_size: entry.file_size.unwrap_or_default(),
        program: -1,
        ..Default::default()
    };
    match plugin {
        PluginBackend::Vst2 { instance, .. } => {
            let info = instance.get_info();
            document.parameters = capture_vst2_state_parameters(instance);
            let params = instance.get_parameter_object();
            document.program = params.get_preset_num();
            if info.preset_chunks {
                document.vst2_bank = params.get_bank_data();
                document.vst2_preset = params.get_preset_data();
            }
        }
        PluginBackend::Vst3 { instance, .. } => match document.state_policy {
            StatePolicy::NativePreferred | StatePolicy::LegacyFullSnapshot => {
                document.parameters = capture_all_vst3_parameters(instance);
                match instance.get_state() {
                    Ok(state) => document.vst3_state = state,
                    Err(error) => background_log(
                        "warn",
                        format!("VST3 native state unavailable; parameters will be saved: {error}"),
                    ),
                }
            }
            StatePolicy::TrackedParametersOnly => {
                let metadata = (0..instance.parameter_count())
                    .filter_map(|index| {
                        instance
                            .parameter_info(index)
                            .ok()
                            .map(|info| (index, info))
                    })
                    .collect::<Vec<_>>();
                match instance.tracked_parameters() {
                    Ok(tracked) => {
                        for (id, value) in tracked {
                            let Some((index, info)) =
                                metadata.iter().find(|(_, info)| info.id == id)
                            else {
                                continue;
                            };
                            if info.flags & VST3_PARAMETER_IS_READ_ONLY == 0 && value.is_finite() {
                                document.parameters.push(StateParameter {
                                    id,
                                    index: *index as u32,
                                    flags: info.flags,
                                    name: info.name.clone(),
                                    value: value.clamp(0.0, 1.0),
                                });
                            }
                        }
                    }
                    Err(error) => background_log(
                        "warn",
                        format!("Failed to capture sparse VST3 parameter state: {error}"),
                    ),
                }
            }
            StatePolicy::StateUnsupported => {}
        },
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        PluginBackend::Remote { .. } => return None,
    }
    (!document.parameters.is_empty()
        || !document.vst2_bank.is_empty()
        || !document.vst2_preset.is_empty()
        || !document.vst3_state.is_empty())
    .then_some(document)
}

fn capture_all_vst3_parameters(instance: &mut rack::vst3::Vst3Plugin) -> Vec<StateParameter> {
    (0..instance.parameter_count())
        .filter_map(|index| {
            let info = instance.parameter_info(index).ok()?;
            if info.flags & VST3_PARAMETER_IS_READ_ONLY != 0 {
                return None;
            }
            let value = instance.get_parameter(index).ok()?;
            value.is_finite().then(|| StateParameter {
                id: info.id,
                index: index as u32,
                flags: info.flags,
                name: info.name,
                value: value.clamp(0.0, 1.0),
            })
        })
        .collect()
}

pub(super) fn save_vst_state_blocking(plugin: &Arc<Mutex<PluginBackend>>, entry: &VstPluginEntry) {
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    {
        let mut guard = plugin.lock();
        if let PluginBackend::Remote { instance } = &mut *guard {
            match instance.save_state() {
                Ok(()) => background_log(
                    "info",
                    format!("VST state saved inside x86 worker for {}", entry.id),
                ),
                Err(error) => background_log(
                    "error",
                    format!("Failed to save VST state in x86 worker: {error}"),
                ),
            }
            return;
        }
    }
    let Some(state_path) = state_path_for_plugin(entry) else {
        background_log("warn", "Could not determine VST state path");
        return;
    };
    let Some(document) = capture_document(&mut plugin.lock(), entry) else {
        background_log(
            "debug",
            "VST state not saved (no persistent state available)",
        );
        return;
    };
    let parameter_count = document.parameters.len();
    let native_bytes =
        document.vst2_bank.len() + document.vst2_preset.len() + document.vst3_state.len();
    let payload = encode_state_document(&document);
    if let Err(error) = crate::atomic_file::write_atomically(&state_path, &payload) {
        background_log("error", format!("Failed to save VST state: {error}"));
    } else {
        background_log(
            "info",
            format!(
                "VST state v{} saved atomically for {} to {:?} (policy={:?}, params={}, native={} bytes)",
                document.state_version,
                entry.id,
                state_path,
                document.state_policy,
                parameter_count,
                native_bytes
            ),
        );
    }
}

fn apply_vst3_parameters(instance: &mut rack::vst3::Vst3Plugin, saved: &[StateParameter]) {
    let current = (0..instance.parameter_count())
        .filter_map(|index| {
            instance
                .parameter_info(index)
                .ok()
                .map(|info| (index, info))
        })
        .collect::<Vec<_>>();
    for parameter in saved {
        let selected = current
            .iter()
            .find(|(_, info)| info.id == parameter.id)
            .or_else(|| {
                current.iter().find(|(index, info)| {
                    *index == parameter.index as usize
                        && (parameter.name.is_empty()
                            || info.name.eq_ignore_ascii_case(&parameter.name))
                })
            });
        let Some((index, info)) = selected else {
            continue;
        };
        if info.flags & VST3_PARAMETER_IS_READ_ONLY != 0 || !parameter.value.is_finite() {
            continue;
        }
        if let Err(error) = instance.set_parameter(*index, parameter.value.clamp(0.0, 1.0)) {
            background_log(
                "warn",
                format!("Failed to restore VST3 parameter {}: {error}", parameter.id),
            );
        }
    }
}

fn restore_document(
    plugin: &mut PluginBackend,
    document: &StateDocument,
    entry: &VstPluginEntry,
    logger: &FrontendLogger,
) {
    match plugin {
        PluginBackend::Vst2 { instance, .. } => {
            let info = instance.get_info();
            let mut native_restored = false;
            if info.preset_chunks {
                let params = instance.get_parameter_object();
                let bank_restored =
                    !document.vst2_bank.is_empty() && params.load_bank_data(&document.vst2_bank);
                if !document.vst2_bank.is_empty() && !bank_restored {
                    logger.warn("VST2 bank restore returned failure");
                }
                if document.program >= 0 && document.program < info.presets.max(0) {
                    params.change_preset(document.program);
                }
                if !document.vst2_preset.is_empty() {
                    native_restored = params.load_preset_data(&document.vst2_preset);
                    if !native_restored {
                        logger.warn("VST2 preset restore returned failure");
                    }
                } else {
                    native_restored = bank_restored;
                }
            } else if document.program >= 0 && document.program < info.presets.max(0) {
                instance
                    .get_parameter_object()
                    .change_preset(document.program);
            }
            if !native_restored {
                apply_vst2_state_parameters(instance, &document.parameters);
            }
            logger.info(format!(
                "VST2 state v2 restored for {} (native={}, fallback_params={})",
                entry.id,
                native_restored,
                if native_restored {
                    0
                } else {
                    document.parameters.len()
                }
            ));
        }
        PluginBackend::Vst3 { instance, .. } => {
            let native_restored = if !document.vst3_state.is_empty()
                && state_policy_for_entry(entry) == StatePolicy::NativePreferred
            {
                match instance.set_state(&document.vst3_state) {
                    Ok(()) => true,
                    Err(error) => {
                        logger.warn(format!(
                        "VST3 native state restore failed; applying parameter fallback: {error}"
                    ));
                        false
                    }
                }
            } else {
                false
            };
            if !native_restored {
                apply_vst3_parameters(instance, &document.parameters);
            }
            logger.info(format!(
                "VST3 state v{} restored for {} (policy={:?}, native={}, fallback_params={})",
                document.state_version,
                entry.id,
                document.state_policy,
                native_restored,
                if native_restored {
                    0
                } else {
                    document.parameters.len()
                }
            ));
        }
        #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
        PluginBackend::Remote { .. } => logger.debug("State is restored inside the x86 DSP worker"),
    }
}

fn candidate_state_paths(entry: &VstPluginEntry) -> Vec<(PathBuf, bool)> {
    let path = Path::new(&entry.path);
    let Some(primary) = state_path_for_plugin(entry) else {
        return Vec::new();
    };
    let mut historical = Vec::new();
    if let Some(old) = old_state_path(path, entry.class_uid.as_deref()) {
        historical.push(old);
    }
    if let Some(old) = old_state_path(path, None) {
        historical.push(old);
    }
    if let Some(old) = legacy_state_path_for_plugin(path) {
        historical.push(old);
    }
    if let (Some(dir), Some(file_name)) = (state_dir(), path.file_name()) {
        let prefix = format!("{}-", file_name.to_string_lossy());
        if let Ok(files) = fs::read_dir(dir) {
            historical.extend(files.flatten().map(|file| file.path()).filter(|candidate| {
                candidate
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
                    && candidate
                        .extension()
                        .is_some_and(|extension| extension == "state")
            }));
        }
    }
    historical.retain(|candidate| candidate != &primary && candidate.exists());
    historical.sort_by_key(|candidate| {
        fs::metadata(candidate)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| std::cmp::Reverse(duration.as_millis()))
    });
    historical.dedup();
    let mut candidates = vec![(primary, false)];
    for candidate in historical {
        if !candidates.iter().any(|(path, _)| path == &candidate) {
            candidates.push((candidate, true));
        }
    }
    candidates
}

pub(super) fn load_vst_state(
    plugin: &Arc<Mutex<PluginBackend>>,
    entry: &VstPluginEntry,
    logger: &FrontendLogger,
) {
    let Some(primary_path) = state_path_for_plugin(entry) else {
        logger.warn("Could not determine VST state path");
        return;
    };
    let selected = candidate_state_paths(entry)
        .into_iter()
        .find_map(|(path, migrate)| {
            let data = fs::read(&path).ok()?;
            match decode_state(&data) {
                SavedState::V2(document)
                    if !document.plugin_id.is_empty() && document.plugin_id != entry.id =>
                {
                    None
                }
                SavedState::V2(document) if unsafe_legacy_parameter_snapshot(&document, entry) => {
                    archive_unsafe_state(&path, logger);
                    None
                }
                SavedState::Params(values) if unsafe_legacy_parameter_values(&values, entry) => {
                    archive_unsafe_state(&path, logger);
                    None
                }
                state => Some((path, migrate, state)),
            }
        });
    let Some((state_path, migrate, state)) = selected else {
        logger.debug("No compatible saved VST state found");
        return;
    };
    let mut migrated_document = None;
    {
        let mut guard = plugin.lock();
        match state {
            SavedState::V2(document) => restore_document(&mut guard, &document, entry, logger),
            SavedState::Chunk(chunk) | SavedState::Raw(chunk) => match &mut *guard {
                PluginBackend::Vst2 { instance, .. } => {
                    let params = instance.get_parameter_object();
                    // The v1 writer always preferred a non-empty bank chunk, but
                    // failed to persist its kind. Treat it as a bank to avoid
                    // dispatching the same opaque bytes twice with conflicting modes.
                    let _ = params.load_bank_data(&chunk);
                    logger.info(format!("Legacy VST2 state restored from {:?}", state_path));
                }
                PluginBackend::Vst3 { instance, .. } => {
                    if !native_vst3_state_is_disabled(entry) {
                        if let Err(error) = instance.set_state(&chunk) {
                            logger.warn(format!("Failed to restore legacy VST3 state: {error}"));
                        }
                    }
                }
                #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
                PluginBackend::Remote { .. } => {}
            },
            SavedState::Params(values) => match &mut *guard {
                PluginBackend::Vst2 { instance, .. } => apply_vst2_parameters(instance, &values),
                PluginBackend::Vst3 { instance, .. } => {
                    let parameters = values
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| StateParameter {
                            id: index as u32,
                            index: index as u32,
                            flags: 0,
                            name: String::new(),
                            value,
                        })
                        .collect::<Vec<_>>();
                    apply_vst3_parameters(instance, &parameters);
                }
                #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
                PluginBackend::Remote { .. } => {}
            },
        }
        if migrate {
            migrated_document = capture_document(&mut guard, entry);
        }
    }
    if let Some(document) = migrated_document {
        let payload = encode_state_document(&document);
        match crate::atomic_file::write_atomically(&primary_path, &payload) {
            Ok(()) => logger.info(format!(
                "Migrated legacy VST state to v3 for {} at {:?}",
                entry.id, primary_path
            )),
            Err(error) => logger.warn(format!("Failed to migrate legacy VST state: {error}")),
        }
    }
}

fn native_vst3_state_is_disabled(entry: &VstPluginEntry) -> bool {
    crate::plugin_probe::vst3_native_state_requires_parameter_fallback(
        Path::new(&entry.path),
        entry.class_uid.as_deref(),
        entry.file_size,
    )
}

fn state_policy_for_entry(entry: &VstPluginEntry) -> StatePolicy {
    if !entry.format.eq_ignore_ascii_case("VST3") {
        return StatePolicy::NativePreferred;
    }
    if native_vst3_state_is_disabled(entry) {
        StatePolicy::TrackedParametersOnly
    } else {
        StatePolicy::NativePreferred
    }
}

fn unsafe_legacy_parameter_snapshot(document: &StateDocument, entry: &VstPluginEntry) -> bool {
    if state_policy_for_entry(entry) != StatePolicy::TrackedParametersOnly
        || document.state_policy != StatePolicy::LegacyFullSnapshot
        || !document.vst3_state.is_empty()
        || document.parameters.len() < 512
    {
        return false;
    }
    unsafe_dense_zero_values(
        document.parameters.len(),
        document
            .parameters
            .iter()
            .filter(|parameter| parameter.value.abs() <= f32::EPSILON)
            .count(),
    )
}

fn unsafe_legacy_parameter_values(values: &[f32], entry: &VstPluginEntry) -> bool {
    state_policy_for_entry(entry) == StatePolicy::TrackedParametersOnly
        && unsafe_dense_zero_values(
            values.len(),
            values
                .iter()
                .filter(|value| value.abs() <= f32::EPSILON)
                .count(),
        )
}

fn unsafe_dense_zero_values(total: usize, zero_like: usize) -> bool {
    total >= 512 && zero_like.saturating_mul(10) >= total.saturating_mul(9)
}

fn archive_unsafe_state(path: &Path, logger: &FrontendLogger) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let archived = path.with_extension(format!("state.invalid-{timestamp}"));
    match fs::rename(path, &archived) {
        Ok(()) => logger.warn(format!(
            "Ignored an unsafe exhaustive VST3 parameter snapshot and preserved it at {:?}",
            archived
        )),
        Err(error) => logger.warn(format!(
            "Ignored an unsafe exhaustive VST3 parameter snapshot at {:?}; archival failed: {error}",
            path
        )),
    }
}

pub(crate) fn restore_vst3_state_file_for_diagnostic(
    instance: &mut rack::vst3::Vst3Plugin,
    entry: &VstPluginEntry,
    path: &Path,
) -> Result<usize, String> {
    let data = fs::read(path).map_err(|error| format!("failed to read state file: {error}"))?;
    match decode_state(&data) {
        SavedState::V2(document) => {
            if !document.plugin_id.is_empty() && document.plugin_id != entry.id {
                return Err("state belongs to a different plug-in identity".into());
            }
            if unsafe_legacy_parameter_snapshot(&document, entry) {
                return Err("unsafe exhaustive legacy parameter snapshot rejected".into());
            }
            if !document.vst3_state.is_empty()
                && state_policy_for_entry(entry) == StatePolicy::NativePreferred
            {
                instance
                    .set_state(&document.vst3_state)
                    .map_err(|error| format!("native state restore failed: {error}"))?;
                Ok(0)
            } else {
                apply_vst3_parameters(instance, &document.parameters);
                Ok(document.parameters.len())
            }
        }
        SavedState::Params(values) => {
            if unsafe_legacy_parameter_values(&values, entry) {
                return Err("unsafe exhaustive legacy parameter snapshot rejected".into());
            }
            let parameters = values
                .into_iter()
                .enumerate()
                .map(|(index, value)| StateParameter {
                    id: index as u32,
                    index: index as u32,
                    value,
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            apply_vst3_parameters(instance, &parameters);
            Ok(parameters.len())
        }
        SavedState::Chunk(state) | SavedState::Raw(state) => {
            if state_policy_for_entry(entry) != StatePolicy::NativePreferred {
                return Err("legacy native state is disabled by the compatibility profile".into());
            }
            instance
                .set_state(&state)
                .map_err(|error| format!("legacy native state restore failed: {error}"))?;
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_v3_round_trip_and_checksum_validation() {
        let document = StateDocument {
            plugin_id: "vst-test".into(),
            class_uid: "class-test".into(),
            format: FORMAT_VST3,
            architecture: ARCH_X64,
            file_modified_ms: 123,
            file_size: 456,
            program: 2,
            vst3_state: vec![1, 2, 3],
            parameters: vec![StateParameter {
                id: 42,
                index: 3,
                flags: 0,
                name: "Gain".into(),
                value: 0.75,
            }],
            ..Default::default()
        };
        let encoded = encode_state_document(&document);
        assert_eq!(encoded[4], STATE_VERSION_V3);
        let SavedState::V2(decoded) = decode_state(&encoded) else {
            panic!("expected versioned state document")
        };
        assert_eq!(decoded.state_version, STATE_VERSION_V3);
        assert_eq!(decoded.plugin_id, document.plugin_id);
        assert_eq!(decoded.vst3_state, document.vst3_state);
        assert_eq!(decoded.parameters[0].id, 42);
        let mut corrupted = encoded;
        corrupted[12] ^= 0x80;
        assert!(matches!(decode_state(&corrupted), SavedState::Raw(_)));
    }

    #[test]
    fn upright_dense_zero_legacy_snapshot_is_rejected_but_sparse_state_is_kept() {
        let entry = VstPluginEntry {
            id: "upright".into(),
            name: "Upright Piano".into(),
            path: r"C:\VST3\Upright Piano.vst3".into(),
            format: "VST3".into(),
            kind: "instrument".into(),
            architecture: "x64".into(),
            available_architectures: vec!["x64".into()],
            vendor: None,
            plugin_version: None,
            status: crate::types::PluginStatus::Compatible,
            supported: true,
            unsupported_reason: None,
            failure_stage: None,
            last_error: None,
            last_probed_ms: None,
            midi_compatible: Some(true),
            has_editor: true,
            channel_layout: Some("0 in / 2 out".into()),
            class_uid: Some("5653545F474B64757072696768742070".into()),
            sub_plugin_id: None,
            hosting_mode: Some("directX64".into()),
            file_modified_ms: None,
            file_size: Some(4_173_312),
            host_abi_version: crate::types::VST_HOST_ABI_VERSION,
        };
        let dense = StateDocument {
            state_version: STATE_VERSION_V2,
            state_policy: StatePolicy::LegacyFullSnapshot,
            parameters: (0..2_086)
                .map(|index| StateParameter {
                    id: index,
                    index,
                    value: 0.0,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert!(unsafe_legacy_parameter_snapshot(&dense, &entry));

        let sparse = StateDocument {
            state_version: STATE_VERSION_V3,
            state_policy: StatePolicy::TrackedParametersOnly,
            parameters: vec![StateParameter {
                id: 42,
                index: 4,
                value: 0.75,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!unsafe_legacy_parameter_snapshot(&sparse, &entry));
    }
}
