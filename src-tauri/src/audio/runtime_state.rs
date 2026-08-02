#![allow(deprecated)]

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use cpal::Stream;
use parking_lot::Mutex;
use rack::prelude::MidiEvent as RackMidiEvent;
use rtrb::Consumer;
use serde::{Deserialize, Serialize};
use vst::{editor::Editor, host::PluginInstance};
use windows::Win32::Foundation::HWND;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSettings {
    pub enabled: bool,
    pub backend: Option<String>,
    pub device: Option<String>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub gain_db: f32,
    #[serde(default)]
    pub limiter_enabled: bool,
    pub vst_path: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Some("auto".to_string()),
            device: None,
            sample_rate: 48_000,
            buffer_size: 256,
            gain_db: 0.0,
            limiter_enabled: false,
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
    },
    Vst3 {
        instance: rack::vst3::Vst3Plugin,
        input_channels: usize,
        output_channels: usize,
    },
}

#[repr(align(64))]
pub(super) struct AudioControls {
    pub(super) gain_bits: AtomicU32,
    pub(super) limiter_enabled: AtomicBool,
}

#[repr(align(64))]
pub(super) struct AudioTelemetry {
    pub(super) xruns: AtomicU32,
    pub(super) meter_left: AtomicU32,
    pub(super) meter_right: AtomicU32,
    pub(super) block_size_frames: AtomicU32,
    pub(super) midi_drop_count: AtomicU32,
    pub(super) audio_lock_miss_count: AtomicU32,
    pub(super) emergency_reset_count: AtomicU32,
    pub(super) callback_last_us: AtomicU32,
    pub(super) callback_max_us: AtomicU32,
    pub(super) callback_over_budget_count: AtomicU32,
}

pub(super) struct AudioRuntime {
    pub(super) _stream: Stream,
    pub(super) plugin: Arc<Mutex<PluginBackend>>,
    pub(super) editor_window: Arc<Mutex<Option<EditorWindow>>>,
    pub(super) controls: Arc<AudioControls>,
    pub(super) telemetry: Arc<AudioTelemetry>,
    pub(super) sample_rate: u32,
    pub(super) requested_buffer_size: u32,
    pub(super) stream_buffer_size: Option<u32>,
    pub(super) vst_midi_compatible: bool,
    pub(super) backend: String,
    pub(super) device: String,
    pub(super) vst_path: PathBuf,
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
    pub(super) timestamp_ms: u64,
}

impl MidiPacket {
    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
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
            timestamp_ms: timestamp_ms(),
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
}

pub(super) struct AudioCallbackState {
    pub(super) midi_rx: Consumer<MidiPacket>,
    pub(super) pending_midi: VecDeque<MidiPacket>,
    pub(super) midi_events: Vec<RackMidiEvent>,
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
    #[cfg(target_os = "windows")]
    pub(super) mmcss_applied: bool,
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
            #[cfg(target_os = "windows")]
            mmcss_applied: false,
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
        for _ in 0..MIDI_DRAIN_BUDGET_PER_CALLBACK {
            let Ok(msg) = self.midi_rx.pop() else {
                break;
            };
            self.enqueue_midi(msg);
        }
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
}

pub(super) fn timestamp_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
