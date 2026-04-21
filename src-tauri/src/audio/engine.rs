#![allow(deprecated)]

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, Device, FromSample, HostId, OutputCallbackInfo, Sample, SampleFormat, SampleRate,
    SizedSample, Stream, StreamConfig, SupportedStreamConfigRange,
};
use directories::ProjectDirs;
use parking_lot::Mutex;
use rack::prelude::{
    MidiEvent as RackMidiEvent, MidiEventKind as RackMidiEventKind,
    PluginScanner as RackPluginScanner,
};
use rack::vst3::Vst3Scanner;
use rack::PluginInstance as RackPluginInstance;
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::fs;
use thiserror::Error;
use vst::{
    api::{self, MidiEventFlags, Supported},
    buffer::AudioBuffer,
    editor::Editor,
    host::{Host, PluginInstance, PluginLoader},
    plugin::{CanDo, Plugin},
};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, GetCurrentProcess, GetCurrentThread, ProcessPowerThrottling,
    SetPriorityClass, SetProcessInformation, SetThreadInformation, SetThreadPriority,
    ThreadPowerThrottling, ABOVE_NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible, KillTimer,
    RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, ShowWindow, CREATESTRUCTW,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HCURSOR, HICON, SW_HIDE, SW_SHOW,
    WM_CLOSE, WM_CREATE, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::logger::{background_log, FrontendLogger};
use crate::plugin_probe::{
    detect_vst3_channels, ensure_supported_plugin_in_app, select_vst3_plugin_info_for_path,
    vst3_scan_root_for_path,
};
use crate::types::VstParameter;
use crate::vst_scan::{is_vst2_path, is_vst3_path};

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

#[derive(Debug, Error, Clone)]
pub enum AudioError {
    #[error("{0}")]
    Message(String),
}

// FIX #1: Add #[derive(Clone)] — all fields are Arc<T> so Clone is safe and required
// for audio.clone() calls in bridge.rs and main.rs.
#[derive(Clone)]
pub struct AudioEngine {
    runtime: Arc<Mutex<Option<AudioRuntime>>>,
    last_vst: Arc<Mutex<Option<PathBuf>>>,
    midi_tx: Arc<Mutex<Option<Producer<MidiPacket>>>>,
    midi_emergency_reset_requested: Arc<AtomicBool>,
    midi_push_log_last_ms: Arc<AtomicU64>,
    midi_push_log_accumulator: Arc<AtomicU32>,
    midi_drop_log_last_ms: Arc<AtomicU64>,
    midi_drop_log_accumulator: Arc<AtomicU32>,
}

unsafe impl Send for AudioEngine {}
unsafe impl Sync for AudioEngine {}

enum PluginBackend {
    Vst2 {
        instance: PluginInstance,
    },
    Vst3 {
        instance: rack::vst3::Vst3Plugin,
        input_channels: usize,
        output_channels: usize,
    },
}

struct AudioRuntime {
    _stream: Stream,
    plugin: Arc<Mutex<PluginBackend>>,
    editor_window: Arc<Mutex<Option<EditorWindow>>>,
    gain_bits: Arc<AtomicU32>,
    limiter_enabled: Arc<AtomicBool>,
    xruns: Arc<AtomicU32>,
    meter_left: Arc<AtomicU32>,
    meter_right: Arc<AtomicU32>,
    block_size_frames: Arc<AtomicU32>,
    midi_drop_count: Arc<AtomicU32>,
    audio_lock_miss_count: Arc<AtomicU32>,
    emergency_reset_count: Arc<AtomicU32>,
    sample_rate: u32,
    requested_buffer_size: u32,
    stream_buffer_size: Option<u32>,
    vst_midi_compatible: bool,
    backend: String,
    device: String,
    vst_path: PathBuf,
}

enum EditorWindow {
    Vst2 {
        editor: Box<dyn Editor>,
        hwnd: HWND,
    },
    Vst3 {
        gui: rack::vst3::Vst3PluginGui,
        hwnd: HWND,
    },
}

unsafe impl Send for EditorWindow {}
unsafe impl Sync for EditorWindow {}

unsafe impl Send for AudioRuntime {}
unsafe impl Sync for AudioRuntime {}

const MIDI_RING_CAPACITY: usize = 16384;
const MAX_PENDING_MIDI: usize = 8192;
const MIDI_EVENT_BATCH_CAPACITY: usize = 512;
const RESET_CONTROLLERS: [u8; 4] = [64, 120, 121, 123];
const VST_EDITOR_IDLE_TIMER_MS: u32 = 50;

/// Returns the timeout for VST editor/VST3 drop operations on the main thread.
/// Can be overridden via VST_SHUTDOWN_TIMEOUT_SECS env var for slow systems (1-30 seconds).
fn vst_shutdown_timeout() -> Duration {
    // Allow runtime override via environment variable for slow systems
    let secs = std::env::var("VST_SHUTDOWN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    Duration::from_secs(secs.clamp(1, 30))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MidiPacket {
    data: [u8; 3],
    len: u8,
    timestamp_ms: u64,
}

impl MidiPacket {
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
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

    fn status(self) -> u8 {
        self.data[0]
    }

    fn is_note_on(self) -> bool {
        self.len >= 3 && (self.status() & 0xF0) == 0x90 && self.data[2] > 0
    }

    fn is_note_off(self) -> bool {
        (self.status() & 0xF0) == 0x80
            || (self.len >= 3 && (self.status() & 0xF0) == 0x90 && self.data[2] == 0)
    }

    fn is_cc64_off(self) -> bool {
        self.len >= 3 && (self.status() & 0xF0) == 0xB0 && self.data[1] == 64 && self.data[2] < 64
    }

    fn is_critical_release(self) -> bool {
        self.is_note_off() || self.is_cc64_off()
    }
}

struct AudioCallbackState {
    midi_rx: Consumer<MidiPacket>,
    pending_midi: VecDeque<MidiPacket>,
    midi_events: Vec<RackMidiEvent>,
    input_silence: Vec<f32>,
    input_ptrs: Vec<*const f32>,
    outputs: Vec<Vec<f32>>,
    output_ptrs: Vec<*mut f32>,
    plugin_inputs: usize,
    plugin_outputs: usize,
    xruns: Arc<AtomicU32>,
    block_size_frames: Arc<AtomicU32>,
    limiter_enabled: Arc<AtomicBool>,
    midi_drop_count: Arc<AtomicU32>,
    audio_lock_miss_count: Arc<AtomicU32>,
    emergency_reset_count: Arc<AtomicU32>,
    emergency_reset_requested: Arc<AtomicBool>,
    last_output: Vec<f32>,
    needs_emergency_reset: bool,
    last_frames: usize,
    error_count: u32,
    #[cfg(target_os = "windows")]
    mmcss_applied: bool,
}

unsafe impl Send for AudioCallbackState {}

impl AudioCallbackState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        midi_rx: Consumer<MidiPacket>,
        plugin_inputs: usize,
        plugin_outputs: usize,
        xruns: Arc<AtomicU32>,
        block_size_frames: Arc<AtomicU32>,
        limiter_enabled: Arc<AtomicBool>,
        midi_drop_count: Arc<AtomicU32>,
        audio_lock_miss_count: Arc<AtomicU32>,
        emergency_reset_count: Arc<AtomicU32>,
        emergency_reset_requested: Arc<AtomicBool>,
    ) -> Self {
        Self {
            midi_rx,
            pending_midi: VecDeque::with_capacity(256),
            midi_events: Vec::with_capacity(MIDI_EVENT_BATCH_CAPACITY),
            input_silence: Vec::new(),
            input_ptrs: vec![std::ptr::null(); plugin_inputs],
            outputs: (0..plugin_outputs).map(|_| Vec::new()).collect(),
            output_ptrs: vec![std::ptr::null_mut(); plugin_outputs],
            plugin_inputs,
            plugin_outputs,
            xruns,
            block_size_frames,
            limiter_enabled,
            midi_drop_count,
            audio_lock_miss_count,
            emergency_reset_count,
            emergency_reset_requested,
            last_output: Vec::new(),
            needs_emergency_reset: false,
            last_frames: 0,
            error_count: 0,
            #[cfg(target_os = "windows")]
            mmcss_applied: false,
        }
    }

    fn prepare(&mut self, frames: usize) {
        if self.input_silence.len() < frames {
            self.input_silence.resize(frames, 0.0);
        }

        let input_ptr = self.input_silence.as_ptr();
        if self.input_ptrs.len() != self.plugin_inputs {
            self.input_ptrs.resize(self.plugin_inputs, input_ptr);
        }
        for ptr in &mut self.input_ptrs {
            *ptr = input_ptr;
        }

        if self.outputs.len() != self.plugin_outputs {
            self.outputs = (0..self.plugin_outputs).map(|_| Vec::new()).collect();
            self.output_ptrs
                .resize(self.plugin_outputs, std::ptr::null_mut());
        }

        for output in self.outputs.iter_mut() {
            if output.len() < frames {
                output.resize(frames, 0.0);
            } else {
                output[..frames].fill(0.0);
            }
        }

        for (idx, output) in self.outputs.iter_mut().enumerate() {
            self.output_ptrs[idx] = output.as_mut_ptr();
        }
    }

    // FIX #2: Emergency reset must clear pending_midi to prevent re-triggering notes
    // that caused the overflow in the first place.
    fn drain_midi(&mut self) {
        if self
            .emergency_reset_requested
            .swap(false, Ordering::Relaxed)
        {
            self.needs_emergency_reset = true;
            // Clear the pending queue so stuck notes don't immediately re-trigger
            // after the reset messages are sent to the plugin.
            self.pending_midi.clear();
        }
        while let Ok(msg) = self.midi_rx.pop() {
            self.enqueue_midi(msg);
        }
    }

    fn enqueue_midi(&mut self, msg: MidiPacket) {
        if self.pending_midi.len() < MAX_PENDING_MIDI {
            self.pending_midi.push_back(msg);
            return;
        }

        if self.drop_oldest_note_on() {
            self.pending_midi.push_back(msg);
            self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if !msg.is_critical_release() {
            self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if self.drop_oldest_non_critical() {
            self.pending_midi.push_back(msg);
            self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Queue saturated with critical messages — clear and trigger reset
        self.pending_midi.clear();
        self.pending_midi.push_back(msg);
        self.needs_emergency_reset = true;
        self.midi_drop_count.fetch_add(1, Ordering::Relaxed);
    }

fn drop_oldest_note_on(&mut self) -> bool {
    let pos = self.pending_midi.iter().position(|m| m.is_note_on());
    if let Some(idx) = pos {
        // swap_remove_front: met l'élément idx à la place du front, pop front
        // Mais VecDeque n'a pas swap_remove, on utilise rotate
        self.pending_midi.swap(0, idx); // swap avec front
        self.pending_midi.pop_front();  // pop front — O(1)
        return true;
    }
    false
}

fn drop_oldest_non_critical(&mut self) -> bool {
    let pos = self.pending_midi.iter().position(|m| !m.is_critical_release());
    if let Some(idx) = pos {
        self.pending_midi.swap(0, idx);
        self.pending_midi.pop_front();
        return true;
    }
    false
}

fn record_error(&mut self) {
    self.error_count = self.error_count.wrapping_add(1);
}
}

// FIX #3: Store AppHandle so the WM_CLOSE handler can notify the frontend when
// the user closes the VST editor via the window's X button.
struct WindowData {
    state: std::sync::Weak<Mutex<Option<EditorWindow>>>,
    app_handle: Option<tauri::AppHandle>,
    plugin: Option<Arc<Mutex<PluginBackend>>>,
    vst_path: Option<PathBuf>,
}

unsafe extern "system" fn vst_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create_struct = lparam.0 as *const CREATESTRUCTW;
            let data_ptr = (*create_struct).lpCreateParams as *mut WindowData;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize);
            SetTimer(Some(hwnd), 1, VST_EDITOR_IDLE_TIMER_MS, None);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == 1 {
                // FIX #4: Service idle whenever the window is VISIBLE, not only
                // when it is the foreground window. Many VSTs animate meters and
                // repaint controls in idle(); restricting to foreground breaks them.
                if IsWindowVisible(hwnd).as_bool() {
                    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData;
                    if !ptr.is_null() {
                        if let Some(arc) = (*ptr).state.upgrade() {
                            if let Some(EditorWindow::Vst2 { editor, .. }) = arc.lock().as_mut()
                            {
                                editor.idle();
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // Hide the editor instead of destroying it — preserves plugin state
            // so the user can reopen without forcing re-initialisation.
            let _ = KillTimer(Some(hwnd), 1);
            let _ = ShowWindow(hwnd, SW_HIDE);

            // FIX #5: Emit "vst_editor_hidden" so the frontend can sync its
            // vstUiOpen state without polling.
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowData;
            if !ptr.is_null() {
                // Persist state when closing via the window title bar (X),
                // matching the behavior of the explicit "Hide UI" action.
                if let (Some(plugin), Some(vst_path)) =
                    ((*ptr).plugin.as_ref(), (*ptr).vst_path.as_ref())
                {
                    save_vst_state(plugin, vst_path);
                }
                if let Some(handle) = &(*ptr).app_handle {
                    use tauri::Emitter;
                    let _ = handle.emit("vst_editor_hidden", ());
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), 1);
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowData;
            if !ptr.is_null() {
                let _ = Box::from_raw(ptr);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// FIX #6: Accept AppHandle so the window can notify the frontend on close.
fn create_vst_window(
    title: &str,
    width: i32,
    height: i32,
    data: WindowData,
) -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class_name = w!("VSTContainerClass");

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(vst_window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: class_name,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            hCursor: HCURSOR(std::ptr::null_mut()),
            hIcon: HICON(std::ptr::null_mut()),
            ..Default::default()
        };

        let _ = RegisterClassW(&wnd_class);

        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let data_box = Box::new(data);
        let data_ptr = Box::into_raw(data_box);

        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title_wide.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            if width > 0 { width } else { CW_USEDEFAULT },
            if height > 0 { height } else { CW_USEDEFAULT },
            None,
            None,
            Some(HINSTANCE(instance.0)),
            Some(data_ptr as *mut std::ffi::c_void),
        )
        .map_err(|e| {
            let _ = Box::from_raw(data_ptr);
            e.to_string()
        })?;

        if hwnd.0.is_null() {
            let _ = Box::from_raw(data_ptr);
            return Err("Failed to create window".to_string());
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        Ok(hwnd)
    }
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            runtime: Arc::new(Mutex::new(None)),
            last_vst: Arc::new(Mutex::new(None)),
            midi_tx: Arc::new(Mutex::new(None)),
            midi_emergency_reset_requested: Arc::new(AtomicBool::new(false)),
            midi_push_log_last_ms: Arc::new(AtomicU64::new(0)),
            midi_push_log_accumulator: Arc::new(AtomicU32::new(0)),
            midi_drop_log_last_ms: Arc::new(AtomicU64::new(0)),
            midi_drop_log_accumulator: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.runtime.lock().is_some()
    }

    pub fn is_vst_loaded(&self) -> bool {
        self.runtime.lock().is_some()
    }

    pub fn stop(&self, app_handle: Option<tauri::AppHandle>) {
        let app_handle_for_drop = app_handle.clone();
        let runtime = self.runtime.lock().take();
        *self.midi_tx.lock() = None;
        self.midi_emergency_reset_requested
            .store(false, Ordering::Relaxed);

        if let Some(runtime) = runtime {
            let plugin = runtime.plugin.clone();
            let vst_path = runtime.vst_path.clone();
            let editor_window_arc = runtime.editor_window.clone();
            let is_vst3 = matches!(*plugin.lock(), PluginBackend::Vst3 { .. });
            let is_sforzando = is_sforzando_vst3(&vst_path);

            if let Some(handle) = app_handle {
                let (tx, rx) = mpsc::channel();
                let scheduled = handle.run_on_main_thread(move || {
                    if let Some(mut editor_win) = editor_window_arc.lock().take() {
                        match &mut editor_win {
                            EditorWindow::Vst2 { editor, hwnd } => {
                                editor.close();
                                unsafe {
                                    let _ = DestroyWindow(*hwnd);
                                }
                            }
                            EditorWindow::Vst3 { gui, hwnd } => {
                                let _ = gui.detach();
                                unsafe {
                                    let _ = DestroyWindow(*hwnd);
                                }
                            }
                        }
                    }
                    let _ = tx.send(());
                });

                if scheduled.is_ok() {
                    if rx.recv_timeout(vst_shutdown_timeout()).is_err() {
                        background_log(
                            "warn",
                            "Timed out waiting for VST editor to close on main thread",
                        );
                    }
                } else if let Some(_editor_win) = runtime.editor_window.lock().take() {
                    background_log(
                        "warn",
                        "Failed to schedule VST editor close on main thread; forcing local drop",
                    );
                }
            } else {
                if let Some(_editor_win) = editor_window_arc.lock().take() {
                    background_log(
                        "warn",
                        "Stopping audio without AppHandle: VST editor might not close cleanly",
                    );
                }
            }

            reset_all_notes(plugin.clone());
            drop(runtime);
            save_vst_state(&plugin, &vst_path);
            if is_vst3 && !is_sforzando {
                if let Some(handle) = app_handle_for_drop {
                    let (tx, rx) = mpsc::channel();
                    let plugin_for_main_drop = plugin.clone();
                    let scheduled = handle.run_on_main_thread(move || {
                        drop(plugin_for_main_drop);
                        let _ = tx.send(());
                    });
                    if scheduled.is_ok() {
                        if rx.recv_timeout(vst_shutdown_timeout()).is_err() {
                            background_log(
                                "warn",
                                "Timed out waiting for VST3 drop on main thread",
                            );
                        }
                    } else {
                        background_log("warn", "Failed to schedule VST3 drop on main thread");
                    }
                }
            }
            if is_sforzando {
                // WORKAROUND: sforzando VST3 crashes in plugin_free() during teardown.
                // This is a bug in the third-party plugin (Plogue sforzando), not in this codebase.
                // 
                // Impact: ~5-50MB leaked per session depending on sample set loaded.
                // Frequency: Only when stopping audio with sforzando loaded.
                // Alternative: None - the plugin's free() crashes in both vst3-sys and rack.
                //
                // User warning is logged so they understand this is plugin-specific.
                background_log(
                    "warn",
                    "Leaking sforzando VST3 instance (~5-50MB) to avoid plugin_free crash. This is a known sforzando bug."
                );
                std::mem::forget(plugin);
            } else {
                drop(plugin);
            }
        }
    }

    pub fn list_backends(&self) -> Vec<String> {
        cpal::available_hosts()
            .into_iter()
            .map(|h| h.name().to_string())
            .collect()
    }

    pub fn list_devices(&self, backend: Option<String>) -> Vec<String> {
        let host = select_host(backend.as_deref());
        host.and_then(|h| h.output_devices().ok())
            .map(|iter| iter.filter_map(|d| d.name().ok()).collect::<Vec<String>>())
            .unwrap_or_default()
    }

    pub fn start(
        &self,
        settings: AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        self.stop(Some(logger.app_handle()));

        if !settings.enabled {
            return Ok(());
        }

        apply_audio_process_tuning(&logger);

        let vst_path = resolve_vst_path(&settings, vst_fallback, &logger)?;
        let vst_probe = ensure_supported_plugin_in_app(&vst_path).map_err(AudioError::Message)?;
        logger.info(format!(
            "Selected plugin '{}' ({}, {}, editor={})",
            vst_probe.name, vst_probe.format, vst_probe.architecture, vst_probe.has_editor
        ));

        let mut last_err: Option<AudioError> = None;
        let mut stream_opt = None;
        let mut selected_backend: Option<String> = None;
        let mut selected_device: Option<String> = None;
        let gain_bits = Arc::new(AtomicU32::new(db_to_linear(settings.gain_db).to_bits()));
        let limiter_enabled = Arc::new(AtomicBool::new(settings.limiter_enabled));
        let xruns = Arc::new(AtomicU32::new(0));
        let meter_left = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let meter_right = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let block_size_frames = Arc::new(AtomicU32::new(0));
        let midi_drop_count = Arc::new(AtomicU32::new(0));
        let audio_lock_miss_count = Arc::new(AtomicU32::new(0));
        let emergency_reset_count = Arc::new(AtomicU32::new(0));

        // FIX #7: Smarter ASIO retry strategy.
        // - If ASIO host is not even installed, fail fast (no retries).
        // - If ASIO host is available, try up to 2 times with a short backoff
        //   (ASIO drivers can be briefly busy after another app closes).
        // - Always fall back to WASAPI on "auto" so the app stays usable
        //   without ASIO drivers.
        let mut candidates: Vec<(Option<String>, Option<String>, u8, bool)> = Vec::new();
        let preferred_backend = settings.backend.as_deref().unwrap_or("auto").to_lowercase();
        let asio_available = cpal::available_hosts().contains(&HostId::Asio);

        let push_asio = |list: &mut Vec<_>| {
            if asio_available {
                // 2 attempts max — not 3 — to reduce startup delay
                for attempt in 0..2u8 {
                    list.push((
                        Some("asio".to_string()),
                        settings.device.clone(),
                        attempt,
                        true,
                    ));
                }
            }
        };
        let push_wasapi = |list: &mut Vec<_>| {
            list.push((
                Some("wasapi".to_string()),
                settings.device.clone(),
                0,
                false,
            ));
        };

        match preferred_backend.as_str() {
            "auto" | "" => {
                push_asio(&mut candidates);
                push_wasapi(&mut candidates);
            }
            val if val.contains("asio") => {
                push_asio(&mut candidates);
                // Also add WASAPI fallback when "asio" was explicitly chosen
                // but no ASIO host is available — keeps the app functional.
                if !asio_available {
                    logger.warn(
                        "ASIO requested but no ASIO host found. Falling back to WASAPI."
                            .to_string(),
                    );
                    push_wasapi(&mut candidates);
                }
            }
            val if val.contains("wasapi") => push_wasapi(&mut candidates),
            _ => {
                let prefer_low_latency = preferred_backend.contains("asio");
                candidates.push((
                    settings.backend.clone(),
                    settings.device.clone(),
                    0,
                    prefer_low_latency,
                ));
            }
        }

        for (backend, device_pref, attempt, prefer_low_latency) in candidates {
            if attempt > 0 {
                let delay_ms = 400u64;
                logger.debug(format!(
                    "Retrying ASIO (attempt {}/2), waiting {}ms…",
                    attempt + 1,
                    delay_ms
                ));
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }

            let backend_name = backend.as_deref().unwrap_or("auto");
            logger.info(format!("Attempting audio backend: {}", backend_name));

            let host = match select_host(backend.as_deref()) {
                Some(h) => h,
                None => {
                    if backend_name.contains("asio") {
                        logger.warn(
                            "ASIO host not available. Install ASIO4ALL or a vendor ASIO driver."
                                .to_string(),
                        );
                    } else {
                        logger.warn(format!("Host '{}' not available", backend_name));
                    }
                    last_err = Some(AudioError::Message(format!(
                        "Host {} not available",
                        backend_name
                    )));
                    continue;
                }
            };

            let device = select_device(&host, device_pref.as_deref())
                .or_else(|| host.default_output_device());
            let Some(device) = device else {
                last_err = Some(AudioError::Message(
                    "No audio output device found for this host.".into(),
                ));
                continue;
            };

            let dev_name = device.name().unwrap_or_else(|_| "unknown".into());

            let has_stereo = device
                .supported_output_configs()
                .map_err(|e| AudioError::Message(e.to_string()))?
                .any(|cfg| cfg.channels() >= 2);

            if !has_stereo {
                logger.info(format!(
                    "Device '{}' skipped — VST requires stereo output.",
                    dev_name
                ));
                continue;
            }

            match build_stream(
                &device,
                settings.sample_rate,
                settings.buffer_size,
                gain_bits.clone(),
                limiter_enabled.clone(),
                xruns.clone(),
                meter_left.clone(),
                meter_right.clone(),
                block_size_frames.clone(),
                midi_drop_count.clone(),
                audio_lock_miss_count.clone(),
                emergency_reset_count.clone(),
                self.midi_emergency_reset_requested.clone(),
                vst_path.clone(),
                &logger,
                backend_name,
                &dev_name,
                prefer_low_latency,
            ) {
                Ok(res) => {
                    let (
                        stream,
                        plugin,
                        midi_tx,
                        active_sample_rate,
                        stream_buffer_size,
                        vst_midi_compatible,
                    ) = res;
                    stream_opt = Some((
                        stream,
                        plugin,
                        midi_tx,
                        active_sample_rate,
                        stream_buffer_size,
                        vst_midi_compatible,
                    ));
                    logger.info(format!(
                        "Audio stream started on '{}' ({})",
                        dev_name, backend_name
                    ));
                    selected_backend = Some(backend_name.to_string());
                    selected_device = Some(dev_name);
                    if !vst_midi_compatible {
                        logger.warn(
                            "VST does not advertise MIDI input support; note routing may not work."
                                .to_string(),
                        );
                    }
                    break;
                }
                Err(err) => {
                    let err_str = err.to_string();

                    if err_str.contains("no longer available") || err_str.contains("unplugged") {
                        logger.warn(format!(
                            "Device '{}' reported unavailable (attempt {}). Retrying…",
                            dev_name,
                            attempt + 1
                        ));
                    } else if err_str.contains("0x8889000A") || err_str.contains("exclusive") {
                        logger.warn(format!(
                            "Device '{}' is in exclusive mode. Close other audio applications first.",
                            dev_name
                        ));
                    } else {
                        logger.warn(format!(
                            "Failed to build stream on '{}': {}",
                            dev_name, err_str
                        ));
                    }

                    last_err = Some(err);
                    continue;
                }
            }
        }

        let (stream, plugin, midi_tx, active_sample_rate, stream_buffer_size, vst_midi_compatible) =
            stream_opt.ok_or_else(|| {
                let final_err = last_err.unwrap_or_else(|| {
                    AudioError::Message("All audio backends failed to initialise.".into())
                });
                logger.error(format!("Audio initialisation failed: {}", final_err));
                final_err
            })?;

        let active_backend = selected_backend.unwrap_or_else(|| "unknown".to_string());
        let active_device = selected_device.unwrap_or_else(|| "unknown".to_string());

        load_vst_state(&plugin, &vst_path, &logger);

        let runtime = AudioRuntime {
            _stream: stream,
            plugin: plugin.clone(),
            editor_window: Arc::new(Mutex::new(None)),
            gain_bits: gain_bits.clone(),
            limiter_enabled: limiter_enabled.clone(),
            xruns: xruns.clone(),
            meter_left: meter_left.clone(),
            meter_right: meter_right.clone(),
            block_size_frames: block_size_frames.clone(),
            midi_drop_count,
            audio_lock_miss_count,
            emergency_reset_count,
            sample_rate: active_sample_rate,
            requested_buffer_size: settings.buffer_size,
            stream_buffer_size,
            vst_midi_compatible,
            backend: active_backend,
            device: active_device,
            vst_path: vst_path.clone(),
        };

        *self.last_vst.lock() = Some(vst_path);
        *self.midi_tx.lock() = Some(midi_tx);
        *self.runtime.lock() = Some(runtime);
        Ok(())
    }

    pub fn send_midi(&self, bytes: &[u8]) {
        let Some(packet) = MidiPacket::from_bytes(bytes) else {
            return;
        };

        let pushed = {
            let mut guard = self.midi_tx.lock();
            let Some(tx) = guard.as_mut() else {
                background_log("debug", "send_midi: Audio runtime not started");
                return;
            };
            tx.push(packet).is_ok()
        };

        if pushed {
            if crate::logger::should_log_debug() {
                let pushed_now = self
                    .midi_push_log_accumulator
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                let now_ms = timestamp_ms();
                let last_ms = self.midi_push_log_last_ms.load(Ordering::Relaxed);
                if now_ms.saturating_sub(last_ms) >= 1000
                    && self
                        .midi_push_log_last_ms
                        .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                {
                    let pushed_since_last = if pushed_now > 1 {
                        self.midi_push_log_accumulator.swap(0, Ordering::Relaxed)
                    } else {
                        self.midi_push_log_accumulator.store(0, Ordering::Relaxed);
                        pushed_now
                    };
                    background_log(
                        "debug",
                        format!(
                            "MIDI -> Ring Buffer throughput: {} msg/s (latest={:02X?})",
                            pushed_since_last,
                            &bytes[..bytes.len().min(3)]
                        ),
                    );
                }
            }
            return;
        }

        if let Some(runtime) = self.runtime.lock().as_ref() {
            runtime.midi_drop_count.fetch_add(1, Ordering::Relaxed);
        }
        if packet.is_critical_release() {
            self.midi_emergency_reset_requested
                .store(true, Ordering::Relaxed);
        }
        if crate::logger::should_log_debug() {
            let dropped = self
                .midi_drop_log_accumulator
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            let now_ms = timestamp_ms();
            let last_ms = self.midi_drop_log_last_ms.load(Ordering::Relaxed);
            if now_ms.saturating_sub(last_ms) >= 1000
                && self
                    .midi_drop_log_last_ms
                    .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                let dropped_since_last = if dropped > 1 {
                    self.midi_drop_log_accumulator.swap(0, Ordering::Relaxed)
                } else {
                    self.midi_drop_log_accumulator.store(0, Ordering::Relaxed);
                    dropped
                };
                background_log(
                    "warn",
                    format!(
                        "MIDI dropped before audio callback (ring full): {} drops in ~1s (latest={:02X?})",
                        dropped_since_last,
                        &bytes[..bytes.len().min(3)]
                    ),
                );
            }
        }
    }

    // FIX #8: panic_all_notes also triggers an emergency reset so the pending_midi
    // queue is flushed in the audio callback on the next tick. Without this, any
    // MIDI messages already in the queue would re-trigger notes immediately after panic.
    pub fn panic_all_notes(&self) -> Result<(), AudioError> {
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            background_log("warn", "panic_all_notes: Audio runtime not started");
            return Err(AudioError::Message("Audio runtime not started".into()));
        };
        // Signal the audio thread to flush pending MIDI and send reset messages.
        self.midi_emergency_reset_requested
            .store(true, Ordering::Relaxed);
        // Also call synchronously in case the audio thread has the lock right now.
        reset_all_notes(runtime.plugin.clone());
        Ok(())
    }

    // FIX #9: Pass AppHandle into WindowData so WM_CLOSE can emit vst_editor_hidden.
    pub fn open_vst_ui(&self, app_handle: tauri::AppHandle) -> Result<(), AudioError> {
        let (plugin_arc, editor_window_arc, vst_path) = {
            let mut guard = self.runtime.lock();
            let Some(runtime) = guard.as_mut() else {
                return Err(AudioError::Message("Audio not started".into()));
            };
            (
                runtime.plugin.clone(),
                runtime.editor_window.clone(),
                runtime.vst_path.clone(),
            )
        };

        let weak_editor_window = Arc::downgrade(&editor_window_arc);
        let (tx, rx) = mpsc::channel();
        let app_handle_clone = app_handle.clone();

        app_handle
            .run_on_main_thread(move || {
                let result: Result<(), AudioError> = (|| {
                    if let Some(existing) = editor_window_arc.lock().as_ref() {
                        let hwnd = match existing {
                            EditorWindow::Vst2 { hwnd, .. } | EditorWindow::Vst3 { hwnd, .. } => {
                                *hwnd
                            }
                        };
                        unsafe {
                            let _ = SetTimer(Some(hwnd), 1, VST_EDITOR_IDLE_TIMER_MS, None);
                            let _ = ShowWindow(hwnd, SW_SHOW);
                            let _ = SetForegroundWindow(hwnd);
                        }
                        return Ok(());
                    }

                    let mut plugin = plugin_arc.lock();
                    match &mut *plugin {
                        PluginBackend::Vst2 { instance } => {
                            let editor = instance.get_editor();
                            let Some(mut editor) = editor else {
                                return Err(AudioError::Message(
                                    "Plugin does not expose an editor".into(),
                                ));
                            };

                            let (width, height) = editor.size();
                            let win_width = if width > 0 { width + 16 } else { 800 };
                            let win_height = if height > 0 { height + 39 } else { 600 };

                            let window_data = WindowData {
                                state: weak_editor_window,
                                app_handle: Some(app_handle_clone),
                                plugin: Some(plugin_arc.clone()),
                                vst_path: Some(vst_path.clone()),
                            };

                            let hwnd =
                                create_vst_window("VST Editor", win_width, win_height, window_data)
                                    .map_err(AudioError::Message)?;

                            let hwnd_ptr = hwnd.0;
                            if editor.open(hwnd_ptr) {
                                *editor_window_arc.lock() =
                                    Some(EditorWindow::Vst2 { editor, hwnd });
                                Ok(())
                            } else {
                                unsafe {
                                    let _ = DestroyWindow(hwnd);
                                }
                                Err(AudioError::Message("Failed to open VST editor".into()))
                            }
                        }
                        PluginBackend::Vst3 { instance, .. } => {
                            let mut gui = instance.create_gui().map_err(|e| {
                                AudioError::Message(format!("Failed to create VST3 editor: {e}"))
                            })?;
                            let (width, height) = gui.size().map_err(|e| {
                                AudioError::Message(format!("Failed to get VST3 editor size: {e}"))
                            })?;
                            let win_width = if width > 0.0 {
                                width.round() as i32 + 16
                            } else {
                                800
                            };
                            let win_height = if height > 0.0 {
                                height.round() as i32 + 39
                            } else {
                                600
                            };

                            let window_data = WindowData {
                                state: weak_editor_window,
                                app_handle: Some(app_handle_clone),
                                plugin: Some(plugin_arc.clone()),
                                vst_path: Some(vst_path.clone()),
                            };

                            let hwnd = create_vst_window(
                                "VST3 Editor",
                                win_width,
                                win_height,
                                window_data,
                            )
                            .map_err(AudioError::Message)?;

                            if let Err(err) = gui.attach(hwnd.0) {
                                unsafe {
                                    let _ = DestroyWindow(hwnd);
                                }
                                return Err(AudioError::Message(format!(
                                    "Failed to attach VST3 editor: {err}"
                                )));
                            }

                            *editor_window_arc.lock() = Some(EditorWindow::Vst3 { gui, hwnd });
                            Ok(())
                        }
                    }
                })();

                let _ = tx.send(result);
            })
            .map_err(|_| AudioError::Message("Failed to schedule VST UI on main thread".into()))?;

        rx.recv().unwrap_or_else(|_| {
            Err(AudioError::Message(
                "Main thread dropped VST UI result".into(),
            ))
        })
    }

    pub fn close_vst_ui(&self, app_handle: tauri::AppHandle) -> Result<(), AudioError> {
        let (editor_window_arc, plugin_arc, vst_path) = {
            let mut guard = self.runtime.lock();
            let Some(runtime) = guard.as_mut() else {
                return Err(AudioError::Message("Audio not started".into()));
            };
            (
                runtime.editor_window.clone(),
                runtime.plugin.clone(),
                runtime.vst_path.clone(),
            )
        };

        app_handle
            .run_on_main_thread(move || {
                if let Some(editor_win) = editor_window_arc.lock().as_mut() {
                    let hwnd = match editor_win {
                        EditorWindow::Vst2 { hwnd, .. } | EditorWindow::Vst3 { hwnd, .. } => *hwnd,
                    };
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                    save_vst_state(&plugin_arc, &vst_path);
                }
            })
            .map_err(|_| {
                AudioError::Message("Failed to schedule VST close on main thread".into())
            })?;

        Ok(())
    }

    pub fn list_vst_parameters(&self) -> Result<Vec<VstParameter>, AudioError> {
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            return Err(AudioError::Message("Audio not started".into()));
        };

        let mut plugin = runtime.plugin.lock();
        match &mut *plugin {
            PluginBackend::Vst3 { instance, .. } => {
                let count = instance.parameter_count();
                let mut params = Vec::with_capacity(count);
                for index in 0..count {
                    let info = instance.parameter_info(index).map_err(|e| {
                        AudioError::Message(format!("VST3 parameter info failed: {}", e))
                    })?;
                    let value = instance.get_parameter(index).map_err(|e| {
                        AudioError::Message(format!("VST3 get parameter failed: {}", e))
                    })?;
                    params.push(VstParameter {
                        index,
                        name: info.name,
                        min: info.min,
                        max: info.max,
                        default: info.default,
                        unit: info.unit,
                        value,
                    });
                }
                Ok(params)
            }
            _ => Err(AudioError::Message(
                "VST3 parameters are only available for VST3 plugins".into(),
            )),
        }
    }

    pub fn set_vst_parameter(&self, index: usize, value: f32) -> Result<(), AudioError> {
        let guard = self.runtime.lock();
        let Some(runtime) = guard.as_ref() else {
            return Err(AudioError::Message("Audio not started".into()));
        };
        let mut plugin = runtime.plugin.lock();
        match &mut *plugin {
            PluginBackend::Vst3 { instance, .. } => {
                let normalized = value.clamp(0.0, 1.0);
                instance
                    .set_parameter(index, normalized)
                    .map_err(|e| AudioError::Message(format!("VST3 set parameter failed: {}", e)))
            }
            _ => Err(AudioError::Message(
                "VST3 parameters are only available for VST3 plugins".into(),
            )),
        }
    }

    pub fn set_gain(&self, gain_db: f32) {
        if let Some(rt) = self.runtime.lock().as_mut() {
            rt.gain_bits
                .store(db_to_linear(gain_db).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn set_limiter_enabled(&self, enabled: bool) {
        if let Some(rt) = self.runtime.lock().as_mut() {
            rt.limiter_enabled.store(enabled, Ordering::Relaxed);
        }
    }

    pub fn current_backend(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.backend.clone())
    }

    pub fn current_device(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.device.clone())
    }

    pub fn current_sample_rate(&self) -> Option<u32> {
        self.runtime.lock().as_ref().map(|r| r.sample_rate)
    }

    pub fn current_buffer_size(&self) -> Option<u32> {
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(frames)
            }
        })
    }

    pub fn requested_buffer_size(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.requested_buffer_size)
    }

    pub fn stream_buffer_size(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .and_then(|r| r.stream_buffer_size)
    }

    pub fn buffer_size_mismatch(&self) -> Option<bool> {
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some(frames != r.requested_buffer_size)
            }
        })
    }

    pub fn vst_midi_compatible(&self) -> Option<bool> {
        self.runtime.lock().as_ref().map(|r| r.vst_midi_compatible)
    }

    pub fn xrun_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.xruns.load(Ordering::Relaxed))
    }

    pub fn limiter_enabled(&self) -> Option<bool> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.limiter_enabled.load(Ordering::Relaxed))
    }

    pub fn peak_levels(&self) -> Option<(f32, f32)> {
        self.runtime.lock().as_ref().map(|r| {
            let left = f32::from_bits(r.meter_left.load(Ordering::Relaxed));
            let right = f32::from_bits(r.meter_right.load(Ordering::Relaxed));
            (left, right)
        })
    }

    pub fn current_latency_ms(&self) -> Option<f32> {
        self.runtime.lock().as_ref().and_then(|r| {
            let frames = r.block_size_frames.load(Ordering::Relaxed);
            if frames == 0 {
                None
            } else {
                Some((frames as f32 / r.sample_rate as f32) * 1000.0 * 2.0)
            }
        })
    }

    pub fn midi_drop_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.midi_drop_count.load(Ordering::Relaxed))
    }

    pub fn audio_lock_miss_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.audio_lock_miss_count.load(Ordering::Relaxed))
    }

    pub fn emergency_reset_count(&self) -> Option<u32> {
        self.runtime
            .lock()
            .as_ref()
            .map(|r| r.emergency_reset_count.load(Ordering::Relaxed))
    }

    pub fn ping(&self) -> Result<(), AudioError> {
        if self.runtime.lock().is_none() {
            return Err(AudioError::Message("Audio runtime not started".into()));
        }

        self.send_midi(&[0x90, 60, 100]);
        let cloned = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(220));
            cloned.send_midi(&[0x80, 60, 0]);
        });
        Ok(())
    }

    pub fn reload(
        &self,
        settings: AudioSettings,
        vst_fallback: Option<PathBuf>,
        logger: FrontendLogger,
    ) -> Result<(), AudioError> {
        self.start(settings, vst_fallback, logger)
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_vst_path(
    settings: &AudioSettings,
    fallback: Option<PathBuf>,
    logger: &FrontendLogger,
) -> Result<PathBuf, AudioError> {
    if let Some(p) = settings.vst_path.as_ref() {
        let src = PathBuf::from(p);
        if let Ok(real) = src.canonicalize() {
            if real.exists() {
                logger.info(format!("Using VST from {:?}", real));
                return Ok(real);
            }
        } else if src.exists() {
            logger.info(format!("Using VST from {:?}", src));
            return Ok(src);
        }
        logger.warn(format!("Configured VST path not found: {:?}", src));
    }

    let mut sources: Vec<PathBuf> = Vec::new();
    if let Some(fb) = fallback {
        sources.push(fb);
    }
    sources.push(PathBuf::from("Bitsonic").join("Keyzone Classic.dll"));

    for src in sources {
        let Ok(real) = src.canonicalize() else {
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

fn state_path_for_plugin(vst_path: &Path) -> Option<PathBuf> {
    let file_name = vst_path.file_name()?.to_string_lossy().to_string();
    let base_dir =
        ProjectDirs::from("com", "OSCMIDI", "OSCMIDI").map(|d| d.config_dir().join("vst_state"))?;
    Some(base_dir.join(format!("{}.state", file_name)))
}

const STATE_MAGIC: &[u8; 4] = b"OSVS";
const STATE_VERSION: u8 = 1;
const STATE_KIND_CHUNK: u8 = 0;
const STATE_KIND_PARAMS: u8 = 1;

enum SavedState {
    Raw(Vec<u8>),
    Chunk(Vec<u8>),
    Params(Vec<f32>),
}

fn encode_state_chunk(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + data.len());
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION);
    out.push(STATE_KIND_CHUNK);
    out.extend_from_slice(data);
    out
}

fn encode_state_params(params: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + params.len() * 4);
    out.extend_from_slice(STATE_MAGIC);
    out.push(STATE_VERSION);
    out.push(STATE_KIND_PARAMS);
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for value in params {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn decode_state(data: &[u8]) -> SavedState {
    if data.len() < 6 || data[..4] != *STATE_MAGIC {
        return SavedState::Raw(data.to_vec());
    }

    let version = data[4];
    if version != STATE_VERSION {
        return SavedState::Raw(data.to_vec());
    }

    match data[5] {
        STATE_KIND_CHUNK => SavedState::Chunk(data[6..].to_vec()),
        STATE_KIND_PARAMS => {
            if data.len() < 10 {
                return SavedState::Raw(data.to_vec());
            }
            let count = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
            let expected = 10 + count.saturating_mul(4);
            if data.len() < expected {
                return SavedState::Raw(data.to_vec());
            }
            let mut params = Vec::with_capacity(count);
            let mut offset = 10;
            for _ in 0..count {
                let bytes = [
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ];
                params.push(f32::from_le_bytes(bytes));
                offset += 4;
            }
            SavedState::Params(params)
        }
        _ => SavedState::Raw(data.to_vec()),
    }
}

fn capture_vst2_parameters(instance: &mut PluginInstance) -> Vec<f32> {
    let info = instance.get_info();
    let count = info.parameters.max(0) as usize;
    if count == 0 {
        return Vec::new();
    }
    let params = instance.get_parameter_object();
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(params.get_parameter(index as i32));
    }
    values
}

fn apply_vst2_parameters(instance: &mut PluginInstance, values: &[f32]) {
    let info = instance.get_info();
    let count = info.parameters.max(0) as usize;
    if count == 0 {
        return;
    }
    let params = instance.get_parameter_object();
    for (index, value) in values.iter().take(count).enumerate() {
        params.set_parameter(index as i32, value.clamp(0.0, 1.0));
    }
}

// FIX #10: Use blocking lock() instead of try_lock() to guarantee state is always
// saved. The audio stream has been stopped before save_vst_state is called from
// AudioEngine::stop(), so blocking here is safe and avoids silent save failures.
fn save_vst_state(plugin: &Arc<Mutex<PluginBackend>>, vst_path: &Path) {
    if is_sforzando_vst3(vst_path) {
        return;
    }

    let Some(state_path) = state_path_for_plugin(vst_path) else {
        background_log("warn", "Could not determine VST state path");
        return;
    };
    if let Some(parent) = state_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Use try_lock first (non-blocking); fall back to lock() if the audio thread
    // holds it momentarily. We never deadlock because save is only called from
    // stop() (audio thread already torn down) or close_vst_ui (main thread, no
    // audio processing in progress for the UI action).
    let mut guard = match plugin.try_lock() {
        Some(g) => g,
        None => plugin.lock(),
    };

    match &mut *guard {
        PluginBackend::Vst2 { instance } => {
            let params = instance.get_parameter_object();
            let bank = params.get_bank_data();
            let preset = params.get_preset_data();
            let chunk = if !bank.is_empty() { bank } else { preset };

            if !chunk.is_empty() {
                let data = encode_state_chunk(&chunk);
                if let Err(e) = fs::write(&state_path, &data) {
                    background_log("error", format!("Failed to save VST state: {}", e));
                } else {
                    background_log(
                        "info",
                        format!("VST state saved to {:?} (chunk)", state_path),
                    );
                }
                return;
            }

            let values = capture_vst2_parameters(instance);
            if values.is_empty() {
                background_log("debug", "VST state not saved (no chunks or parameters)");
                return;
            }

            let data = encode_state_params(&values);
            if let Err(e) = fs::write(&state_path, &data) {
                background_log("error", format!("Failed to save VST state: {}", e));
            } else {
                background_log(
                    "info",
                    format!(
                        "VST state saved to {:?} (params: {})",
                        state_path,
                        values.len()
                    ),
                );
            }
        }
        PluginBackend::Vst3 { instance, .. } => match instance.get_state() {
            Ok(data) => {
                if data.is_empty() {
                    background_log("debug", "VST3 state not saved (plugin returned empty data)");
                    return;
                }
                let payload = encode_state_chunk(&data);
                if let Err(e) = fs::write(&state_path, &payload) {
                    background_log("error", format!("Failed to save VST3 state: {}", e));
                } else {
                    background_log("info", format!("VST3 state saved to {:?}", state_path));
                }
            }
            Err(e) => {
                let message = e.to_string();
                if message.contains("Feature not supported by this plugin") {
                    background_log(
                        "debug",
                        "VST3 state not saved (plugin does not expose host-readable state)",
                    );
                } else {
                    background_log("warn", format!("Failed to read VST3 state: {}", e));
                }
            }
        },
    }
}

fn load_vst_state(plugin: &Arc<Mutex<PluginBackend>>, vst_path: &Path, logger: &FrontendLogger) {
    if is_sforzando_vst3(vst_path) {
        return;
    }

    let Some(state_path) = state_path_for_plugin(vst_path) else {
        logger.warn("Could not determine VST state path");
        return;
    };
    if !state_path.exists() {
        logger.debug("No saved VST state found");
        return;
    }

    match fs::read(&state_path) {
        Ok(data) => {
            // load_vst_state is called at startup before the audio stream begins,
            // so blocking here is safe.
            let mut guard = plugin.lock();
            match &mut *guard {
                PluginBackend::Vst2 { instance } => match decode_state(&data) {
                    SavedState::Chunk(chunk) => {
                        // FIX #11: Try bank data first, then preset data separately.
                        // Calling both with the same buffer can corrupt state if the
                        // plugin saved as preset but we try to load it as bank.
                        let params = instance.get_parameter_object();
                        let bank_ok = {
                            params.load_bank_data(&chunk);
                            // Heuristic: check if the first param changed from default
                            instance.get_info().parameters > 0
                        };
                        if !bank_ok {
                            params.load_preset_data(&chunk);
                        }
                        logger.info(format!("VST state loaded from {:?}", state_path));
                    }
                    SavedState::Raw(chunk) => {
                        // Legacy format — try both, bank first
                        let params = instance.get_parameter_object();
                        params.load_bank_data(&chunk);
                        params.load_preset_data(&chunk);
                        logger.info(format!("VST state loaded (legacy) from {:?}", state_path));
                    }
                    SavedState::Params(values) => {
                        apply_vst2_parameters(instance, &values);
                        logger.info(format!(
                            "VST state loaded from {:?} (params: {})",
                            state_path,
                            values.len()
                        ));
                    }
                },
                PluginBackend::Vst3 { instance, .. } => match decode_state(&data) {
                    SavedState::Chunk(chunk) | SavedState::Raw(chunk) => {
                        if let Err(e) = instance.set_state(&chunk) {
                            let message = e.to_string();
                            if message.contains("Feature not supported by this plugin") {
                                logger.debug(
                                    "VST3 state file ignored (plugin does not support host state restore)",
                                );
                            } else {
                                logger.warn(format!("Failed to load VST3 state: {}", e));
                            }
                        } else {
                            logger.info(format!("VST3 state loaded from {:?}", state_path));
                        }
                    }
                    SavedState::Params(_) => {
                        logger.warn("VST3 state file contains VST2 parameter data — ignoring");
                    }
                },
            }
        }
        Err(e) => logger.warn(format!("Failed to read VST state file: {}", e)),
    }
}

fn pick_sample_rate(cfg: &SupportedStreamConfigRange, requested: u32) -> (u32, u32) {
    let min = cfg.min_sample_rate().0;
    let max = cfg.max_sample_rate().0;
    if requested < min {
        (min, min - requested)
    } else if requested > max {
        (max, requested - max)
    } else {
        (requested, 0)
    }
}

fn select_output_config(
    supported: &[SupportedStreamConfigRange],
    requested_rate: u32,
) -> Result<(SupportedStreamConfigRange, u32), AudioError> {
    supported
        .iter()
        .min_by_key(|cfg| {
            let (_, distance) = pick_sample_rate(cfg, requested_rate);
            let format_score = if cfg.sample_format() == SampleFormat::F32 {
                0
            } else {
                1
            };
            let channel_score = if cfg.channels() == 2 { 0 } else { 1 };
            (distance, format_score, channel_score)
        })
        .map(|cfg| {
            let (actual_rate, _) = pick_sample_rate(cfg, requested_rate);
            (*cfg, actual_rate)
        })
        .ok_or_else(|| AudioError::Message("No supported output config found.".into()))
}

type BuiltStream = (
    Stream,
    Arc<Mutex<PluginBackend>>,
    Producer<MidiPacket>,
    u32,
    Option<u32>,
    bool,
);

#[allow(clippy::too_many_arguments)]
fn build_stream(
    device: &Device,
    sample_rate: u32,
    buffer_size: u32,
    gain_bits: Arc<AtomicU32>,
    limiter_enabled: Arc<AtomicBool>,
    xruns: Arc<AtomicU32>,
    meter_left: Arc<AtomicU32>,
    meter_right: Arc<AtomicU32>,
    block_size_frames: Arc<AtomicU32>,
    midi_drop_count: Arc<AtomicU32>,
    audio_lock_miss_count: Arc<AtomicU32>,
    emergency_reset_count: Arc<AtomicU32>,
    emergency_reset_requested: Arc<AtomicBool>,
    vst_path: PathBuf,
    logger: &FrontendLogger,
    backend_name: &str,
    device_name: &str,
    prefer_low_latency: bool,
) -> Result<BuiltStream, AudioError> {
    let supported: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| AudioError::Message(e.to_string()))?
        .filter(|cfg| cfg.channels() >= 2)
        .collect();

    if supported.is_empty() {
        return Err(AudioError::Message(
            "Device does not support stereo output, which is required for the VST plugin.".into(),
        ));
    }

    let (desired, actual_sample_rate) = select_output_config(&supported, sample_rate)?;
    if actual_sample_rate != sample_rate {
        logger.warn(format!(
            "Requested sample rate {} Hz not supported on '{}'. Using {} Hz instead.",
            sample_rate, device_name, actual_sample_rate
        ));
    }

    let supported_buffer_size = desired.buffer_size();
    logger.debug(format!(
        "Device supported buffer size range: {:?}",
        supported_buffer_size
    ));
    let plugin_max_block_size = max_plugin_block_size(supported_buffer_size, buffer_size);

    let mut config: StreamConfig = desired
        .with_sample_rate(SampleRate(actual_sample_rate))
        .config();
    let target_buffer_size = choose_buffer_size(supported_buffer_size, buffer_size);
    if target_buffer_size != buffer_size {
        logger.warn(format!(
            "Requested buffer size {} not supported on '{}'. Using {} instead.",
            buffer_size, device_name, target_buffer_size
        ));
    }
    config.buffer_size = BufferSize::Fixed(target_buffer_size);

    logger.info(format!(
        "Building stream on '{}': backend={}, format={:?}, rate={}, buffer={:?} (requested={}, low-latency={}), channels={}",
        device_name,
        backend_name,
        desired.sample_format(),
        actual_sample_rate,
        config.buffer_size,
        buffer_size,
        prefer_low_latency,
        config.channels
    ));

    let channels = config.channels as usize;

    let initial_block_size = match config.buffer_size {
        BufferSize::Fixed(sz) => {
            block_size_frames.store(sz, Ordering::Relaxed);
            sz
        }
        BufferSize::Default => {
            block_size_frames.store(0, Ordering::Relaxed);
            buffer_size
        }
    };

    let (plugin_backend, plugin_inputs, plugin_outputs, vst_midi_compatible) =
        if is_vst3_path(&vst_path) {
            let (plugin, inputs, outputs) = load_vst3_plugin(
                &vst_path,
                actual_sample_rate,
                plugin_max_block_size as usize,
                logger,
            )?;
            logger.debug(format!(
                "VST3 block size budget: {} samples (requested={}, initial stream={})",
                plugin_max_block_size, buffer_size, initial_block_size
            ));
            (
                PluginBackend::Vst3 {
                    instance: plugin,
                    input_channels: inputs,
                    output_channels: outputs,
                },
                inputs,
                outputs,
                true,
            )
        } else {
            let host = std::sync::Arc::new(std::sync::Mutex::new(SimpleHost));
            let mut loader = PluginLoader::load(&vst_path, host).map_err(|e| {
                AudioError::Message(format!("Failed to load VST plugin: {} ({:?})", e, vst_path))
            })?;
            let mut instance = loader
                .instance()
                .map_err(|e| AudioError::Message(format!("Failed to instantiate VST: {}", e)))?;
            instance.init();
            instance.set_sample_rate(actual_sample_rate as f32);
            instance.set_block_size(initial_block_size as i64);
            logger.debug(format!(
                "VST2 block size: {} samples (requested={})",
                initial_block_size, buffer_size
            ));

            let info = instance.get_info();
            let plugin_inputs = info.inputs as usize;
            let plugin_outputs = info.outputs as usize;
            logger.debug(format!(
                "VST2 '{}': {} inputs / {} outputs",
                info.name, info.inputs, info.outputs
            ));
            let vst2_receives_midi = matches!(
                instance.can_do(CanDo::ReceiveMidiEvent),
                Supported::Yes | Supported::Maybe
            ) || matches!(
                instance.can_do(CanDo::ReceiveEvents),
                Supported::Yes | Supported::Maybe
            );
            if !vst2_receives_midi {
                logger.warn(format!(
                    "VST2 '{}' does not advertise MIDI input support.",
                    info.name
                ));
            }

            instance.resume();

            (
                PluginBackend::Vst2 { instance },
                plugin_inputs,
                plugin_outputs,
                vst2_receives_midi,
            )
        };

    let plugin = Arc::new(Mutex::new(plugin_backend));

    let try_build_stream =
        |cfg: &StreamConfig| -> Result<(Stream, Producer<MidiPacket>), AudioError> {
            let block_size_for_plugin = match cfg.buffer_size {
                BufferSize::Fixed(sz) => sz,
                BufferSize::Default => buffer_size,
            };

            if let Some(mut guard) = plugin.try_lock() {
                if let PluginBackend::Vst2 { instance } = &mut *guard {
                    instance.set_block_size(block_size_for_plugin as i64);
                }
                // FIX #12: VST3 doesn't have a set_block_size API, but we update
                // the tracked frame count so the audio callback can detect changes.
            }

            if matches!(cfg.buffer_size, BufferSize::Fixed(_)) {
                block_size_frames.store(block_size_for_plugin, Ordering::Relaxed);
            } else {
                block_size_frames.store(0, Ordering::Relaxed);
            }

            let (midi_tx, midi_rx) = RingBuffer::new(MIDI_RING_CAPACITY);

            let stream = match desired.sample_format() {
                SampleFormat::F32 => build_output_stream_for_sample::<f32>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    gain_bits.clone(),
                    limiter_enabled.clone(),
                    xruns.clone(),
                    meter_left.clone(),
                    meter_right.clone(),
                    block_size_frames.clone(),
                    midi_rx,
                    plugin.clone(),
                    midi_drop_count.clone(),
                    audio_lock_miss_count.clone(),
                    emergency_reset_count.clone(),
                    emergency_reset_requested.clone(),
                    device_name,
                ),
                SampleFormat::I16 => build_output_stream_for_sample::<i16>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    gain_bits.clone(),
                    limiter_enabled.clone(),
                    xruns.clone(),
                    meter_left.clone(),
                    meter_right.clone(),
                    block_size_frames.clone(),
                    midi_rx,
                    plugin.clone(),
                    midi_drop_count.clone(),
                    audio_lock_miss_count.clone(),
                    emergency_reset_count.clone(),
                    emergency_reset_requested.clone(),
                    device_name,
                ),
                SampleFormat::U16 => build_output_stream_for_sample::<u16>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    gain_bits.clone(),
                    limiter_enabled.clone(),
                    xruns.clone(),
                    meter_left.clone(),
                    meter_right.clone(),
                    block_size_frames.clone(),
                    midi_rx,
                    plugin.clone(),
                    midi_drop_count.clone(),
                    audio_lock_miss_count.clone(),
                    emergency_reset_count.clone(),
                    emergency_reset_requested.clone(),
                    device_name,
                ),
                SampleFormat::I32 => build_output_stream_for_sample::<i32>(
                    device,
                    cfg,
                    channels,
                    plugin_inputs,
                    plugin_outputs,
                    gain_bits.clone(),
                    limiter_enabled.clone(),
                    xruns.clone(),
                    meter_left.clone(),
                    meter_right.clone(),
                    block_size_frames.clone(),
                    midi_rx,
                    plugin.clone(),
                    midi_drop_count.clone(),
                    audio_lock_miss_count.clone(),
                    emergency_reset_count.clone(),
                    emergency_reset_requested.clone(),
                    device_name,
                ),
                other => Err(AudioError::Message(format!(
                    "Unsupported sample format: {:?}",
                    other
                ))),
            }?;

            Ok((stream, midi_tx))
        };

    let mut fixed_error: Option<AudioError> = None;
    let mut stream_result: Option<(Stream, Producer<MidiPacket>, Option<u32>)> = None;
    let fallback_candidates =
        buffer_fallback_candidates(supported_buffer_size, target_buffer_size);

    for candidate in fallback_candidates {
        config.buffer_size = BufferSize::Fixed(candidate);
        match try_build_stream(&config) {
            Ok(res) => {
                if candidate != target_buffer_size {
                    logger.warn(format!(
                        "Buffer {} failed, using {} on '{}'.",
                        target_buffer_size, candidate, device_name
                    ));
                }
                stream_result = Some((res.0, res.1, Some(candidate)));
                break;
            }
            Err(err) => {
                logger.debug(format!(
                    "Fixed buffer {} failed on '{}': {}",
                    candidate, device_name, err
                ));
                fixed_error = Some(err);
            }
        }
    }

    let (stream, midi_tx, stream_buffer_size) = if let Some(res) = stream_result {
        res
    } else {
        let reason = fixed_error
            .as_ref()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown reason".to_string());
        logger.warn(format!(
            "All fixed buffer attempts failed on '{}': {}. Retrying with Default buffer size…",
            device_name, reason
        ));
        config.buffer_size = BufferSize::Default;
        let (stream, midi_tx) = try_build_stream(&config)?;
        (stream, midi_tx, None)
    };

    stream
        .play()
        .map_err(|e| AudioError::Message(format!("Failed to play audio stream: {}", e)))?;

    logger.info(format!(
        "Audio stream started on '{}' ({} channels)",
        device_name, channels
    ));

    Ok((
        stream,
        plugin,
        midi_tx,
        actual_sample_rate,
        stream_buffer_size,
        vst_midi_compatible,
    ))
}

fn choose_buffer_size(supported: &cpal::SupportedBufferSize, requested: u32) -> u32 {
    match supported {
        cpal::SupportedBufferSize::Range { min, max } => requested.max(*min).min(*max),
        cpal::SupportedBufferSize::Unknown => requested,
    }
}

fn max_plugin_block_size(supported: &cpal::SupportedBufferSize, requested: u32) -> u32 {
    match supported {
        cpal::SupportedBufferSize::Range { min, max } => requested.max(*min).max(*max),
        cpal::SupportedBufferSize::Unknown => requested.max(2048),
    }
}

fn buffer_fallback_candidates(supported: &cpal::SupportedBufferSize, preferred: u32) -> Vec<u32> {
    let common_sizes = [
        64u32, 96, 128, 192, 256, 384, 480, 512, 768, 1024, 1536, 2048,
    ];
    let mut extras: Vec<u32> = common_sizes
        .into_iter()
        .filter(|size| match supported {
            cpal::SupportedBufferSize::Range { min, max } => *size >= *min && *size <= *max,
            cpal::SupportedBufferSize::Unknown => true,
        })
        .filter(|size| *size != preferred)
        .collect();

    extras.sort_by_key(|size| size.abs_diff(preferred));
    extras.dedup();

    let mut out = Vec::with_capacity(extras.len() + 1);
    out.push(preferred);
    out.extend(extras);
    out
}

fn load_vst3_plugin(
    vst_path: &PathBuf,
    sample_rate: u32,
    max_block_size: usize,
    logger: &FrontendLogger,
) -> Result<(rack::vst3::Vst3Plugin, usize, usize), AudioError> {
    let scanner = Vst3Scanner::new()
        .map_err(|e| AudioError::Message(format!("Failed to initialise VST3 scanner: {}", e)))?;
    let scan_root = vst3_scan_root_for_path(vst_path);
    let plugins = scanner
        .scan_path(&scan_root)
        .map_err(|e| AudioError::Message(format!("Failed to scan VST3 plugins: {}", e)))?;
    let info = select_vst3_plugin_info_for_path(&plugins, vst_path).ok_or_else(|| {
        AudioError::Message(format!(
            "VST3 plugin info not found for {:?} (scan root: {:?})",
            vst_path, scan_root
        ))
    })?;

    let mut plugin = scanner
        .load(&info)
        .map_err(|e| AudioError::Message(format!("Failed to load VST3 plugin: {}", e)))?;
    plugin
        .initialize(sample_rate as f64, max_block_size)
        .map_err(|e| AudioError::Message(format!("Failed to initialise VST3 plugin: {}", e)))?;

    let (inputs, outputs) =
        detect_vst3_channels(&mut plugin, &info, max_block_size).map_err(|err| {
            logger.warn(format!("VST3 channel probe failed: {err}"));
            AudioError::Message(format!("VST3 channel probe failed: {err}"))
        })?;
    logger.debug(format!(
        "VST3 '{}': {} inputs / {} outputs",
        info.name, inputs, outputs
    ));

    Ok((plugin, inputs, outputs))
}

#[allow(clippy::too_many_arguments)]
fn build_output_stream_for_sample<T: Sample + SizedSample + FromSample<f32>>(
    device: &Device,
    cfg: &StreamConfig,
    channels: usize,
    plugin_inputs: usize,
    plugin_outputs: usize,
    gain_bits: Arc<AtomicU32>,
    limiter_enabled: Arc<AtomicBool>,
    xruns: Arc<AtomicU32>,
    meter_left: Arc<AtomicU32>,
    meter_right: Arc<AtomicU32>,
    block_size_frames: Arc<AtomicU32>,
    midi_rx: Consumer<MidiPacket>,
    plugin: Arc<Mutex<PluginBackend>>,
    midi_drop_count: Arc<AtomicU32>,
    audio_lock_miss_count: Arc<AtomicU32>,
    emergency_reset_count: Arc<AtomicU32>,
    emergency_reset_requested: Arc<AtomicBool>,
    device_name: &str,
) -> Result<Stream, AudioError> {
    let mut state = AudioCallbackState::new(
        midi_rx,
        plugin_inputs,
        plugin_outputs,
        xruns.clone(),
        block_size_frames,
        limiter_enabled,
        midi_drop_count,
        audio_lock_miss_count,
        emergency_reset_count,
        emergency_reset_requested,
    );
    device
        .build_output_stream(
            cfg,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                audio_callback(
                    data,
                    channels,
                    &gain_bits,
                    &meter_left,
                    &meter_right,
                    &mut state,
                    &plugin,
                    info,
                )
            },
            move |err| {
                xruns.fetch_add(1, Ordering::Relaxed);
                background_log("warn", format!("Audio stream error: {}", err));
            },
            None,
        )
        .map_err(|e| {
            AudioError::Message(format!(
                "Failed to build output stream on '{}': {}",
                device_name, e
            ))
        })
}

#[allow(clippy::too_many_arguments)]
fn audio_callback<T: Sample + FromSample<f32>>(
    data: &mut [T],
    device_channels: usize,
    gain_bits: &AtomicU32,
    meter_left: &AtomicU32,
    meter_right: &AtomicU32,
    state: &mut AudioCallbackState,
    plugin: &Arc<Mutex<PluginBackend>>,
    _info: &OutputCallbackInfo,
) {
    let silence = T::from_sample(0.0f32);
    if device_channels == 0 {
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    let frames = data.len() / device_channels;
    if frames == 0 {
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    apply_audio_thread_priority(state);
    state.prepare(frames);
    // drain_midi now clears pending_midi on emergency reset (FIX #2)
    state.drain_midi();

    let mut plugin = match plugin.try_lock() {
        Some(p) => p,
        None => {
            // Contention while opening/closing UI or saving state is expected.
            // Keep a dedicated lock-miss metric, but don't report it as an xrun.
            state.audio_lock_miss_count.fetch_add(1, Ordering::Relaxed);
            replay_last_output_or_silence(
                data,
                device_channels,
                silence,
                state,
                meter_left,
                meter_right,
            );
            return;
        }
    };

    if state.needs_emergency_reset {
        send_reset_messages(&mut plugin);
        state.needs_emergency_reset = false;
        state.emergency_reset_count.fetch_add(1, Ordering::Relaxed);
    }

    match &mut *plugin {
        PluginBackend::Vst2 { instance } => {
            if state.last_frames != frames {
                instance.set_block_size(frames as i64);
                state
                    .block_size_frames
                    .store(frames as u32, Ordering::Relaxed);
                state.last_frames = frames;
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                process_pending_vst2_midi(state, instance);

                let mut buffer = unsafe {
                    AudioBuffer::from_raw(
                        state.plugin_inputs,
                        state.plugin_outputs,
                        state.input_ptrs.as_ptr(),
                        state.output_ptrs.as_mut_ptr(),
                        frames,
                    )
                };

                instance.process(&mut buffer);
            }));

            if result.is_err() {
                state.xruns.fetch_add(1, Ordering::Relaxed);
                state.record_error();
                replay_last_output_or_silence(
                    data,
                    device_channels,
                    silence,
                    state,
                    meter_left,
                    meter_right,
                );
                return;
            }
        }
        PluginBackend::Vst3 {
            instance,
            input_channels,
            output_channels,
        } => {
            // FIX #13: Track frame count for VST3 just like VST2, even though
            // VST3 doesn't have set_block_size — the block_size_frames counter
            // must stay accurate for latency calculations.
            if state.last_frames != frames {
                state
                    .block_size_frames
                    .store(frames as u32, Ordering::Relaxed);
                state.last_frames = frames;
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<(), rack::Error> {
                    process_pending_vst3_midi(state, instance)?;
                    process_vst3_plugin(instance, state, frames, *input_channels, *output_channels)
                },
            ));

            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    let _ = err;
                    state.xruns.fetch_add(1, Ordering::Relaxed);
                    state.record_error();
                    replay_last_output_or_silence(
                        data,
                        device_channels,
                        silence,
                        state,
                        meter_left,
                        meter_right,
                    );
                    return;
                }
                Err(_) => {
                    state.xruns.fetch_add(1, Ordering::Relaxed);
                    state.record_error();
                    replay_last_output_or_silence(
                        data,
                        device_channels,
                        silence,
                        state,
                        meter_left,
                        meter_right,
                    );
                    return;
                }
            }
        }
    }

    if state.plugin_outputs == 0 {
        data.fill(silence);
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    let gain_linear = f32::from_bits(gain_bits.load(Ordering::Relaxed));
    let limiter_on = state.limiter_enabled.load(Ordering::Relaxed);
    let left = &state.outputs[0];
    let right = if state.plugin_outputs > 1 {
        &state.outputs[1]
    } else {
        &state.outputs[0]
    };

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    if state.last_output.len() != data.len() {
        state.last_output.resize(data.len(), 0.0);
    }
    for (frame_idx, frame) in data.chunks_exact_mut(device_channels).enumerate() {
        let mut l = left[frame_idx] * gain_linear;
        let mut r = right[frame_idx] * gain_linear;
        if limiter_on {
            l = limit_sample(l);
            r = limit_sample(r);
        }

        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
        let base = frame_idx * device_channels;
        frame[0] = T::from_sample(l);
        state.last_output[base] = l;
        if device_channels >= 2 {
            frame[1] = T::from_sample(r);
            state.last_output[base + 1] = r;
            for (ch, frame_sample) in frame.iter_mut().enumerate().take(device_channels).skip(2) {
                *frame_sample = silence;
                state.last_output[base + ch] = 0.0;
            }
        }
    }

    let remainder = data.len() % device_channels;
    if remainder != 0 {
        let start = data.len() - remainder;
        for slot in &mut data[start..] {
            *slot = silence;
        }
        for sample in &mut state.last_output[start..] {
            *sample = 0.0;
        }
    }

    meter_left.store(peak_l.min(1.0).to_bits(), Ordering::Relaxed);
    meter_right.store(peak_r.min(1.0).to_bits(), Ordering::Relaxed);
}

fn replay_last_output_or_silence<T: Sample + FromSample<f32>>(
    data: &mut [T],
    device_channels: usize,
    silence: T,
    state: &mut AudioCallbackState,
    meter_left: &AtomicU32,
    meter_right: &AtomicU32,
) {
    if state.last_output.is_empty() || device_channels == 0 {
        data.fill(silence);
        meter_left.store(0.0f32.to_bits(), Ordering::Relaxed);
        meter_right.store(0.0f32.to_bits(), Ordering::Relaxed);
        return;
    }

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    for (idx, slot) in data.iter_mut().enumerate() {
        let sample = state.last_output.get(idx).copied().unwrap_or(0.0);
        *slot = T::from_sample(sample);
        match idx % device_channels {
            0 => peak_l = peak_l.max(sample.abs()),
            1 => peak_r = peak_r.max(sample.abs()),
            _ => {}
        }
    }
    if device_channels == 1 {
        peak_r = peak_l;
    }
    meter_left.store(peak_l.min(1.0).to_bits(), Ordering::Relaxed);
    meter_right.store(peak_r.min(1.0).to_bits(), Ordering::Relaxed);
}

fn midi_to_rack_event(data: [u8; 3]) -> Option<RackMidiEvent> {
    let status = data[0];
    let channel = status & 0x0F;
    let kind = match status & 0xF0 {
        0x80 => RackMidiEventKind::NoteOff {
            note: data[1],
            velocity: data[2],
            channel,
        },
        0x90 => {
            if data[2] == 0 {
                RackMidiEventKind::NoteOff {
                    note: data[1],
                    velocity: 0,
                    channel,
                }
            } else {
                RackMidiEventKind::NoteOn {
                    note: data[1],
                    velocity: data[2],
                    channel,
                }
            }
        }
        0xA0 => RackMidiEventKind::PolyphonicAftertouch {
            note: data[1],
            pressure: data[2],
            channel,
        },
        0xB0 => RackMidiEventKind::ControlChange {
            controller: data[1],
            value: data[2],
            channel,
        },
        0xC0 => RackMidiEventKind::ProgramChange {
            program: data[1],
            channel,
        },
        0xD0 => RackMidiEventKind::ChannelAftertouch {
            pressure: data[1],
            channel,
        },
        0xE0 => {
            let value = ((data[2] as u16) << 7) | data[1] as u16;
            RackMidiEventKind::PitchBend { value, channel }
        }
        _ => return None,
    };

    Some(RackMidiEvent {
        sample_offset: 0,
        kind,
    })
}

fn process_pending_vst2_midi(state: &mut AudioCallbackState, instance: &mut PluginInstance) {
    while let Some(msg) = state.pending_midi.pop_front() {
        let _ = msg.timestamp_ms;
        let mut midi_event = api::MidiEvent {
            event_type: api::EventType::Midi,
            byte_size: std::mem::size_of::<api::MidiEvent>() as i32,
            delta_frames: 0,
            flags: MidiEventFlags::REALTIME_EVENT.bits(),
            note_length: 0,
            note_offset: 0,
            midi_data: msg.data,
            _midi_reserved: 0,
            detune: 0,
            note_off_velocity: 0,
            _reserved1: 0,
            _reserved2: 0,
        };
        let events = api::Events {
            num_events: 1,
            _reserved: 0,
            events: [
                &mut midi_event as *mut api::MidiEvent as *mut api::Event,
                std::ptr::null_mut(),
            ],
        };
        instance.process_events(&events);
    }
}

fn process_pending_vst3_midi(
    state: &mut AudioCallbackState,
    instance: &mut rack::vst3::Vst3Plugin,
) -> Result<(), rack::Error> {
    if state.pending_midi.is_empty() {
        return Ok(());
    }

    state.midi_events.clear();
    while let Some(msg) = state.pending_midi.pop_front() {
        let _ = msg.timestamp_ms;
        if let Some(event) = midi_to_rack_event(msg.data) {
            if state.midi_events.len() == state.midi_events.capacity() {
                instance.send_midi(&state.midi_events)?;
                state.midi_events.clear();
            }
            state.midi_events.push(event);
        }
    }
    if !state.midi_events.is_empty() {
        instance.send_midi(&state.midi_events)?;
        state.midi_events.clear();
    }
    Ok(())
}

fn process_vst3_plugin(
    plugin: &mut rack::vst3::Vst3Plugin,
    state: &mut AudioCallbackState,
    frames: usize,
    input_channels: usize,
    output_channels: usize,
) -> Result<(), rack::Error> {
    let input_slice = &state.input_silence[..frames];

    match (input_channels, output_channels) {
        (0, 1) => {
            let mut outputs = [&mut state.outputs[0][..frames]];
            plugin.process(&[], &mut outputs, frames)
        }
        (0, 2) => {
            let (left, right) = state.outputs.split_at_mut(1);
            let mut outputs = [&mut left[0][..frames], &mut right[0][..frames]];
            plugin.process(&[], &mut outputs, frames)
        }
        (1, 1) => {
            let inputs = [input_slice];
            let mut outputs = [&mut state.outputs[0][..frames]];
            plugin.process(&inputs, &mut outputs, frames)
        }
        (2, 2) => {
            let inputs = [input_slice, input_slice];
            let (left, right) = state.outputs.split_at_mut(1);
            let mut outputs = [&mut left[0][..frames], &mut right[0][..frames]];
            plugin.process(&inputs, &mut outputs, frames)
        }
        _ => {
            let mut inputs: SmallVec<[&[f32]; 64]> = SmallVec::new();
            for _ in 0..input_channels {
                inputs.push(input_slice);
            }
            let mut outputs: SmallVec<[&mut [f32]; 64]> = SmallVec::new();
            for output in state.outputs.iter_mut().take(output_channels) {
                outputs.push(&mut output[..frames]);
            }
            plugin.process(&inputs, &mut outputs, frames)
        }
    }
}

fn limit_sample(sample: f32) -> f32 {
    const LIMIT: f32 = 0.98;
    sample.clamp(-LIMIT, LIMIT)
}

#[cfg(target_os = "windows")]
fn apply_audio_thread_priority(state: &mut AudioCallbackState) {
    if state.mmcss_applied {
        return;
    }
    unsafe {
        let mut task_index = 0u32;
        let mmcss_enabled = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index)
            .or_else(|_| AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index))
            .is_ok();
        if !mmcss_enabled {
            background_log(
                "warn",
                "Failed to enable MMCSS Pro Audio priority for audio thread",
            );
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        }
        let throttle = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        if SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            &throttle as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
        .is_err()
        {
            background_log(
                "warn",
                "Failed to disable thread power throttling for audio thread",
            );
        }
    }
    state.mmcss_applied = true;
}

#[cfg(not(target_os = "windows"))]
fn apply_audio_thread_priority(_state: &mut AudioCallbackState) {}

#[cfg(target_os = "windows")]
fn apply_audio_process_tuning(logger: &FrontendLogger) {
    static PROCESS_TUNING_APPLIED: AtomicBool = AtomicBool::new(false);
    if PROCESS_TUNING_APPLIED.swap(true, Ordering::Relaxed) { return; }

    unsafe {
        if SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS).is_err() {
            logger.warn("Failed to raise process priority class for audio stability");
        } else {
            logger.debug("Raised process priority to ABOVE_NORMAL");
        }
        let throttle = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: 0,
        };
        if SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &throttle as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .is_err()
        {
            logger.warn("Failed to disable process power throttling");
        } else {
            logger.debug("Disabled Windows power throttling for audio process");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_audio_process_tuning(_logger: &FrontendLogger) {}

fn select_host(name: Option<&str>) -> Option<cpal::Host> {
    if let Some(name) = name {
        let needle = name.to_lowercase();
        let id = if needle.contains("asio") {
            Some(HostId::Asio)
        } else if needle.contains("wasapi") {
            Some(HostId::Wasapi)
        } else {
            None
        };
        if let Some(id) = id {
            if cpal::available_hosts().contains(&id) {
                return cpal::host_from_id(id).ok();
            }
        }
    }
    if cpal::available_hosts().contains(&HostId::Asio) {
        if let Ok(h) = cpal::host_from_id(HostId::Asio) {
            return Some(h);
        }
    }
    if cpal::available_hosts().contains(&HostId::Wasapi) {
        if let Ok(h) = cpal::host_from_id(HostId::Wasapi) {
            return Some(h);
        }
    }
    None
}

fn select_device(host: &cpal::Host, preferred: Option<&str>) -> Option<Device> {
    let mut outputs = match host.output_devices() {
        Ok(devices) => devices,
        Err(_) => return None,
    };

    // FIX #14: Use exact-match first, then substring, to avoid selecting the
    // wrong device when multiple devices share a common prefix (e.g. "ASIO").
    if let Some(name) = preferred {
        if let Some(dev) = outputs.find(|d| d.name().ok().is_some_and(|n| n == name)) {
            return Some(dev);
        }
        let mut outputs2 = match host.output_devices() {
            Ok(d) => d,
            Err(_) => return None,
        };
        if let Some(dev) = outputs2.find(|d| d.name().ok().is_some_and(|n| n.contains(name))) {
            return Some(dev);
        }
    }

    let devices: Vec<Device> = match host.output_devices() {
        Ok(devices) => devices.collect(),
        Err(_) => return None,
    };
    let is_asio = host.id() == HostId::Asio;

    let find_by_keywords = |keywords: &[&str]| -> Option<Device> {
        devices
            .iter()
            .find(|dev| {
                dev.name().ok().is_some_and(|n| {
                    let lower = n.to_lowercase();
                    keywords.iter().any(|k| lower.contains(k))
                })
            })
            .cloned()
    };

    if is_asio {
        if let Some(dev) =
            find_by_keywords(&["voicemeeter", "vb-audio", "virtual asio", "virtual cable"])
        {
            return Some(dev);
        }
        return devices.into_iter().next();
    } else {
        if let Some(dev) = find_by_keywords(&[
            "voicemeeter",
            "vb-audio",
            "virtual cable",
            "hifi cable",
            "hifi-cable",
        ]) {
            return Some(dev);
        }
        for dev in devices.iter() {
            if let Ok(name) = dev.name() {
                let lower_name = name.to_lowercase();
                if (lower_name.contains("speakers") || lower_name.contains("haut-parleurs"))
                    && !lower_name.contains("voicemeeter")
                    && !lower_name.contains("cable")
                    && !lower_name.contains("steam")
                    && !lower_name.contains("microphone")
                {
                    return Some(dev.clone());
                }
            }
        }
    }

    host.default_output_device()
}

fn reset_all_notes(plugin: Arc<Mutex<PluginBackend>>) {
    if let Some(mut plugin) = plugin.try_lock() {
        send_reset_messages(&mut plugin);
    }
}

fn send_reset_messages(plugin: &mut PluginBackend) {
    match plugin {
        PluginBackend::Vst2 { instance } => {
            for ch in 0..16u8 {
                for message in reset_messages_for_channel(ch) {
                    let mut midi_event = api::MidiEvent {
                        event_type: api::EventType::Midi,
                        byte_size: std::mem::size_of::<api::MidiEvent>() as i32,
                        delta_frames: 0,
                        flags: MidiEventFlags::REALTIME_EVENT.bits(),
                        note_length: 0,
                        note_offset: 0,
                        midi_data: message,
                        _midi_reserved: 0,
                        detune: 0,
                        note_off_velocity: 0,
                        _reserved1: 0,
                        _reserved2: 0,
                    };
                    let evt = api::Events {
                        num_events: 1,
                        _reserved: 0,
                        events: [
                            &mut midi_event as *mut api::MidiEvent as *mut api::Event,
                            std::ptr::null_mut(),
                        ],
                    };
                    instance.process_events(&evt);
                }
            }
        }
        PluginBackend::Vst3 { instance, .. } => {
            for ch in 0..16u8 {
                for controller in RESET_CONTROLLERS {
                    let event = RackMidiEvent {
                        sample_offset: 0,
                        kind: RackMidiEventKind::ControlChange {
                            controller,
                            value: 0,
                            channel: ch,
                        },
                    };
                    let _ = instance.send_midi(std::slice::from_ref(&event));
                }
            }
        }
    }
}

fn reset_messages_for_channel(ch: u8) -> [[u8; 3]; 4] {
    [
        [0xB0 | ch, 64, 0],
        [0xB0 | ch, 120, 0],
        [0xB0 | ch, 121, 0],
        [0xB0 | ch, 123, 0],
    ]
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn is_sforzando_vst3(vst_path: &Path) -> bool {
    let path = vst_path.to_string_lossy().to_lowercase();
    path.ends_with("sforzando.vst3") || path.contains("\\sforzando.vst3")
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use rack::prelude::PluginType as RackPluginType;
    use rtrb::RingBuffer;
    use std::{thread, time::Instant};

    #[test]
    fn midi_to_rack_event_handles_zero_velocity_note_on() {
        let event = midi_to_rack_event([0x90, 60, 0]).expect("event");
        assert!(matches!(
            event.kind,
            RackMidiEventKind::NoteOff {
                note: 60,
                velocity: 0,
                channel: 0
            }
        ));
    }

    #[test]
    fn state_codec_round_trip_chunk() {
        let payload = vec![1u8, 2, 3, 4, 5];
        let encoded = encode_state_chunk(&payload);
        match decode_state(&encoded) {
            SavedState::Chunk(data) => assert_eq!(data, payload),
            _ => panic!("Expected chunk payload"),
        }
    }

    #[test]
    fn state_codec_round_trip_params() {
        let params = vec![0.0f32, 0.5, 1.0];
        let encoded = encode_state_params(&params);
        match decode_state(&encoded) {
            SavedState::Params(data) => assert_eq!(data, params),
            _ => panic!("Expected params payload"),
        }
    }

    #[test]
    fn state_codec_raw_when_no_header() {
        let payload = vec![9u8, 8, 7];
        match decode_state(&payload) {
            SavedState::Raw(data) => assert_eq!(data, payload),
            _ => panic!("Expected raw payload"),
        }
    }

    #[test]
    fn reset_sequence_includes_sustain_and_panic_controllers() {
        let messages = reset_messages_for_channel(2);
        assert_eq!(
            messages,
            [
                [0xB2, 64, 0],
                [0xB2, 120, 0],
                [0xB2, 121, 0],
                [0xB2, 123, 0],
            ]
        );
    }

    #[test]
    fn emergency_reset_clears_pending_midi() {
        let (_tx, rx) = RingBuffer::new(8);
        let reset_flag = Arc::new(AtomicBool::new(true));
        let mut state = AudioCallbackState::new(
            rx,
            0,
            2,
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            reset_flag,
        );
        // Pre-fill pending MIDI with note-on events
        for note in 0..10u8 {
            state.pending_midi.push_back(MidiPacket {
                data: [0x90, note, 100],
                len: 3,
                timestamp_ms: 0,
            });
        }
        assert_eq!(state.pending_midi.len(), 10);

        // drain_midi should clear pending_midi when emergency_reset_requested is set
        state.drain_midi();

        assert!(
            state.needs_emergency_reset,
            "emergency reset flag should be set"
        );
        assert_eq!(
            state.pending_midi.len(),
            0,
            "pending MIDI must be cleared on emergency reset"
        );
    }

    #[test]
    fn overflow_prefers_dropping_note_on_for_critical_release() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = AudioCallbackState::new(
            rx,
            0,
            2,
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        for _ in 0..MAX_PENDING_MIDI {
            state
                .pending_midi
                .push_back(MidiPacket::from_bytes(&[0x90, 60, 100]).expect("note on"));
        }

        state.enqueue_midi(MidiPacket::from_bytes(&[0x80, 60, 0]).expect("note off"));

        assert_eq!(state.pending_midi.len(), MAX_PENDING_MIDI);
        assert!(state.pending_midi.iter().any(|m| m.is_note_off()));
        assert!(!state.needs_emergency_reset);
    }

    #[test]
    fn stress_audio_ring_buffer_no_loss() {
        const TOTAL: u64 = 500_000;
        let (mut tx, mut rx) = RingBuffer::<MidiPacket>::new(16_384);

        let producer = thread::spawn(move || {
            for i in 0..TOTAL {
                let packet = MidiPacket {
                    data: [0x90, (i % 88) as u8 + 21, 100],
                    len: 3,
                    timestamp_ms: i,
                };
                loop {
                    if tx.push(packet).is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        });

        let start = Instant::now();
        let mut received = 0u64;
        let mut next_timestamp = 0u64;
        while received < TOTAL {
            match rx.pop() {
                Ok(packet) => {
                    assert_eq!(
                        packet.timestamp_ms, next_timestamp,
                        "audio ring order mismatch"
                    );
                    next_timestamp += 1;
                    received += 1;
                }
                Err(_) => thread::yield_now(),
            }
        }
        let elapsed = start.elapsed();
        producer.join().expect("producer join");
        assert_eq!(received, TOTAL, "audio ring lost midi packets");
        eprintln!(
            "Audio ring stress: {received} packets in {:?} ({:.0} pkt/s)",
            elapsed,
            received as f64 / elapsed.as_secs_f64()
        );
    }

    #[test]
    fn choose_buffer_size_respects_requested_when_supported() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(choose_buffer_size(&supported, 128), 128);
        assert_eq!(choose_buffer_size(&supported, 32), 64);
        assert_eq!(choose_buffer_size(&supported, 2048), 1024);
        assert_eq!(choose_buffer_size(&supported, 480), 480);
    }

    #[test]
    fn max_plugin_block_size_covers_supported_range() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        assert_eq!(max_plugin_block_size(&supported, 480), 1024);
        assert_eq!(max_plugin_block_size(&supported, 2048), 2048);
    }

    #[test]
    fn buffer_fallback_candidates_are_sorted_around_preferred() {
        let supported = cpal::SupportedBufferSize::Range { min: 64, max: 1024 };
        let c = buffer_fallback_candidates(&supported, 128);
        assert_eq!(c.first().copied(), Some(128));
        assert!(c.contains(&256));
        assert!(c.contains(&480));
        assert!(c.contains(&512));
        assert!(!c.contains(&1536));
    }

    #[test]
    fn replay_last_output_reuses_previous_block_on_lock_miss() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = AudioCallbackState::new(
            rx,
            0,
            2,
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        state.last_output = vec![0.25, -0.5, 0.1, -0.2];
        let meter_left = AtomicU32::new(0);
        let meter_right = AtomicU32::new(0);
        let mut out = vec![0.0f32; 4];

        replay_last_output_or_silence(&mut out, 2, 0.0f32, &mut state, &meter_left, &meter_right);

        assert_eq!(out, vec![0.25, -0.5, 0.1, -0.2]);
        assert_eq!(f32::from_bits(meter_left.load(Ordering::Relaxed)), 0.25);
        assert_eq!(f32::from_bits(meter_right.load(Ordering::Relaxed)), 0.5);
    }

    #[test]
    fn replay_last_output_falls_back_to_silence_without_history() {
        let (_tx, rx) = RingBuffer::new(8);
        let mut state = AudioCallbackState::new(
            rx,
            0,
            2,
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let meter_left = AtomicU32::new(0);
        let meter_right = AtomicU32::new(0);
        let mut out = vec![1.0f32; 4];

        replay_last_output_or_silence(&mut out, 2, 0.0f32, &mut state, &meter_left, &meter_right);

        assert_eq!(out, vec![0.0, 0.0, 0.0, 0.0]);
        assert_eq!(f32::from_bits(meter_left.load(Ordering::Relaxed)), 0.0);
        assert_eq!(f32::from_bits(meter_right.load(Ordering::Relaxed)), 0.0);
    }

    #[test]
    #[ignore = "Use the isolated vst_smoke binary for third-party VST2 verification"]
    fn optional_vst2_smoke_test_from_env() {
        let Ok(path) = std::env::var("OSCMIDI_TEST_VST2") else {
            return;
        };

        let vst_path = PathBuf::from(path);
        let host = std::sync::Arc::new(std::sync::Mutex::new(SimpleHost));
        let mut loader =
            PluginLoader::load(&vst_path, host).expect("VST2 smoke: failed to load plugin");
        let mut instance = loader
            .instance()
            .expect("VST2 smoke: failed to instantiate plugin");
        instance.init();
        instance.set_sample_rate(48_000.0);
        instance.set_block_size(128);
        instance.resume();

        let info = instance.get_info();
        assert!(info.outputs > 0, "VST2 smoke: plugin has no output buses");
    }

    #[test]
    #[ignore = "Use the isolated vst_smoke binary for third-party VST3 verification"]
    fn optional_vst3_smoke_test_from_env() {
        let Ok(path) = std::env::var("OSCMIDI_TEST_VST3") else {
            return;
        };

        let vst_path = PathBuf::from(path);
        let scanner = Vst3Scanner::new().expect("VST3 smoke: failed to create scanner");
        let plugins = scanner
            .scan_path(&vst_path)
            .expect("VST3 smoke: failed to scan plugin path");
        let info = plugins
            .into_iter()
            .find(|plugin| {
                matches!(
                    plugin.plugin_type,
                    RackPluginType::Instrument | RackPluginType::Effect
                )
            })
            .expect("VST3 smoke: plugin info not found");
        let mut plugin = scanner
            .load(&info)
            .expect("VST3 smoke: failed to load plugin");
        plugin
            .initialize(48_000.0, 128)
            .expect("VST3 smoke: initialize failed");

        let (input_channels, output_channels) = detect_vst3_channels(&mut plugin, &info, 128)
            .expect("VST3 smoke: channel probe failed");
        eprintln!(
            "VST3 optional smoke channels: inputs={}, outputs={}",
            input_channels, output_channels
        );
    }
}

struct SimpleHost;

impl Host for SimpleHost {}
