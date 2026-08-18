#![allow(deprecated)]

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum AudioLifecycleState {
    Stopped = 0,
    Loading = 1,
    Running = 2,
    Stopping = 3,
    Faulted = 4,
}

impl AudioLifecycleState {
    pub(super) fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Loading,
            2 => Self::Running,
            3 => Self::Stopping,
            4 => Self::Faulted,
            _ => Self::Stopped,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Loading => "loading",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Faulted => "faulted",
        }
    }
}

use cpal::Stream;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use rack::prelude::MidiEvent as RackMidiEvent;
use rtrb::Consumer;
use serde::{Deserialize, Serialize};
use vst::api;
use vst::{editor::Editor, host::PluginInstance};
use windows::Win32::Foundation::HWND;

use crate::types::VstParameter;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    pub enabled: bool,
    pub backend: Option<String>,
    pub device: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub gain_db: f32,
    #[serde(default)]
    pub limiter_enabled: bool,
    #[serde(default)]
    pub vst_plugin_id: Option<String>,
    pub vst_path: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Some("auto".to_string()),
            device: None,
            device_id: None,
            sample_rate: 48_000,
            buffer_size: 256,
            gain_db: 0.0,
            limiter_enabled: false,
            vst_plugin_id: None,
            vst_path: None,
        }
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum AudioError {
    #[error("{0}")]
    Message(String),
}

pub(super) enum PluginBackend {
    Vst2 {
        instance: PluginInstance,
        time: Arc<Vst2TimeContext>,
    },
    Vst3 {
        instance: rack::vst3::Vst3Plugin,
        input_channels: usize,
        output_channels: usize,
    },
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    Remote {
        instance: super::remote_bridge::RemotePlugin,
    },
}

pub(super) struct Vst2TimeContext {
    sample_position: AtomicU64,
}

impl Vst2TimeContext {
    pub(super) fn new() -> Self {
        Self {
            sample_position: AtomicU64::new(0),
        }
    }

    pub(super) fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Acquire)
    }

    pub(super) fn advance(&self, frames: usize) {
        self.sample_position
            .fetch_add(frames as u64, Ordering::Release);
    }
}

#[repr(align(64))]
pub(super) struct AudioControls {
    pub(super) gain_bits: AtomicU32,
    pub(super) limiter_enabled: AtomicBool,
}

#[repr(align(64))]
pub(super) struct AudioTelemetry {
    pub(super) xruns: AtomicU32,
    pub(super) stream_fatal_error: AtomicU32,
    pub(super) stream_recovery_requests: AtomicU32,
    pub(super) stream_route_changes: AtomicU32,
    pub(super) backend_latency_us: AtomicU64,
    pub(super) meter_left: AtomicU32,
    pub(super) meter_right: AtomicU32,
    pub(super) block_size_frames: AtomicU32,
    pub(super) midi_drop_count: AtomicU32,
    pub(super) audio_lock_miss_count: AtomicU32,
    pub(super) emergency_reset_count: AtomicU32,
    pub(super) callback_last_us: AtomicU32,
    pub(super) callback_max_us: AtomicU32,
    pub(super) callback_over_budget_count: AtomicU32,
    pub(super) consecutive_deadline_misses: AtomicU32,
    pub(super) dsp_process_last_us: AtomicU32,
    pub(super) dsp_process_max_us: AtomicU32,
    pub(super) dsp_histogram: [AtomicU32; DSP_HISTOGRAM_BUCKETS],
    pub(super) audio_midi_queue_depth: AtomicU32,
    pub(super) audio_midi_queue_max_depth: AtomicU32,
    pub(super) audio_midi_oldest_us: AtomicU64,
}

pub(super) const DSP_HISTOGRAM_BUCKETS: usize = 256;
pub(super) const DSP_HISTOGRAM_BUCKET_US: u32 = 250;

impl AudioTelemetry {
    pub(super) fn new() -> Self {
        Self {
            xruns: AtomicU32::new(0),
            stream_fatal_error: AtomicU32::new(0),
            stream_recovery_requests: AtomicU32::new(0),
            stream_route_changes: AtomicU32::new(0),
            backend_latency_us: AtomicU64::new(0),
            meter_left: AtomicU32::new(0.0f32.to_bits()),
            meter_right: AtomicU32::new(0.0f32.to_bits()),
            block_size_frames: AtomicU32::new(0),
            midi_drop_count: AtomicU32::new(0),
            audio_lock_miss_count: AtomicU32::new(0),
            emergency_reset_count: AtomicU32::new(0),
            callback_last_us: AtomicU32::new(0),
            callback_max_us: AtomicU32::new(0),
            callback_over_budget_count: AtomicU32::new(0),
            consecutive_deadline_misses: AtomicU32::new(0),
            dsp_process_last_us: AtomicU32::new(0),
            dsp_process_max_us: AtomicU32::new(0),
            dsp_histogram: std::array::from_fn(|_| AtomicU32::new(0)),
            audio_midi_queue_depth: AtomicU32::new(0),
            audio_midi_queue_max_depth: AtomicU32::new(0),
            audio_midi_oldest_us: AtomicU64::new(0),
        }
    }

    pub(super) fn dsp_percentile_us(&self, percentile: u32) -> u32 {
        let total = self
            .dsp_histogram
            .iter()
            .map(|bucket| u64::from(bucket.load(Ordering::Relaxed)))
            .sum::<u64>();
        if total == 0 {
            return 0;
        }
        let target = (total * u64::from(percentile).clamp(1, 100)).div_ceil(100);
        let mut seen = 0u64;
        for (index, bucket) in self.dsp_histogram.iter().enumerate() {
            seen += u64::from(bucket.load(Ordering::Relaxed));
            if seen >= target {
                return ((index as u32) + 1) * DSP_HISTOGRAM_BUCKET_US;
            }
        }
        (DSP_HISTOGRAM_BUCKETS as u32) * DSP_HISTOGRAM_BUCKET_US
    }
}

pub(super) struct AudioRuntime {
    pub(super) _stream: Stream,
    pub(super) _process_tuning: super::windows_tuning::AudioProcessTuning,
    // Keep the CPAL device (and therefore the ASIO driver) alive for the whole
    // runtime. Hot VST reloads can transfer this handle to the next runtime
    // instead of unloading/re-enumerating the exclusive ASIO driver.
    pub(super) _device: cpal::Device,
    pub(super) plugin: Arc<Mutex<PluginBackend>>,
    // This is immutable for the lifetime of the runtime. Status/heartbeat
    // readers can therefore identify a remote x86 plug-in without contending
    // with the real-time callback for the plug-in mutex.
    pub(super) is_x86_bridge: bool,
    pub(super) editor_window: Arc<Mutex<Option<EditorWindow>>>,
    pub(super) controls: Arc<AudioControls>,
    pub(super) telemetry: Arc<AudioTelemetry>,
    pub(super) sample_rate: u32,
    pub(super) requested_buffer_size: u32,
    pub(super) stream_buffer_size: Option<u32>,
    pub(super) plugin_latency_samples: u32,
    pub(super) parameter_tx: Sender<ParameterCommand>,
    pub(super) parameter_cache: Arc<Mutex<Vec<VstParameter>>>,
    pub(super) vst_midi_compatible: bool,
    pub(super) backend: String,
    pub(super) device: String,
    pub(super) device_id: Option<String>,
    pub(super) vst_path: PathBuf,
    pub(super) vst_class_uid: Option<String>,
    pub(super) vst_plugin: crate::types::VstPluginEntry,
}

pub(super) enum EditorWindow {
    Vst2 {
        editor: Box<dyn Editor>,
        hwnd: HWND,
    },
    Vst3 {
        gui: rack::vst3::Vst3PluginGui,
        hwnd: HWND,
    },
}

// SAFETY: this type is moved only into closures scheduled by `run_on_main_thread`.
// Every editor/GUI method and normal drop happens on that thread. Exceptional
// scheduling failures deliberately leak the thread-affine value rather than
// dropping it on the caller thread.
unsafe impl Send for EditorWindow {}

// SAFETY: the CPAL stream is created, controlled, and dropped as part of the
// synchronized runtime lifecycle. Plugin/editor mutation remains guarded, and
// editor operations are dispatched to the main thread as documented above.
unsafe impl Send for AudioRuntime {}

pub(super) const MIDI_RING_CAPACITY: usize = 16384;
pub(super) const MAX_PENDING_MIDI: usize = 8192;
pub(super) const MIDI_DRAIN_BUDGET_PER_CALLBACK: usize = 512;
pub(super) const MIDI_EVENT_BATCH_CAPACITY: usize = 512;
pub(super) const RESET_CONTROLLERS: [u8; 4] = [64, 120, 121, 123];
pub(super) const VST_EDITOR_IDLE_TIMER_MS: u32 = 50;
pub(super) const PARAMETER_COMMAND_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug)]
#[cfg_attr(target_pointer_width = "32", allow(dead_code))]
pub(super) struct ParameterCommand {
    pub(super) index: usize,
    pub(super) id: u32,
    pub(super) flags: u32,
    pub(super) step_count: i32,
    pub(super) value: f32,
}

#[repr(C)]
pub(super) struct Vst2EventBatch {
    pub(super) num_events: i32,
    pub(super) reserved: isize,
    pub(super) events: [*mut api::Event; MIDI_EVENT_BATCH_CAPACITY],
}

pub(super) fn vst_shutdown_timeout() -> Duration {
    let secs = std::env::var("VST_SHUTDOWN_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2);
    Duration::from_secs(secs.clamp(1, 30))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MidiPacket {
    pub(super) data: [u8; 3],
    pub(super) len: u8,
    pub(super) timestamp_us: u64,
}

impl MidiPacket {
    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_bytes_with_age(bytes, 0)
    }

    pub(super) fn from_bytes_with_age(bytes: &[u8], age_us: u64) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        let mut data = [0u8; 3];
        let len = bytes.len().min(3);
        for (idx, byte) in bytes.iter().take(len).enumerate() {
            data[idx] = *byte;
        }
        Some(Self {
            data,
            len: len as u8,
            timestamp_us: monotonic_us().saturating_sub(age_us),
        })
    }

    pub(super) fn status(self) -> u8 {
        self.data[0]
    }

    pub(super) fn is_note_on(self) -> bool {
        self.len >= 3 && (self.status() & 0xF0) == 0x90 && self.data[2] > 0
    }

    pub(super) fn is_note_off(self) -> bool {
        (self.status() & 0xF0) == 0x80
            || (self.len >= 3 && (self.status() & 0xF0) == 0x90 && self.data[2] == 0)
    }

    pub(super) fn is_cc64_off(self) -> bool {
        self.len >= 3 && (self.status() & 0xF0) == 0xB0 && self.data[1] == 64 && self.data[2] < 64
    }

    pub(super) fn is_critical_release(self) -> bool {
        self.is_note_off() || self.is_cc64_off()
    }

    pub(super) fn age_us(self, now_us: u64) -> u64 {
        now_us.saturating_sub(self.timestamp_us)
    }
}

pub(super) struct AudioCallbackState {
    pub(super) midi_rx: Consumer<MidiPacket>,
    pub(super) pending_midi: VecDeque<MidiPacket>,
    pub(super) midi_events: Vec<RackMidiEvent>,
    pub(super) vst2_midi_events: Vec<api::MidiEvent>,
    pub(super) vst2_event_batch: Vst2EventBatch,
    pub(super) parameter_rx: Receiver<ParameterCommand>,
    pub(super) parameter_commands: Vec<ParameterCommand>,
    pub(super) input_silence: Vec<f32>,
    pub(super) input_ptrs: Vec<*const f32>,
    pub(super) outputs: Vec<Vec<f32>>,
    pub(super) output_ptrs: Vec<*mut f32>,
    pub(super) plugin_inputs: usize,
    pub(super) plugin_outputs: usize,
    pub(super) max_frames: usize,
    pub(super) controls: Arc<AudioControls>,
    pub(super) telemetry: Arc<AudioTelemetry>,
    pub(super) emergency_reset_requested: Arc<AtomicBool>,
    pub(super) sample_rate: u32,
    pub(super) last_output: Vec<f32>,
    pub(super) needs_emergency_reset: bool,
    pub(super) last_frames: usize,
    pub(super) error_count: u32,
    pub(super) recovering_from_silence: bool,
    #[cfg(target_os = "windows")]
    pub(super) mmcss_applied: bool,
    #[cfg(target_os = "windows")]
    pub(super) mmcss_registration: Option<super::windows_tuning::MmcssRegistration>,
}

// SAFETY: AudioCallbackState is moved into the callback before any callback runs and
// remains confined there. Its raw pointers refer to fixed-capacity heap allocations;
// `prepare` never resizes those allocations and refreshes the pointers before use.
unsafe impl Send for AudioCallbackState {}

impl AudioCallbackState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        midi_rx: Consumer<MidiPacket>,
        plugin_inputs: usize,
        plugin_outputs: usize,
        max_frames: usize,
        device_channels: usize,
        controls: Arc<AudioControls>,
        telemetry: Arc<AudioTelemetry>,
        emergency_reset_requested: Arc<AtomicBool>,
        sample_rate: u32,
        parameter_rx: Receiver<ParameterCommand>,
    ) -> Self {
        let max_frames = max_frames.max(1);
        let input_silence = vec![0.0; max_frames];
        let input_ptrs = vec![input_silence.as_ptr(); plugin_inputs];
        let mut outputs: Vec<Vec<f32>> =
            (0..plugin_outputs).map(|_| vec![0.0; max_frames]).collect();
        let output_ptrs = outputs.iter_mut().map(Vec::as_mut_ptr).collect();

        Self {
            midi_rx,
            pending_midi: VecDeque::with_capacity(MAX_PENDING_MIDI),
            midi_events: Vec::with_capacity(MIDI_EVENT_BATCH_CAPACITY),
            vst2_midi_events: Vec::with_capacity(MIDI_EVENT_BATCH_CAPACITY),
            vst2_event_batch: Vst2EventBatch {
                num_events: 0,
                reserved: 0,
                events: [std::ptr::null_mut(); MIDI_EVENT_BATCH_CAPACITY],
            },
            parameter_rx,
            parameter_commands: Vec::with_capacity(MIDI_EVENT_BATCH_CAPACITY),
            input_silence,
            input_ptrs,
            outputs,
            output_ptrs,
            plugin_inputs,
            plugin_outputs,
            max_frames,
            controls,
            telemetry,
            emergency_reset_requested,
            sample_rate,
            last_output: vec![0.0; max_frames.saturating_mul(device_channels)],
            needs_emergency_reset: false,
            last_frames: 0,
            error_count: 0,
            recovering_from_silence: false,
            #[cfg(target_os = "windows")]
            mmcss_applied: false,
            #[cfg(target_os = "windows")]
            mmcss_registration: None,
        }
    }

    pub(super) fn prepare(&mut self, frames: usize) -> bool {
        if frames > self.max_frames {
            return false;
        }

        let input_ptr = self.input_silence.as_ptr();
        debug_assert_eq!(self.input_ptrs.len(), self.plugin_inputs);
        for ptr in &mut self.input_ptrs {
            *ptr = input_ptr;
        }

        debug_assert_eq!(self.outputs.len(), self.plugin_outputs);

        for output in &mut self.outputs {
            output[..frames].fill(0.0);
        }

        for (idx, output) in self.outputs.iter_mut().enumerate() {
            self.output_ptrs[idx] = output.as_mut_ptr();
        }
        true
    }

    pub(super) fn drain_midi(&mut self) {
        if self
            .emergency_reset_requested
            .swap(false, Ordering::Relaxed)
        {
            self.needs_emergency_reset = true;
            self.pending_midi.clear();
        }
        let now_us = monotonic_us();
        let measured_frames = self.telemetry.block_size_frames.load(Ordering::Relaxed);
        let current_frames = if measured_frames == 0 {
            self.max_frames.min(u32::MAX as usize) as u32
        } else {
            measured_frames
        };
        let stale_after_us = if self.sample_rate == 0 {
            u64::MAX
        } else {
            u64::from(current_frames)
                .saturating_mul(2_000_000)
                .checked_div(u64::from(self.sample_rate))
                .unwrap_or(u64::MAX)
        };
        for _ in 0..MIDI_DRAIN_BUDGET_PER_CALLBACK {
            let Ok(msg) = self.midi_rx.pop() else {
                break;
            };
            if !msg.is_critical_release() && msg.age_us(now_us) > stale_after_us {
                self.telemetry
                    .midi_drop_count
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            self.enqueue_midi(msg);
        }
        let depth = self.pending_midi.len().saturating_add(self.midi_rx.slots());
        let depth = depth.min(u32::MAX as usize) as u32;
        self.telemetry
            .audio_midi_queue_depth
            .store(depth, Ordering::Relaxed);
        atomic_max(&self.telemetry.audio_midi_queue_max_depth, depth);
        let oldest_us = self
            .pending_midi
            .front()
            .map(|packet| packet.age_us(now_us))
            .unwrap_or(0);
        self.telemetry
            .audio_midi_oldest_us
            .store(oldest_us, Ordering::Relaxed);
    }

    pub(super) fn enqueue_midi(&mut self, msg: MidiPacket) {
        if self.pending_midi.len() < MAX_PENDING_MIDI {
            self.pending_midi.push_back(msg);
            return;
        }

        if self.drop_oldest_note_on() {
            self.pending_midi.push_back(msg);
            self.telemetry
                .midi_drop_count
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        if !msg.is_critical_release() {
            self.telemetry
                .midi_drop_count
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        if self.drop_oldest_non_critical() {
            self.pending_midi.push_back(msg);
            self.telemetry
                .midi_drop_count
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.pending_midi.clear();
        self.pending_midi.push_back(msg);
        self.needs_emergency_reset = true;
        self.telemetry
            .midi_drop_count
            .fetch_add(1, Ordering::Relaxed);
    }

    fn drop_oldest_note_on(&mut self) -> bool {
        let pos = self.pending_midi.iter().position(|m| m.is_note_on());
        if let Some(idx) = pos {
            self.pending_midi.swap(0, idx);
            self.pending_midi.pop_front();
            return true;
        }
        false
    }

    fn drop_oldest_non_critical(&mut self) -> bool {
        let pos = self
            .pending_midi
            .iter()
            .position(|m| !m.is_critical_release());
        if let Some(idx) = pos {
            self.pending_midi.swap(0, idx);
            self.pending_midi.pop_front();
            return true;
        }
        false
    }

    pub(super) fn record_error(&mut self) {
        self.error_count = self.error_count.wrapping_add(1);
    }

    pub(super) fn drain_parameter_commands(&mut self) {
        self.parameter_commands.clear();
        while self.parameter_commands.len() < self.parameter_commands.capacity() {
            let Ok(command) = self.parameter_rx.try_recv() else {
                break;
            };
            if let Some(existing) = self
                .parameter_commands
                .iter_mut()
                .find(|existing| existing.index == command.index)
            {
                *existing = command;
            } else {
                self.parameter_commands.push(command);
            }
        }
    }
}

pub(super) fn timestamp_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn monotonic_us() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH
        .get_or_init(Instant::now)
        .elapsed()
        .as_micros()
        .min(u64::MAX as u128) as u64
}

pub(super) fn atomic_max(cell: &AtomicU32, value: u32) {
    let mut current = cell.load(Ordering::Relaxed);
    while value > current {
        match cell.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}
