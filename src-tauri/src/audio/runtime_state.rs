#![allow(deprecated)]

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
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

use crate::config::VstEqSettings;
use crate::types::VstParameter;

pub(super) const MAX_PLUGIN_CHANNELS: usize = 256;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DspThreadPriority {
    #[default]
    Mmcss,
    TimeCritical,
}

pub(crate) fn default_render_ahead_blocks() -> u32 {
    1
}

pub(crate) fn deserialize_render_ahead_blocks<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u32, D::Error> {
    let blocks = u32::deserialize(deserializer)?;
    if blocks > 2 {
        return Err(serde::de::Error::custom(
            "renderAheadBlocks must be 0, 1 or 2",
        ));
    }
    Ok(blocks)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    #[serde(default = "default_game_mode")]
    pub game_mode: bool,
    pub enabled: bool,
    pub backend: Option<String>,
    pub device: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    #[serde(
        default = "default_render_ahead_blocks",
        deserialize_with = "deserialize_render_ahead_blocks"
    )]
    pub render_ahead_blocks: u32,
    #[serde(default)]
    pub dsp_thread_priority: DspThreadPriority,
    pub gain_db: f32,
    #[serde(default)]
    pub limiter_enabled: bool,
    #[serde(default)]
    pub vst_eq: VstEqSettings,
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
            buffer_size: 512,
            render_ahead_blocks: default_render_ahead_blocks(),
            dsp_thread_priority: DspThreadPriority::default(),
            game_mode: true,
            gain_db: 0.0,
            limiter_enabled: false,
            vst_eq: VstEqSettings::default(),
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
        instance: Box<super::remote_bridge::RemotePlugin>,
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
    pub(super) equalizer: super::equalizer::PublishedEq,
}

#[repr(align(64))]
pub(super) struct AudioTelemetry {
    pub(super) render_underflows: AtomicU64,
    pub(super) render_missing_frames: AtomicU64,
    pub(super) render_reset_silence_frames: AtomicU64,
    pub(super) render_queue_frames: AtomicU64,
    pub(super) render_ahead_samples: AtomicU32,
    pub(super) render_running: AtomicBool,
    pub(super) render_busy_since_qpc: AtomicU64,
    pub(super) render_last_completed_qpc: AtomicU64,
    pub(super) render_block_max_us: AtomicU32,
    pub(super) render_over_budget: AtomicU64,
    pub(super) callback_gap_count: AtomicU64,
    pub(super) callback_gate: super::callback_gate::CallbackGate,
    pub(super) mmcss_enabled: AtomicU8,
    pub(super) mmcss_owned: AtomicU8,
    pub(super) mmcss_critical: AtomicU8,
    pub(super) editor_open: AtomicBool,
    pub(super) parameter_accepted_sequence: AtomicU64,
    pub(super) parameter_failed_sequence: AtomicU64,
    pub(super) parameter_applied_sequence: AtomicU64,
    pub(super) midi_generation: AtomicU64,
    pub(super) pipe_wait_max_us: AtomicU64,
    pub(super) audio_wait_max_us: AtomicU64,
    pub(super) x86_underruns: AtomicU32,
    pub(super) x86_overruns: AtomicU32,
    pub(super) x86_alive: AtomicBool,
    pub(super) midi_delivery_max_us: AtomicU64,
    pub(super) midi_delivery_histogram: [AtomicU64; 512],
    pub(super) callback_count: AtomicU64,
    pub(super) callback_last_qpc: AtomicU64,
    pub(super) callback_interval_max_us: AtomicU64,
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
            render_underflows: AtomicU64::new(0),
            render_missing_frames: AtomicU64::new(0),
            render_reset_silence_frames: AtomicU64::new(0),
            render_queue_frames: AtomicU64::new(0),
            render_ahead_samples: AtomicU32::new(0),
            render_running: AtomicBool::new(false),
            render_busy_since_qpc: AtomicU64::new(0),
            render_last_completed_qpc: AtomicU64::new(0),
            render_block_max_us: AtomicU32::new(0),
            render_over_budget: AtomicU64::new(0),
            callback_gap_count: AtomicU64::new(0),
            mmcss_enabled: AtomicU8::new(0),
            mmcss_owned: AtomicU8::new(0),
            mmcss_critical: AtomicU8::new(0),
            callback_gate: super::callback_gate::CallbackGate::default(),
            editor_open: AtomicBool::new(false),
            parameter_accepted_sequence: AtomicU64::new(0),
            parameter_applied_sequence: AtomicU64::new(0),
            parameter_failed_sequence: AtomicU64::new(0),
            midi_generation: AtomicU64::new(0),
            pipe_wait_max_us: AtomicU64::new(0),
            audio_wait_max_us: AtomicU64::new(0),
            x86_underruns: AtomicU32::new(0),
            x86_overruns: AtomicU32::new(0),
            x86_alive: AtomicBool::new(true),
            midi_delivery_max_us: AtomicU64::new(0),
            midi_delivery_histogram: std::array::from_fn(|_| AtomicU64::new(0)),
            callback_count: AtomicU64::new(0),
            callback_last_qpc: AtomicU64::new(0),
            callback_interval_max_us: AtomicU64::new(0),
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

    pub(super) fn record_midi_delivery(&self, packet: MidiPacket) {
        #[cfg(feature = "latency-diagnostics")]
        crate::latency_trace::record(
            crate::latency_trace::Stage::DspDequeued,
            &packet.data[..usize::from(packet.len)],
            packet.ingress_qpc,
            crate::vst_worker::protocol::monotonic_qpc(),
        );
        self.record_midi_delivery_at(
            packet.ingress_qpc,
            packet.worker_qpc,
            crate::vst_worker::protocol::monotonic_qpc(),
        );
    }
    pub(super) fn record_midi_delivery_at(
        &self,
        ingress_qpc: u64,
        worker_qpc: u64,
        consumed_qpc: u64,
    ) {
        use crate::vst_worker::protocol::qpc_elapsed_us;
        self.audio_wait_max_us
            .fetch_max(qpc_elapsed_us(worker_qpc, consumed_qpc), Ordering::Relaxed);
        let delay = qpc_elapsed_us(ingress_qpc, consumed_qpc);
        self.midi_delivery_max_us
            .fetch_max(delay, Ordering::Relaxed);
        self.midi_delivery_histogram[(delay / 250).min(511) as usize]
            .fetch_add(1, Ordering::Relaxed);
    }
    pub(super) fn midi_delivery_percentile(&self, percentile: u64) -> u64 {
        let total: u64 = self
            .midi_delivery_histogram
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .sum();
        if total == 0 {
            return 0;
        }
        let target = (total * percentile).div_ceil(100);
        let mut sum = 0;
        for (index, bucket) in self.midi_delivery_histogram.iter().enumerate() {
            sum += bucket.load(Ordering::Relaxed);
            if sum >= target {
                return if index == 511 {
                    self.midi_delivery_max_us.load(Ordering::Relaxed)
                } else {
                    (index as u64 + 1) * 250
                };
            }
        }
        0
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
    pub(super) renderer: Option<super::render_ahead::DspWorkerGuard>,
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
pub(super) const OVERSIZED_CALLBACK_ERROR: u32 = 13;

#[derive(Clone, Copy, Debug)]
#[cfg_attr(target_pointer_width = "32", allow(dead_code))]
pub(super) struct ParameterCommand {
    pub(super) sequence: u64,
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
    pub(super) worker_qpc: u64,
    pub(super) generation: u64,
    pub(super) data: [u8; 3],
    pub(super) len: u8,
    pub(super) timestamp_us: u64,
    pub(super) ingress_qpc: u64,
}

impl MidiPacket {
    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Self::from_bytes_with_age(bytes, 0)
    }

    pub(super) fn from_bytes_with_age(bytes: &[u8], age_us: u64) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > 3 || bytes[0] == 0xF0 {
            return None;
        }
        let mut data = [0u8; 3];
        let len = bytes.len();
        for (idx, byte) in bytes.iter().take(len).enumerate() {
            data[idx] = *byte;
        }
        if !crate::midi::normalize_message(&mut data[..len]) {
            return None;
        }
        Some(Self {
            worker_qpc: crate::vst_worker::protocol::monotonic_qpc(),
            generation: 0,
            data,
            len: len as u8,
            timestamp_us: monotonic_us().saturating_sub(age_us),
            ingress_qpc: crate::vst_worker::protocol::monotonic_qpc(),
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
        self.is_note_off()
            || self.is_cc64_off()
            || crate::midi::is_critical_release_message(&self.data[..usize::from(self.len)])
    }

    pub(super) fn age_us(self, now_us: u64) -> u64 {
        now_us.saturating_sub(self.timestamp_us)
    }
}

pub(super) struct AudioCallbackState {
    pub(super) render_source_end_us: Option<u64>,
    pub(super) midi_generation: u64,
    pub(super) midi_rx: Consumer<MidiPacket>,
    pub(super) pending_midi: VecDeque<MidiPacket>,
    pub(super) midi_playback: super::midi_playback::MidiPlaybackClock,
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
    pub(super) equalizer: super::equalizer::StereoEqualizer,
    pub(super) telemetry: Arc<AudioTelemetry>,
    pub(super) emergency_reset_requested: Arc<AtomicBool>,
    pub(super) sample_rate: u32,
    pub(super) last_output: Vec<f32>,
    pub(super) needs_emergency_reset: bool,
    pub(super) last_frames: usize,
    pub(super) error_count: u32,
    pub(super) recovering_from_silence: bool,
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

        let (eq_revision, eq_target) = controls
            .equalizer
            .load()
            .expect("initial EQ snapshot must be coherent");

        Self {
            midi_generation: 0,
            render_source_end_us: None,
            midi_rx,
            pending_midi: VecDeque::with_capacity(MAX_PENDING_MIDI),
            midi_playback: super::midi_playback::MidiPlaybackClock::default(),
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
            equalizer: super::equalizer::StereoEqualizer::new(eq_revision, eq_target, sample_rate),
            telemetry,
            emergency_reset_requested,
            sample_rate,
            last_output: vec![0.0; max_frames.saturating_mul(device_channels)],
            needs_emergency_reset: false,
            last_frames: 0,
            error_count: 0,
            recovering_from_silence: false,
        }
    }

    pub(super) fn prepare(&mut self, frames: usize) -> bool {
        if frames > self.max_frames {
            // The advertised backend maximum has changed or was inaccurate.
            // Keep the buffers fixed, but let the supervisor recover the stream
            // instead of silently rejecting every subsequent oversized block.
            if self
                .telemetry
                .stream_fatal_error
                .compare_exchange(
                    0,
                    OVERSIZED_CALLBACK_ERROR,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.telemetry
                    .stream_recovery_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.emergency_reset_requested
                .store(true, Ordering::Release);
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
        if self.emergency_reset_requested.swap(false, Ordering::AcqRel) {
            // A standalone panic must invalidate ring entries as well as the
            // callback backlog. Producers stamp subsequent events with the new epoch.
            self.telemetry
                .midi_generation
                .fetch_add(1, Ordering::AcqRel);
            self.needs_emergency_reset = true;
            self.pending_midi.clear();
            self.midi_playback.reset();
            self.last_output.fill(0.0);
            self.recovering_from_silence = true;
        }
        self.observe_midi_generation();
        let now_us = monotonic_us();
        // Age is telemetry, not an overflow condition. A delayed wake-up must
        // not discard valid Note-Ons/controllers while queue capacity remains.
        // Generation resets already invalidate obsolete pre-panic messages.
        for _ in 0..MIDI_DRAIN_BUDGET_PER_CALLBACK {
            let Ok(msg) = self.midi_rx.pop() else {
                break;
            };
            self.observe_midi_generation();
            if msg.generation == self.midi_generation {
                self.enqueue_midi(msg);
            }
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

    pub(super) fn next_midi_timestamp_us(&self) -> Option<u64> {
        self.pending_midi
            .front()
            .or_else(|| self.midi_rx.peek().ok())
            .map(|packet| packet.timestamp_us)
    }

    fn observe_midi_generation(&mut self) {
        let generation = self.telemetry.midi_generation.load(Ordering::Acquire);
        if generation != self.midi_generation {
            self.midi_generation = generation;
            self.pending_midi.clear();
            self.midi_playback.reset();
            self.last_output.fill(0.0);
            self.recovering_from_silence = true;
            self.needs_emergency_reset = true;
        }
    }

    pub(super) fn enqueue_midi(&mut self, msg: MidiPacket) {
        if !msg.is_critical_release()
            && self.pending_midi.len()
                >= MAX_PENDING_MIDI - crate::realtime::reserve(MAX_PENDING_MIDI)
        {
            self.telemetry
                .midi_drop_count
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
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

        let discarded = self.pending_midi.len() as u32;
        self.pending_midi.clear();
        self.midi_playback.reset();
        self.pending_midi.push_back(msg);
        self.needs_emergency_reset = true;
        self.telemetry
            .midi_drop_count
            .fetch_add(discarded, Ordering::Relaxed);
    }

    fn drop_oldest_note_on(&mut self) -> bool {
        let pos = self.pending_midi.iter().position(|m| m.is_note_on());
        if let Some(idx) = pos {
            // Preserve release/controller ordering when evicting a lower-priority message.
            self.pending_midi.remove(idx);
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
            // Preserve release/controller ordering when evicting a lower-priority message.
            self.pending_midi.remove(idx);
            return true;
        }
        false
    }

    pub(super) fn record_error(&mut self) {
        self.error_count = self.error_count.wrapping_add(1);
    }

    pub(super) fn drain_parameter_commands(&mut self) {
        self.drain_parameter_commands_limit(MIDI_EVENT_BATCH_CAPACITY);
    }

    pub(super) fn drain_parameter_commands_limit(&mut self, limit: usize) {
        self.parameter_commands.clear();
        for _ in 0..limit.min(MIDI_EVENT_BATCH_CAPACITY) {
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

fn default_game_mode() -> bool {
    true
}
