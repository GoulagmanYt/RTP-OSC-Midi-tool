#![allow(deprecated)]

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicIsize, Ordering},
        mpsc,
    },
    time::Duration,
};

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use rack::{prelude::PluginScanner as _, vst3::Vst3Scanner, PluginInstance as _};
use vst::{
    api::Supported,
    host::{Host, PluginLoader},
    plugin::{CanDo, Plugin},
};

use crate::{
    logger::FrontendLogger,
    plugin_probe::{
        detect_vst3_channels, select_vst3_plugin_info_for_path, vst3_scan_root_for_path,
    },
    types::{VstParameter, VstPluginEntry},
    vst_scan::{is_vst2_path, is_vst3_path},
};

use super::runtime_state::{
    AudioError, AudioSettings, PluginBackend, Vst2TimeContext, MAX_PLUGIN_CHANNELS,
};

#[cfg(target_os = "windows")]
pub(super) const VST2_RESIZE_MESSAGE: u32 = WM_APP + 0x317;
#[cfg(target_os = "windows")]
static ACTIVE_VST2_EDITOR_HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(target_os = "windows")]
pub(super) fn set_active_vst2_editor_window(hwnd: Option<HWND>) {
    ACTIVE_VST2_EDITOR_HWND.store(
        hwnd.map(|window| window.0 as isize).unwrap_or_default(),
        Ordering::Release,
    );
}

pub(super) struct LoadedPluginBackend {
    pub(super) backend: PluginBackend,
    pub(super) input_channels: usize,
    pub(super) output_channels: usize,
    pub(super) vst_midi_compatible: bool,
    pub(super) latency_samples: u32,
    pub(super) parameter_cache: Vec<VstParameter>,
}

pub(super) fn resolve_vst_path(
    settings: &AudioSettings,
    fallback: Option<PathBuf>,
    logger: &FrontendLogger,
) -> Result<PathBuf, AudioError> {
    if let Some(path) = settings.vst_path.as_ref() {
        let source = PathBuf::from(path);
        if let Ok(real) = source.canonicalize() {
            if real.exists() {
                logger.info(format!("Using VST from {:?}", real));
                return Ok(real);
            }
        } else if source.exists() {
            logger.info(format!("Using VST from {:?}", source));
            return Ok(source);
        }
        logger.warn(format!("Configured VST path not found: {:?}", source));
    }

    let mut sources = Vec::new();
    if let Some(fallback) = fallback {
        sources.push(fallback);
    }
    sources.push(PathBuf::from("Bitsonic").join("Keyzone Classic.dll"));

    for source in sources {
        let Ok(real) = source.canonicalize() else {
            continue;
        };
        if !real.exists() {
            continue;
        }
        if is_vst2_path(&real) || is_vst3_path(&real) {
            logger.info(format!("Using VST fallback {:?}", real));
            return Ok(real);
        }
    }

    logger.warn("No valid VST path found");
    Err(AudioError::Message("No VST found".into()))
}

pub(super) fn load_plugin_backend(
    vst_path: &Path,
    sample_rate: u32,
    initial_block_size: u32,
    max_block_size: usize,
    logger: &FrontendLogger,
    probe: Option<&VstPluginEntry>,
) -> Result<LoadedPluginBackend, AudioError> {
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    if let Some(probe) = probe.filter(|entry| entry.architecture.eq_ignore_ascii_case("x86")) {
        let remote =
            super::remote_bridge::RemotePlugin::spawn(vst_path, sample_rate, max_block_size, probe)
                .map_err(AudioError::Message)?;
        let input_channels = remote.input_channels();
        let output_channels = remote.output_channels();
        let latency_samples = remote.latency_samples().saturating_add(initial_block_size);
        let parameter_cache = remote.parameters().to_vec();
        logger.info(format!(
            "VST x86 bridge '{}': {} inputs / {} outputs, bridge latency={} samples",
            probe.name, input_channels, output_channels, initial_block_size
        ));
        return Ok(LoadedPluginBackend {
            backend: PluginBackend::Remote {
                instance: Box::new(remote),
            },
            input_channels,
            output_channels,
            vst_midi_compatible: probe.midi_compatible.unwrap_or(true),
            latency_samples,
            parameter_cache,
        });
    }

    if is_vst3_path(vst_path) {
        let (mut plugin, inputs, outputs) =
            load_vst3_plugin(vst_path, sample_rate, max_block_size, logger, probe)?;
        let latency_samples = plugin.latency_samples();
        let parameter_cache = cache_vst3_parameters(&mut plugin)?;
        logger.debug(format!(
            "VST3 block size budget: {} samples (initial stream={})",
            max_block_size, initial_block_size
        ));
        return Ok(LoadedPluginBackend {
            backend: PluginBackend::Vst3 {
                instance: plugin,
                input_channels: inputs,
                output_channels: outputs,
            },
            input_channels: inputs,
            output_channels: outputs,
            vst_midi_compatible: true,
            latency_samples,
            parameter_cache,
        });
    }

    let plugin_id = probe
        .and_then(|entry| entry.class_uid.as_deref())
        .and_then(|uid| uid.strip_prefix("vst2shell:"))
        .and_then(|value| i32::from_str_radix(value, 16).ok())
        .unwrap_or(0);
    let time = std::sync::Arc::new(Vst2TimeContext::new());
    let host = std::sync::Arc::new(std::sync::Mutex::new(SimpleHost::new(
        plugin_id,
        sample_rate,
        max_block_size.min(u32::MAX as usize) as u32,
        std::sync::Arc::clone(&time),
    )));
    let load_path = crate::plugin_probe::native_plugin_load_path(vst_path);
    let mut loader = PluginLoader::load(&load_path, host).map_err(|error| {
        AudioError::Message(format!(
            "Failed to load VST plugin: {} ({:?})",
            error, vst_path
        ))
    })?;
    let mut instance = loader
        .instance()
        .map_err(|error| AudioError::Message(format!("Failed to instantiate VST: {}", error)))?;
    instance.init();
    instance.set_sample_rate(sample_rate as f32);
    // VST2's effSetBlockSize contract is a processing capacity, not a promise
    // that every processReplacing call has exactly this size. CPAL/WASAPI may
    // legitimately vary callback sizes, so advertise the preallocated maximum.
    instance.set_block_size(max_block_size.min(i64::MAX as usize) as i64);
    logger.debug(format!(
        "VST2 block size budget: {} samples (initial stream={})",
        max_block_size, initial_block_size
    ));

    let info = instance.get_info();
    let input_channels = info.inputs as usize;
    let output_channels = info.outputs as usize;
    validate_plugin_channel_count(input_channels, output_channels)?;
    let latency_samples = info.initial_delay.max(0) as u32;
    logger.debug(format!(
        "VST2 '{}': {} inputs / {} outputs",
        info.name, info.inputs, info.outputs
    ));
    let vst_midi_compatible = matches!(
        instance.can_do(CanDo::ReceiveMidiEvent),
        Supported::Yes | Supported::Maybe
    ) || matches!(
        instance.can_do(CanDo::ReceiveEvents),
        Supported::Yes | Supported::Maybe
    );
    if !vst_midi_compatible {
        logger.warn(format!(
            "VST2 '{}' does not advertise MIDI input support.",
            info.name
        ));
    }

    let parameters = instance.get_parameter_object();
    let parameter_cache = (0..info.parameters.max(0) as usize)
        .map(|index| {
            let index_i32 = index as i32;
            let value = parameters.get_parameter(index_i32);
            VstParameter {
                index,
                id: index as u32,
                flags: 0,
                step_count: 0,
                unit_id: 0,
                name: parameters.get_parameter_name(index_i32),
                min: 0.0,
                max: 1.0,
                default: value,
                unit: parameters.get_parameter_label(index_i32),
                value,
            }
        })
        .collect();

    instance.resume();
    instance.start_process();

    Ok(LoadedPluginBackend {
        backend: PluginBackend::Vst2 { instance, time },
        input_channels,
        output_channels,
        vst_midi_compatible,
        latency_samples,
        parameter_cache,
    })
}

fn cache_vst3_parameters(
    instance: &mut rack::vst3::Vst3Plugin,
) -> Result<Vec<VstParameter>, AudioError> {
    let count = instance.parameter_count();
    let mut parameters = Vec::with_capacity(count);
    for index in 0..count {
        let info = instance.parameter_info(index).map_err(|error| {
            AudioError::Message(format!("VST3 parameter metadata failed: {error}"))
        })?;
        let value = instance.get_parameter(index).unwrap_or(info.default);
        parameters.push(VstParameter {
            index,
            id: info.id,
            flags: info.flags,
            step_count: info.step_count,
            unit_id: info.unit_id,
            name: info.name,
            min: info.min,
            max: info.max,
            default: info.default,
            unit: info.unit,
            value,
        });
    }
    Ok(parameters)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_plugin_backend_thread_affine(
    vst_path: &Path,
    sample_rate: u32,
    initial_block_size: u32,
    max_block_size: usize,
    logger: &FrontendLogger,
    probe: Option<&VstPluginEntry>,
) -> Result<LoadedPluginBackend, AudioError> {
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    if probe.is_some_and(|entry| entry.architecture.eq_ignore_ascii_case("x86")) {
        // No third-party DLL is loaded in this process for bridged x86
        // plugins. Waiting for the child DSP process on Tauri's UI thread can
        // deadlock legacy plugins which synchronously message host windows
        // during startup. Keep the UI message pump alive and perform only the
        // bridge handshake on the serialized lifecycle thread.
        logger.debug("Starting the x86 DSP bridge without blocking the x64 UI thread");
        return load_plugin_backend(
            vst_path,
            sample_rate,
            initial_block_size,
            max_block_size,
            logger,
            probe,
        );
    }

    if super::thread_affinity::is_main_ui_thread() {
        logger.debug("Creating and initializing VST directly on the main UI thread");
        return load_plugin_backend(
            vst_path,
            sample_rate,
            initial_block_size,
            max_block_size,
            logger,
            probe,
        );
    }

    let path = vst_path.to_path_buf();
    let logger_for_main = logger.clone();
    let probe = probe.cloned();
    let (tx, rx) = mpsc::sync_channel(1);
    logger.debug("Scheduling VST creation and initialization on the main UI thread");
    logger
        .app_handle()
        .run_on_main_thread(move || {
            let result = load_plugin_backend(
                &path,
                sample_rate,
                initial_block_size,
                max_block_size,
                &logger_for_main,
                probe.as_ref(),
            );
            let _ = tx.send(result);
        })
        .map_err(|_| {
            AudioError::Message("Failed to schedule VST creation on main thread".into())
        })?;

    rx.recv_timeout(Duration::from_secs(30)).map_err(|_| {
        AudioError::Message("Timed out waiting for VST creation on main thread".into())
    })?
}

fn load_vst3_plugin(
    vst_path: &Path,
    sample_rate: u32,
    max_block_size: usize,
    logger: &FrontendLogger,
    probe: Option<&VstPluginEntry>,
) -> Result<(rack::vst3::Vst3Plugin, usize, usize), AudioError> {
    let scanner = Vst3Scanner::new().map_err(|error| {
        AudioError::Message(format!("Failed to initialise VST3 scanner: {}", error))
    })?;
    let info = if let Some(entry) =
        probe.filter(|entry| entry.host_abi_version == crate::types::VST_HOST_ABI_VERSION)
    {
        if let Some(class_uid) = entry.class_uid.as_ref() {
            logger.debug(format!(
                "Loading cached VST3 class UID {} without rescanning parent directory",
                class_uid
            ));
            rack::PluginInfo::new(
                entry.name.clone(),
                String::new(),
                0,
                rack::PluginType::Instrument,
                crate::plugin_probe::native_plugin_load_path(vst_path),
                class_uid.clone(),
            )
        } else {
            scan_vst3_info(&scanner, vst_path)?
        }
    } else {
        scan_vst3_info(&scanner, vst_path)?
    };

    let mut plugin = scanner
        .load(&info)
        .map_err(|error| AudioError::Message(format!("Failed to load VST3 plugin: {}", error)))?;
    plugin
        .initialize(sample_rate as f64, max_block_size)
        .map_err(|error| {
            AudioError::Message(format!("Failed to initialise VST3 plugin: {}", error))
        })?;

    let (inputs, outputs) =
        detect_vst3_channels(&mut plugin, &info, max_block_size).map_err(|error| {
            logger.warn(format!("VST3 channel probe failed: {error}"));
            AudioError::Message(format!("VST3 channel probe failed: {error}"))
        })?;
    validate_plugin_channel_count(inputs, outputs)?;
    logger.debug(format!(
        "VST3 '{}': {} inputs / {} outputs",
        info.name, inputs, outputs
    ));

    Ok((plugin, inputs, outputs))
}

fn validate_plugin_channel_count(inputs: usize, outputs: usize) -> Result<(), AudioError> {
    if inputs > MAX_PLUGIN_CHANNELS || outputs > MAX_PLUGIN_CHANNELS {
        return Err(AudioError::Message(format!(
            "Plugin channel count exceeds the real-time-safe limit ({inputs} inputs, {outputs} outputs, limit {MAX_PLUGIN_CHANNELS})"
        )));
    }
    Ok(())
}

fn scan_vst3_info(scanner: &Vst3Scanner, vst_path: &Path) -> Result<rack::PluginInfo, AudioError> {
    let scan_root = vst3_scan_root_for_path(vst_path);
    let plugins = scanner
        .scan_path(&scan_root)
        .map_err(|error| AudioError::Message(format!("Failed to scan VST3 plugins: {error}")))?;
    select_vst3_plugin_info_for_path(&plugins, vst_path).ok_or_else(|| {
        AudioError::Message(format!(
            "VST3 plugin info not found for {:?} (scan root: {:?})",
            vst_path, scan_root
        ))
    })
}

pub(super) struct SimpleHost {
    plugin_id: i32,
    sample_rate: u32,
    block_size: u32,
    started: std::time::Instant,
    time: std::sync::Arc<Vst2TimeContext>,
}

impl Default for SimpleHost {
    fn default() -> Self {
        Self::new(0, 48_000, 256, std::sync::Arc::new(Vst2TimeContext::new()))
    }
}

impl SimpleHost {
    fn new(
        plugin_id: i32,
        sample_rate: u32,
        block_size: u32,
        time: std::sync::Arc<Vst2TimeContext>,
    ) -> Self {
        Self {
            plugin_id,
            sample_rate,
            block_size,
            started: std::time::Instant::now(),
            time,
        }
    }
}

impl Host for SimpleHost {
    fn get_plugin_id(&self) -> i32 {
        self.plugin_id
    }

    fn get_block_size(&self) -> isize {
        self.block_size as isize
    }

    fn get_info(&self) -> (isize, String, String) {
        (2_400, "OSCMidi".to_string(), "OSCMidi VST Host".to_string())
    }

    fn get_time_info(&self, mask: i32) -> Option<vst::api::TimeInfo> {
        let sample_pos = self.time.sample_position() as f64;
        let tempo = 120.0;
        let ppq_pos = sample_pos / f64::from(self.sample_rate) * tempo / 60.0;
        Some(vst::api::TimeInfo {
            sample_pos,
            sample_rate: f64::from(self.sample_rate),
            nanoseconds: self.started.elapsed().as_nanos() as f64,
            ppq_pos,
            tempo,
            bar_start_pos: (ppq_pos / 4.0).floor() * 4.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            flags: ((vst::api::TimeInfoFlags::NANOSECONDS_VALID
                | vst::api::TimeInfoFlags::PPQ_POS_VALID
                | vst::api::TimeInfoFlags::TEMPO_VALID
                | vst::api::TimeInfoFlags::BARS_VALID
                | vst::api::TimeInfoFlags::TIME_SIG_VALID
                | vst::api::TimeInfoFlags::TRANSPORT_PLAYING)
                .bits()
                & (mask | vst::api::TimeInfoFlags::TRANSPORT_PLAYING.bits())),
            ..Default::default()
        })
    }

    fn get_sample_rate(&self) -> isize {
        self.sample_rate as isize
    }
    fn get_input_latency(&self) -> isize {
        0
    }
    fn get_output_latency(&self) -> isize {
        self.block_size as isize
    }
    fn get_current_process_level(&self) -> isize {
        vst::api::ProcessLevel::Realtime as isize
    }
    fn get_automation_state(&self) -> isize {
        1
    }
    fn can_do(&self, capability: &str) -> Supported {
        match capability {
            "sendVstEvents" | "sendVstMidiEvent" | "receiveVstTimeInfo" => Supported::Yes,
            _ => Supported::No,
        }
    }

    #[cfg(target_os = "windows")]
    fn size_window(&self, width: i32, height: i32) -> bool {
        if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
            return false;
        }
        let raw = ACTIVE_VST2_EDITOR_HWND.load(Ordering::Acquire);
        if raw == 0 {
            return false;
        }
        let hwnd = HWND(raw as *mut std::ffi::c_void);
        unsafe {
            PostMessageW(
                Some(hwnd),
                VST2_RESIZE_MESSAGE,
                WPARAM(width as usize),
                LPARAM(height as isize),
            )
        }
        .is_ok()
    }
}
