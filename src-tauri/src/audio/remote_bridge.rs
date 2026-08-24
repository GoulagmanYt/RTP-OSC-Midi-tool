#![allow(deprecated)]
#![cfg_attr(target_pointer_width = "32", allow(dead_code, unused_imports))]

use std::{
    cell::Cell,
    collections::VecDeque,
    mem::size_of,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use smallvec::SmallVec;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, RECT, WPARAM,
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Memory::{
                CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile,
                FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
            },
            Threading::{
                CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
                SYNCHRONIZATION_SYNCHRONIZE,
            },
        },
        UI::{
            HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
                PeekMessageW, RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow,
                TranslateMessage, CW_USEDEFAULT, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE,
                SWP_NOZORDER, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
                WM_DESTROY, WM_DPICHANGED, WNDCLASSW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED,
                WS_SYSMENU,
            },
        },
    },
};

use crate::{
    logger::{background_log, FrontendLogger},
    types::{PluginStatus, VstPluginEntry},
};

use super::{
    callback_midi::midi_to_rack_event,
    plugin_host::{load_plugin_backend, set_active_vst2_editor_window, VST2_RESIZE_MESSAGE},
    runtime_state::{MidiPacket, PluginBackend, Vst2EventBatch, MIDI_EVENT_BATCH_CAPACITY},
    state_codec::{load_vst_state, save_vst_state_blocking},
};

#[cfg(target_pointer_width = "64")]
fn merge_parameter_commands(
    pending: &mut Vec<super::runtime_state::ParameterCommand>,
    incoming: &[super::runtime_state::ParameterCommand],
) {
    for command in incoming {
        if let Some(existing) = pending.iter_mut().find(|existing| {
            existing.index == command.index
                || (existing.id != 0 && command.id != 0 && existing.id == command.id)
        }) {
            *existing = *command;
            continue;
        }
        if pending.len() == BRIDGE_MAX_PARAMETER_CHANGES {
            pending.remove(0);
        }
        pending.push(*command);
    }
}

const BRIDGE_MAGIC: u32 = 0x4F53_4342;
const BRIDGE_VERSION: u32 = 5;
const BRIDGE_SLOTS: usize = 3;
const BRIDGE_MAX_FRAMES: usize = 8192;
const BRIDGE_MAX_CHANNELS: usize = 64;
const BRIDGE_MAX_MIDI: usize = 512;
const BRIDGE_MAX_PARAMETERS: usize = 2048;
const BRIDGE_MAX_PARAMETER_CHANGES: usize = 512;
const SLOT_EMPTY: u32 = 0;
const SLOT_REQUESTED: u32 = 1;
const SLOT_READY: u32 = 2;
const SERVER_READY: u32 = 1;
const SERVER_FAILED: u32 = 2;
const COMMAND_NONE: u32 = 0;
const COMMAND_OPEN_EDITOR: u32 = 1;
const COMMAND_CLOSE_EDITOR: u32 = 2;
const COMMAND_SAVE_STATE: u32 = 3;
const COMMAND_PENDING: u32 = 1;
const COMMAND_OK: u32 = 2;
const COMMAND_FAILED: u32 = 3;
const REMOTE_EDITOR_STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0);
static REMOTE_EDITOR_CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);
thread_local! {
    // The remote editor and its window are owned and pumped by this one control
    // thread. Boxing the GUI keeps this pointer stable until worker teardown.
    static REMOTE_VST3_GUI: Cell<*mut rack::vst3::Vst3PluginGui> = const {
        Cell::new(std::ptr::null_mut())
    };
}
// Eight 512-frame callbacks at 48 kHz represented only ~85 ms and caused
// healthy legacy plug-ins to be restarted after ordinary scheduling spikes.
// Keep the callback non-blocking, but require a sustained two-second DSP stall
// before declaring the isolated process unhealthy.
const BRIDGE_STALL_BUDGET_MS: u64 = 2_000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BridgeMidiEvent {
    data: [u8; 3],
    len: u8,
    sample_offset: u32,
}

fn write_panic_events(events: &mut [BridgeMidiEvent; BRIDGE_MAX_MIDI]) -> usize {
    let mut event_count = 0;
    for channel in 0..16u8 {
        for controller in super::runtime_state::RESET_CONTROLLERS {
            let Some(event) = events.get_mut(event_count) else {
                return event_count;
            };
            *event = BridgeMidiEvent {
                data: [0xB0 | channel, controller, 0],
                len: 3,
                sample_offset: 0,
            };
            event_count += 1;
        }
    }
    event_count
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BridgeParameterChange {
    index: u32,
    id: u32,
    flags: u32,
    step_count: i32,
    value: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BridgeParameterInfo {
    id: u32,
    flags: u32,
    step_count: i32,
    unit_id: i32,
    name: [u8; 64],
    unit: [u8; 24],
    min: f32,
    max: f32,
    default: f32,
    value: f32,
}

#[repr(C, align(64))]
struct BridgeSlot {
    state: AtomicU32,
    frames: AtomicU32,
    event_count: AtomicU32,
    parameter_change_count: AtomicU32,
    sequence: AtomicU64,
    events: [BridgeMidiEvent; BRIDGE_MAX_MIDI],
    parameter_changes: [BridgeParameterChange; BRIDGE_MAX_PARAMETER_CHANGES],
    outputs: [f32; BRIDGE_MAX_FRAMES * BRIDGE_MAX_CHANNELS],
}

#[repr(C, align(64))]
struct BridgeShared {
    magic: u32,
    version: u32,
    token: u64,
    server_status: AtomicU32,
    shutdown: AtomicU32,
    input_channels: AtomicU32,
    output_channels: AtomicU32,
    plugin_latency_samples: AtomicU32,
    parameter_count: AtomicU32,
    max_block_size: AtomicU32,
    underruns: AtomicU32,
    overruns: AtomicU32,
    error_len: AtomicU32,
    error: [u8; 512],
    parameter_info: [BridgeParameterInfo; BRIDGE_MAX_PARAMETERS],
    command: AtomicU32,
    command_status: AtomicU32,
    slots: [BridgeSlot; BRIDGE_SLOTS],
}

struct Mapping {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    shared: NonNull<BridgeShared>,
}

unsafe impl Send for Mapping {}

impl Mapping {
    #[cfg(target_pointer_width = "64")]
    fn create(name: &str, token: u64, max_block_size: usize) -> Result<Self, String> {
        let wide = wide(name);
        let bytes = size_of::<BridgeShared>();
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                (bytes as u64 >> 32) as u32,
                bytes as u32,
                PCWSTR(wide.as_ptr()),
            )
        }
        .map_err(|error| format!("CreateFileMappingW failed: {error}"))?;
        let mapping = Self::map(handle)?;
        unsafe {
            std::ptr::write_bytes(mapping.view.Value, 0, bytes);
            let shared = mapping.shared.as_ptr();
            (*shared).magic = BRIDGE_MAGIC;
            (*shared).version = BRIDGE_VERSION;
            (*shared).token = token;
            (*shared).max_block_size.store(
                max_block_size.min(BRIDGE_MAX_FRAMES) as u32,
                Ordering::Relaxed,
            );
        }
        Ok(mapping)
    }

    fn open(name: &str, token: u64) -> Result<Self, String> {
        let wide = wide(name);
        let handle =
            unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, PCWSTR(wide.as_ptr())) }
                .map_err(|error| format!("OpenFileMappingW failed: {error}"))?;
        let mapping = Self::map(handle)?;
        let shared = unsafe { mapping.shared.as_ref() };
        if shared.magic != BRIDGE_MAGIC || shared.version != BRIDGE_VERSION || shared.token != token
        {
            return Err("DSP bridge mapping authentication/version mismatch".to_string());
        }
        Ok(mapping)
    }

    fn map(handle: HANDLE) -> Result<Self, String> {
        let view =
            unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size_of::<BridgeShared>()) };
        let Some(shared) = NonNull::new(view.Value.cast::<BridgeShared>()) else {
            let _ = unsafe { CloseHandle(handle) };
            return Err("MapViewOfFile returned a null address".to_string());
        };
        Ok(Self {
            handle,
            view,
            shared,
        })
    }

    fn get(&self) -> &BridgeShared {
        unsafe { self.shared.as_ref() }
    }

    fn get_mut(&mut self) -> &mut BridgeShared {
        unsafe { self.shared.as_mut() }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = unsafe { UnmapViewOfFile(self.view) };
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(target_pointer_width = "64")]
pub(super) struct RemotePlugin {
    mapping: Mapping,
    request_event: HANDLE,
    child: Child,
    next_sequence: u64,
    output_channels: usize,
    input_channels: usize,
    latency_samples: u32,
    sample_rate: u32,
    panic_pending: bool,
    pending_parameter_changes: Vec<super::runtime_state::ParameterCommand>,
    consecutive_missed_frames: u64,
    parameter_cache: Vec<crate::types::VstParameter>,
    last_output_samples: [f32; BRIDGE_MAX_CHANNELS],
}

#[cfg(target_pointer_width = "64")]
unsafe impl Send for RemotePlugin {}

#[cfg(target_pointer_width = "64")]
impl RemotePlugin {
    pub(super) fn spawn(
        path: &Path,
        sample_rate: u32,
        max_block_size: usize,
        probe: &VstPluginEntry,
    ) -> Result<Self, String> {
        if max_block_size > BRIDGE_MAX_FRAMES {
            return Err(format!(
                "x86 bridge supports blocks up to {BRIDGE_MAX_FRAMES} frames, requested {max_block_size}"
            ));
        }
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let mapping_name = format!("Local\\OSCMidi-VST-{nonce}");
        let event_name = format!("Local\\OSCMidi-VST-Request-{nonce}");
        let token = stable_token(&nonce);
        let mapping = Mapping::create(&mapping_name, token, max_block_size)?;
        let event_wide = wide(&event_name);
        let request_event =
            unsafe { CreateEventW(None, false, false, PCWSTR(event_wide.as_ptr())) }
                .map_err(|error| format!("CreateEventW failed: {error}"))?;

        let current = std::env::current_exe()
            .map_err(|error| format!("Cannot resolve x64 worker path: {error}"))?;
        let directory = current
            .parent()
            .ok_or_else(|| "Worker directory unavailable".to_string())?;
        let worker = std::env::var_os("OSCMIDI_VST_WORKER_X86_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join("vst-host-worker-x86.exe"));
        if !worker.is_file() {
            let _ = unsafe { CloseHandle(request_event) };
            return Err(format!("VST x86 worker not found at {}", worker.display()));
        }

        let mut command = Command::new(worker);
        command
            .arg("--dsp-bridge")
            .arg(&mapping_name)
            .arg("--request-event")
            .arg(&event_name)
            .arg("--bridge-token")
            .arg(token.to_string())
            .arg("--plugin")
            .arg(path)
            .arg("--plugin-id")
            .arg(&probe.id)
            .arg("--sample-rate")
            .arg(sample_rate.to_string())
            .arg("--max-block-size")
            .arg(max_block_size.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(uid) = probe.class_uid.as_deref() {
            command.arg("--class-uid").arg(uid);
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("Failed to launch VST x86 DSP worker: {error}"))?;

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match mapping.get().server_status.load(Ordering::Acquire) {
                SERVER_READY => break,
                SERVER_FAILED => {
                    let reason = mapping_error(mapping.get());
                    let _ = child.kill();
                    let _ = unsafe { CloseHandle(request_event) };
                    return Err(reason);
                }
                _ => {}
            }
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                let _ = unsafe { CloseHandle(request_event) };
                return Err(format!(
                    "VST x86 DSP worker exited during startup: {status}"
                ));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = unsafe { CloseHandle(request_event) };
                return Err("VST x86 DSP worker timed out during startup".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let shared = mapping.get();
        let output_channels = shared.output_channels.load(Ordering::Acquire) as usize;
        let input_channels = shared.input_channels.load(Ordering::Acquire) as usize;
        let latency_samples = shared.plugin_latency_samples.load(Ordering::Acquire);
        let parameter_count =
            (shared.parameter_count.load(Ordering::Acquire) as usize).min(BRIDGE_MAX_PARAMETERS);
        let parameter_cache = shared.parameter_info[..parameter_count]
            .iter()
            .enumerate()
            .map(|(index, info)| crate::types::VstParameter {
                index,
                id: info.id,
                flags: info.flags,
                step_count: info.step_count,
                unit_id: info.unit_id,
                name: fixed_string(&info.name),
                min: info.min,
                max: info.max,
                default: info.default,
                unit: fixed_string(&info.unit),
                value: info.value,
            })
            .collect();
        if output_channels == 0 || output_channels > BRIDGE_MAX_CHANNELS {
            let _ = child.kill();
            let _ = unsafe { CloseHandle(request_event) };
            return Err(format!(
                "Unsupported x86 plugin output count: {output_channels}"
            ));
        }
        Ok(Self {
            mapping,
            request_event,
            child,
            next_sequence: 1,
            output_channels,
            input_channels,
            latency_samples,
            sample_rate,
            panic_pending: false,
            pending_parameter_changes: Vec::with_capacity(BRIDGE_MAX_PARAMETER_CHANGES),
            consecutive_missed_frames: 0,
            parameter_cache,
            last_output_samples: [0.0; BRIDGE_MAX_CHANNELS],
        })
    }

    pub(super) fn output_channels(&self) -> usize {
        self.output_channels
    }

    pub(super) fn input_channels(&self) -> usize {
        self.input_channels
    }

    pub(super) fn latency_samples(&self) -> u32 {
        self.latency_samples
    }

    pub(super) fn parameters(&self) -> &[crate::types::VstParameter] {
        &self.parameter_cache
    }

    pub(super) fn process(
        &mut self,
        midi: &mut VecDeque<MidiPacket>,
        parameters: &[super::runtime_state::ParameterCommand],
        outputs: &mut [Vec<f32>],
        frames: usize,
    ) -> bool {
        if frames == 0 || frames > BRIDGE_MAX_FRAMES {
            return false;
        }
        merge_parameter_commands(&mut self.pending_parameter_changes, parameters);
        let priming = self.next_sequence == 1;
        let shared = self.mapping.get_mut();
        let expected_sequence = self.next_sequence.saturating_sub(1);
        let mut rendered = false;
        for slot in &shared.slots {
            if slot.state.load(Ordering::Acquire) != SLOT_READY {
                continue;
            }
            let sequence = slot.sequence.load(Ordering::Acquire);
            if sequence < expected_sequence {
                slot.state.store(SLOT_EMPTY, Ordering::Release);
                shared.overruns.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if sequence != expected_sequence {
                continue;
            }
            let rendered_frames = slot.frames.load(Ordering::Acquire) as usize;
            if rendered_frames != frames {
                slot.state.store(SLOT_EMPTY, Ordering::Release);
                break;
            }
            for (channel, output) in outputs.iter_mut().take(self.output_channels).enumerate() {
                let source = &slot.outputs
                    [channel * BRIDGE_MAX_FRAMES..channel * BRIDGE_MAX_FRAMES + frames];
                output[..frames].copy_from_slice(source);
                self.last_output_samples[channel] = source[frames.saturating_sub(1)];
            }
            slot.state.store(SLOT_EMPTY, Ordering::Release);
            rendered = true;
            break;
        }
        if !rendered && self.next_sequence > 1 {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
            for (channel, output) in outputs.iter_mut().take(self.output_channels).enumerate() {
                let start = self.last_output_samples[channel];
                for (index, sample) in output[..frames].iter_mut().enumerate() {
                    let remaining = 1.0 - (index + 1) as f32 / frames.max(1) as f32;
                    *sample = start * remaining;
                }
                self.last_output_samples[channel] = 0.0;
            }
            let control_busy = shared.command.load(Ordering::Acquire) != COMMAND_NONE
                || shared.command_status.load(Ordering::Acquire) == COMMAND_PENDING;
            if control_busy {
                self.consecutive_missed_frames = 0;
            } else {
                self.consecutive_missed_frames =
                    self.consecutive_missed_frames.saturating_add(frames as u64);
            }
            if bridge_stall_budget_exceeded(self.consecutive_missed_frames, self.sample_rate) {
                set_mapping_error(
                    shared,
                    "x86 DSP bridge produced no completed block for 2000 ms",
                );
            }
        } else if rendered {
            self.consecutive_missed_frames = 0;
        } else if priming {
            for output in outputs.iter_mut().take(self.output_channels) {
                output[..frames].fill(0.0);
            }
        }

        if let Some(slot) = shared
            .slots
            .iter_mut()
            .find(|slot| slot.state.load(Ordering::Acquire) == SLOT_EMPTY)
        {
            let mut event_count = 0usize;
            if self.panic_pending {
                midi.clear();
                event_count = write_panic_events(&mut slot.events);
                self.panic_pending = false;
            } else {
                while event_count < BRIDGE_MAX_MIDI {
                    let Some(packet) = midi.pop_front() else {
                        break;
                    };
                    slot.events[event_count] = BridgeMidiEvent {
                        data: packet.data,
                        len: packet.len,
                        sample_offset: bridge_sample_offset(packet, frames, self.sample_rate),
                    };
                    event_count += 1;
                }
            }
            let parameter_count = self.pending_parameter_changes.len();
            for (target, source) in slot
                .parameter_changes
                .iter_mut()
                .zip(self.pending_parameter_changes.iter())
                .take(parameter_count)
            {
                *target = BridgeParameterChange {
                    index: source.index.min(u32::MAX as usize) as u32,
                    id: source.id,
                    flags: source.flags,
                    step_count: source.step_count,
                    value: source.value,
                };
            }
            slot.frames.store(frames as u32, Ordering::Relaxed);
            slot.event_count
                .store(event_count as u32, Ordering::Relaxed);
            slot.parameter_change_count
                .store(parameter_count as u32, Ordering::Relaxed);
            self.pending_parameter_changes.clear();
            slot.sequence.store(self.next_sequence, Ordering::Relaxed);
            self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
            slot.state.store(SLOT_REQUESTED, Ordering::Release);
            let _ = unsafe { SetEvent(self.request_event) };
        } else {
            shared.overruns.fetch_add(1, Ordering::Relaxed);
        }
        rendered || priming
    }

    pub(super) fn panic_all_notes(&mut self) {
        self.panic_pending = true;
    }

    pub(super) fn telemetry(&mut self) -> (u32, u32, bool) {
        let shared = self.mapping.get();
        let process_alive = self
            .child
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(false);
        let alive = process_alive
            && shared.server_status.load(Ordering::Acquire) == SERVER_READY
            && shared.shutdown.load(Ordering::Acquire) == 0;
        (
            shared.underruns.load(Ordering::Relaxed),
            shared.overruns.load(Ordering::Relaxed),
            alive,
        )
    }

    pub(super) fn fault_reason(&mut self) -> Option<String> {
        let shared = self.mapping.get();
        if shared.server_status.load(Ordering::Acquire) == SERVER_FAILED {
            return Some(mapping_error(shared));
        }
        if shared.shutdown.load(Ordering::Acquire) != 0 {
            let reason = mapping_error(shared);
            return Some(if shared.error_len.load(Ordering::Acquire) == 0 {
                "x86 DSP worker requested shutdown after a processing failure".to_string()
            } else {
                reason
            });
        }
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("x86 DSP worker exited: {status}")),
            Err(error) => Some(format!("cannot query x86 DSP worker: {error}")),
            Ok(None) => None,
        }
    }

    pub(super) fn open_editor(&mut self) -> Result<(), String> {
        self.control_command(COMMAND_OPEN_EDITOR, "editor")
    }

    pub(super) fn close_editor(&mut self) -> Result<(), String> {
        self.control_command(COMMAND_CLOSE_EDITOR, "editor")
    }

    pub(super) fn save_state(&mut self) -> Result<(), String> {
        self.control_command(COMMAND_SAVE_STATE, "state save")
    }

    fn control_command(&mut self, command: u32, operation: &str) -> Result<(), String> {
        let shared = self.mapping.get();
        if shared
            .command
            .compare_exchange(COMMAND_NONE, command, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(format!("x86 {operation} command already in progress"));
        }
        shared
            .command_status
            .store(COMMAND_PENDING, Ordering::Release);
        let _ = unsafe { SetEvent(self.request_event) };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match shared.command_status.load(Ordering::Acquire) {
                COMMAND_OK => {
                    shared.command.store(COMMAND_NONE, Ordering::Release);
                    return Ok(());
                }
                COMMAND_FAILED => {
                    shared.command.store(COMMAND_NONE, Ordering::Release);
                    return Err(mapping_error(shared));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                shared.command.store(COMMAND_NONE, Ordering::Release);
                return Err(format!("x86 {operation} command timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(target_pointer_width = "64")]
fn bridge_sample_offset(packet: MidiPacket, frames: usize, sample_rate: u32) -> u32 {
    let period_us = (frames as u64)
        .saturating_mul(1_000_000)
        .checked_div(u64::from(sample_rate.max(1)))
        .unwrap_or(0);
    let block_start = super::runtime_state::monotonic_us().saturating_sub(period_us);
    packet
        .timestamp_us
        .saturating_sub(block_start)
        .saturating_mul(u64::from(sample_rate))
        .checked_div(1_000_000)
        .unwrap_or(0)
        .min(frames.saturating_sub(1) as u64) as u32
}

#[cfg(target_pointer_width = "64")]
fn bridge_stall_budget_exceeded(missed_frames: u64, sample_rate: u32) -> bool {
    missed_frames.saturating_mul(1_000)
        >= u64::from(sample_rate.max(1)).saturating_mul(BRIDGE_STALL_BUDGET_MS)
}

#[cfg(target_pointer_width = "64")]
impl Drop for RemotePlugin {
    fn drop(&mut self) {
        self.mapping.get().shutdown.store(1, Ordering::Release);
        let _ = unsafe { SetEvent(self.request_event) };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                let _ = unsafe { CloseHandle(self.request_event) };
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = unsafe { CloseHandle(self.request_event) };
    }
}

#[derive(Clone)]
struct DspArgs {
    mapping_name: String,
    event_name: String,
    token: u64,
    plugin: PathBuf,
    plugin_id: String,
    sample_rate: u32,
    max_block_size: usize,
    class_uid: Option<String>,
}

fn parse_dsp_args() -> Option<Result<DspArgs, String>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|arg| arg == "--dsp-bridge") {
        return None;
    }
    let value = |flag: &str| -> Result<String, String> {
        let index = args
            .iter()
            .position(|arg| arg == flag)
            .ok_or_else(|| format!("Missing {flag}"))?;
        args.get(index + 1)
            .cloned()
            .ok_or_else(|| format!("Missing value for {flag}"))
    };
    Some((|| {
        Ok(DspArgs {
            mapping_name: value("--dsp-bridge")?,
            event_name: value("--request-event")?,
            token: value("--bridge-token")?
                .parse()
                .map_err(|_| "Invalid bridge token".to_string())?,
            plugin: PathBuf::from(value("--plugin")?),
            plugin_id: value("--plugin-id")?,
            sample_rate: value("--sample-rate")?
                .parse()
                .map_err(|_| "Invalid sample rate".to_string())?,
            max_block_size: value("--max-block-size")?
                .parse()
                .map_err(|_| "Invalid max block size".to_string())?,
            class_uid: args
                .iter()
                .position(|arg| arg == "--class-uid")
                .and_then(|index| args.get(index + 1).cloned()),
        })
    })())
}

pub fn run_dsp_bridge_from_env(logger: FrontendLogger) -> Option<Result<(), String>> {
    let args = match parse_dsp_args()? {
        Ok(args) => args,
        Err(error) => return Some(Err(error)),
    };
    Some(run_dsp_server(args, logger))
}

enum RemoteEditor {
    Vst2 {
        editor: Box<dyn vst::editor::Editor>,
        hwnd: HWND,
    },
    Vst3 {
        _gui: Box<rack::vst3::Vst3PluginGui>,
        hwnd: HWND,
    },
}

unsafe extern "system" fn remote_editor_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            REMOTE_EDITOR_CLOSE_REQUESTED.store(true, Ordering::Release);
            LRESULT(0)
        }
        WM_DESTROY => {
            set_active_vst2_editor_window(None);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        VST2_RESIZE_MESSAGE => {
            let _ = resize_remote_editor_to_client(hwnd, wparam.0 as i32, lparam.0 as i32);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: Windows owns the suggested RECT for the duration of this
            // message, and the GUI pointer is stable and thread-local.
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            let dpi = ((wparam.0 >> 16) & 0xffff).max(96) as f32;
            REMOTE_VST3_GUI.with(|slot| {
                let gui = slot.get();
                if !gui.is_null() {
                    let _ = unsafe { &mut *gui }.set_content_scale_factor(dpi / 96.0);
                }
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_remote_editor_window(title: &str) -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|error| error.to_string())?;
        let class_name = w!("OSCMidiX86VstEditor");
        let class = WNDCLASSW {
            lpfnWndProc: Some(remote_editor_window_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(instance.0),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR(title.as_ptr()),
            REMOTE_EDITOR_STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            600,
            None,
            None,
            Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
            None,
        )
        .map_err(|error| error.to_string())
    }
}

fn resize_remote_editor_to_client(hwnd: HWND, width: i32, height: i32) -> Result<(), String> {
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return Err(format!("invalid remote editor size {width}x{height}"));
    }
    unsafe {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        AdjustWindowRectExForDpi(
            &mut rect,
            REMOTE_EDITOR_STYLE,
            false,
            WINDOW_EX_STYLE(0),
            GetDpiForWindow(hwnd).max(96),
        )
        .map_err(|error| error.to_string())?;
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .map_err(|error| error.to_string())?;
        // Some legacy 32-bit plug-ins alter non-client metrics while opening.
        // Measure the actual client area and apply one bounded correction so
        // the host client matches effEditGetRect exactly.
        let mut actual = RECT::default();
        GetClientRect(hwnd, &mut actual).map_err(|error| error.to_string())?;
        let actual_width = actual.right - actual.left;
        let actual_height = actual.bottom - actual.top;
        if actual_width != width || actual_height != height {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                rect.right - rect.left + (width - actual_width),
                rect.bottom - rect.top + (height - actual_height),
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn open_remote_editor(
    plugin: &mut PluginBackend,
    current: &mut Option<RemoteEditor>,
) -> Result<(), String> {
    if let Some(editor) = current.as_ref() {
        let hwnd = match editor {
            RemoteEditor::Vst2 { hwnd, .. } | RemoteEditor::Vst3 { hwnd, .. } => *hwnd,
        };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        return Ok(());
    }
    match plugin {
        PluginBackend::Vst2 { instance, .. } => {
            use vst::plugin::Plugin as _;
            let mut editor = instance
                .get_editor()
                .ok_or_else(|| "x86 VST2 plugin does not expose an editor".to_string())?;
            let hwnd = create_remote_editor_window("VST2 x86 Editor")?;
            set_active_vst2_editor_window(Some(hwnd));
            if !editor.open(hwnd.0) {
                set_active_vst2_editor_window(None);
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err("x86 VST2 editor refused its parent window".to_string());
            }
            editor.idle();
            let (width, height) = editor.size();
            if let Err(error) = resize_remote_editor_to_client(hwnd, width, height) {
                editor.close();
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err(format!("x86 VST2 editor sizing failed: {error}"));
            }
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
            editor.idle();
            let settled_size = editor.size();
            if settled_size != (width, height) {
                resize_remote_editor_to_client(hwnd, settled_size.0, settled_size.1)?;
            }
            *current = Some(RemoteEditor::Vst2 { editor, hwnd });
            Ok(())
        }
        PluginBackend::Vst3 { instance, .. } => {
            let mut gui = Box::new(instance.create_gui().map_err(|error| error.to_string())?);
            let hwnd = create_remote_editor_window("VST3 x86 Editor")?;
            gui.attach(hwnd.0).map_err(|error| error.to_string())?;
            let (width, height) = gui.size().map_err(|error| error.to_string())?;
            resize_remote_editor_to_client(hwnd, width.round() as i32, height.round() as i32)?;
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
            REMOTE_VST3_GUI.with(|slot| slot.set(gui.as_mut() as *mut _));
            *current = Some(RemoteEditor::Vst3 { _gui: gui, hwnd });
            Ok(())
        }
        #[cfg(target_pointer_width = "64")]
        PluginBackend::Remote { .. } => Err("Nested remote editor is unsupported".to_string()),
    }
}

fn close_remote_editor(current: &mut Option<RemoteEditor>) {
    if let Some(editor) = current.as_mut() {
        let hwnd = match editor {
            RemoteEditor::Vst2 { hwnd, .. } => *hwnd,
            RemoteEditor::Vst3 { hwnd, .. } => *hwnd,
        };
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
}

fn pump_remote_editor(current: &mut Option<RemoteEditor>) {
    if let Some(RemoteEditor::Vst2 { editor, .. }) = current.as_mut() {
        editor.idle();
    }
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}

fn run_dsp_server(args: DspArgs, logger: FrontendLogger) -> Result<(), String> {
    let mut mapping = Mapping::open(&args.mapping_name, args.token)?;
    let event_wide = wide(&args.event_name);
    let request_event = unsafe {
        OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            false,
            PCWSTR(event_wide.as_ptr()),
        )
    }
    .map_err(|error| format!("OpenEventW failed: {error}"))?;

    // The parent already probed and selected the plugin in a disposable
    // process. Probing again here would instantiate the DLL once before the
    // real DSP instance. Legacy plugins such as Church Organ cannot safely
    // create that second instance after the quarantined probe instance.
    let selected_probe =
        runtime_plugin_reference(&args.plugin, args.plugin_id.clone(), args.class_uid.clone());
    let load_result = load_plugin_backend(
        &args.plugin,
        args.sample_rate,
        args.max_block_size.min(u32::MAX as usize) as u32,
        args.max_block_size,
        &logger,
        Some(&selected_probe),
    );
    let loaded = match load_result {
        Ok(loaded) => loaded,
        Err(error) => {
            set_mapping_error(mapping.get_mut(), &error.to_string());
            let _ = unsafe { CloseHandle(request_event) };
            return Err(error.to_string());
        }
    };
    let quarantine_destructor = match &loaded.backend {
        PluginBackend::Vst2 { instance, .. } => {
            use vst::plugin::Plugin as _;
            crate::plugin_probe::requires_vst2_destructor_quarantine(instance.get_info().unique_id)
        }
        _ => false,
    };
    let plugin = std::sync::Arc::new(parking_lot::Mutex::new(loaded.backend));
    let mut restored_parameter_cache = loaded.parameter_cache.clone();

    // State restoration may invoke slow native plugin code. Do it before the
    // ready flag so the x64 callback cannot start publishing audio requests
    // and incorrectly fault the bridge for startup underruns.
    load_vst_state(&plugin, &selected_probe, &logger);
    refresh_parameter_values_after_restore(&mut plugin.lock(), &mut restored_parameter_cache);
    {
        let shared = mapping.get_mut();
        shared
            .input_channels
            .store(loaded.input_channels as u32, Ordering::Relaxed);
        shared
            .output_channels
            .store(loaded.output_channels as u32, Ordering::Relaxed);
        shared
            .plugin_latency_samples
            .store(loaded.latency_samples, Ordering::Relaxed);
        let parameter_count = restored_parameter_cache.len().min(BRIDGE_MAX_PARAMETERS);
        for (target, source) in shared
            .parameter_info
            .iter_mut()
            .zip(restored_parameter_cache.iter())
            .take(parameter_count)
        {
            target.id = source.id;
            target.flags = source.flags;
            target.step_count = source.step_count;
            target.unit_id = source.unit_id;
            target.name = fixed_bytes(&source.name);
            target.unit = fixed_bytes(&source.unit);
            target.min = source.min;
            target.max = source.max;
            target.default = source.default;
            target.value = source.value;
        }
        shared
            .parameter_count
            .store(parameter_count as u32, Ordering::Relaxed);
        shared.server_status.store(SERVER_READY, Ordering::Release);
    }

    let mut input_silence = vec![0.0f32; args.max_block_size];
    let mut vst2_events = Vec::with_capacity(BRIDGE_MAX_MIDI);
    let mut vst2_batch = Vst2EventBatch {
        num_events: 0,
        reserved: 0,
        events: [std::ptr::null_mut(); MIDI_EVENT_BATCH_CAPACITY],
    };
    let mut rack_events = Vec::with_capacity(BRIDGE_MAX_MIDI);
    let mut editor = None;

    while mapping.get().shutdown.load(Ordering::Acquire) == 0 {
        let _ = unsafe { WaitForSingleObject(request_event, 50) };
        if mapping.get().shutdown.load(Ordering::Acquire) != 0 {
            break;
        }
        pump_remote_editor(&mut editor);
        if REMOTE_EDITOR_CLOSE_REQUESTED.swap(false, Ordering::AcqRel) {
            if mapping
                .get()
                .command
                .compare_exchange(
                    COMMAND_NONE,
                    COMMAND_SAVE_STATE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                mapping
                    .get()
                    .command_status
                    .store(COMMAND_PENDING, Ordering::Release);
                save_vst_state_blocking(&plugin, &selected_probe);
                mapping
                    .get()
                    .command_status
                    .store(COMMAND_OK, Ordering::Release);
                mapping.get().command.store(COMMAND_NONE, Ordering::Release);
            } else {
                REMOTE_EDITOR_CLOSE_REQUESTED.store(true, Ordering::Release);
            }
        }
        let command = mapping.get().command.load(Ordering::Acquire);
        if command != COMMAND_NONE
            && mapping.get().command_status.load(Ordering::Acquire) == COMMAND_PENDING
        {
            let result = match command {
                COMMAND_OPEN_EDITOR => open_remote_editor(&mut plugin.lock(), &mut editor),
                COMMAND_CLOSE_EDITOR => {
                    close_remote_editor(&mut editor);
                    Ok(())
                }
                COMMAND_SAVE_STATE => {
                    save_vst_state_blocking(&plugin, &selected_probe);
                    for slot in &mapping.get().slots {
                        slot.state.store(SLOT_EMPTY, Ordering::Release);
                    }
                    Ok(())
                }
                _ => Err("unknown x86 control command".to_string()),
            };
            match result {
                Ok(()) => mapping
                    .get()
                    .command_status
                    .store(COMMAND_OK, Ordering::Release),
                Err(error) => {
                    set_command_error(mapping.get_mut(), &error);
                    mapping
                        .get()
                        .command_status
                        .store(COMMAND_FAILED, Ordering::Release);
                }
            }
        }
        loop {
            let slot_index = (0..BRIDGE_SLOTS)
                .filter(|index| {
                    mapping.get().slots[*index].state.load(Ordering::Acquire) == SLOT_REQUESTED
                })
                .min_by_key(|index| mapping.get().slots[*index].sequence.load(Ordering::Acquire));
            let Some(slot_index) = slot_index else {
                break;
            };
            let shared = mapping.get_mut();
            let slot = &mut shared.slots[slot_index];
            if slot.state.load(Ordering::Acquire) != SLOT_REQUESTED {
                continue;
            }
            let frames = slot.frames.load(Ordering::Relaxed) as usize;
            let event_count = slot.event_count.load(Ordering::Relaxed) as usize;
            let parameter_change_count =
                slot.parameter_change_count.load(Ordering::Relaxed) as usize;
            if frames == 0
                || frames > args.max_block_size
                || event_count > BRIDGE_MAX_MIDI
                || parameter_change_count > BRIDGE_MAX_PARAMETER_CHANGES
            {
                slot.state.store(SLOT_EMPTY, Ordering::Release);
                continue;
            }
            input_silence[..frames].fill(0.0);
            let process_result = process_local_plugin(
                &mut plugin.lock(),
                slot,
                frames,
                event_count,
                parameter_change_count,
                &restored_parameter_cache,
                &input_silence,
                loaded.input_channels,
                loaded.output_channels,
                &mut vst2_events,
                &mut vst2_batch,
                &mut rack_events,
            );
            if let Err(error) = process_result {
                background_log("error", format!("x86 DSP bridge process failed: {error}"));
                mapping.get().shutdown.store(1, Ordering::Release);
                break;
            }
            slot.state.store(SLOT_READY, Ordering::Release);
        }
    }
    close_remote_editor(&mut editor);
    REMOTE_VST3_GUI.with(|slot| slot.set(std::ptr::null_mut()));
    if let Some(editor) = editor.take() {
        let hwnd = match editor {
            RemoteEditor::Vst2 { mut editor, hwnd } => {
                editor.close();
                hwnd
            }
            RemoteEditor::Vst3 { hwnd, .. } => hwnd,
        };
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    save_vst_state_blocking(&plugin, &selected_probe);
    if quarantine_destructor {
        background_log(
            "warn",
            "Quarantining Church Organ VST2 instance destructor; memory will be reclaimed when the x86 worker exits",
        );
        std::mem::forget(plugin);
    } else {
        drop(plugin);
    }
    let _ = unsafe { CloseHandle(request_event) };
    Ok(())
}

fn refresh_parameter_values_after_restore(
    plugin: &mut PluginBackend,
    cache: &mut [crate::types::VstParameter],
) {
    match plugin {
        PluginBackend::Vst2 { instance, .. } => {
            use vst::plugin::Plugin as _;
            let parameters = instance.get_parameter_object();
            for parameter in cache {
                parameter.value = parameters
                    .get_parameter(parameter.index as i32)
                    .clamp(0.0, 1.0);
            }
        }
        PluginBackend::Vst3 { instance, .. } => {
            use rack::PluginInstance as _;
            for parameter in cache {
                if let Ok(value) = instance.get_parameter(parameter.index) {
                    parameter.value = value.clamp(0.0, 1.0);
                }
            }
        }
        #[cfg(target_pointer_width = "64")]
        PluginBackend::Remote { .. } => {}
    }
}

fn runtime_plugin_reference(
    path: &Path,
    plugin_id: String,
    class_uid: Option<String>,
) -> VstPluginEntry {
    let format = if crate::vst_scan::is_vst3_path(path) {
        "VST3"
    } else {
        "VST2"
    };
    let architecture = if cfg!(target_pointer_width = "32") {
        "x86"
    } else {
        "x64"
    };
    let (file_modified_ms, file_size) = crate::plugin_probe::plugin_file_metadata(path);
    let mut entry = VstPluginEntry {
        id: plugin_id,
        name: path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        path: path.to_string_lossy().into_owned(),
        format: format.to_string(),
        kind: "instrument".to_string(),
        architecture: architecture.to_string(),
        available_architectures: vec![architecture.to_string()],
        vendor: None,
        plugin_version: None,
        status: PluginStatus::Unverified,
        supported: true,
        unsupported_reason: None,
        failure_stage: None,
        last_error: None,
        last_probed_ms: None,
        midi_compatible: None,
        has_editor: false,
        channel_layout: None,
        class_uid,
        sub_plugin_id: None,
        hosting_mode: None,
        file_modified_ms,
        file_size,
        host_abi_version: crate::types::VST_HOST_ABI_VERSION,
    };
    entry.refresh_derived_fields();
    entry
}

#[allow(clippy::too_many_arguments)]
fn process_local_plugin(
    plugin: &mut PluginBackend,
    slot: &mut BridgeSlot,
    frames: usize,
    event_count: usize,
    parameter_change_count: usize,
    parameter_metadata: &[crate::types::VstParameter],
    input_silence: &[f32],
    input_channels: usize,
    output_channels: usize,
    vst2_events: &mut Vec<vst::api::MidiEvent>,
    vst2_batch: &mut Vst2EventBatch,
    rack_events: &mut Vec<rack::prelude::MidiEvent>,
) -> Result<(), String> {
    match plugin {
        PluginBackend::Vst2 { instance, .. } => {
            use vst::plugin::Plugin as _;
            let parameters = instance.get_parameter_object();
            for change in slot.parameter_changes.iter().take(parameter_change_count) {
                let Some(metadata) = parameter_metadata.get(change.index as usize) else {
                    continue;
                };
                if metadata.id != change.id
                    || metadata.flags != change.flags
                    || metadata.step_count != change.step_count
                {
                    continue;
                }
                parameters.set_parameter(change.index as i32, change.value.clamp(0.0, 1.0));
            }
        }
        PluginBackend::Vst3 { instance, .. } => {
            for change in slot.parameter_changes.iter().take(parameter_change_count) {
                let Some(metadata) = parameter_metadata.get(change.index as usize) else {
                    continue;
                };
                if metadata.id != change.id
                    || metadata.flags != change.flags
                    || metadata.step_count != change.step_count
                {
                    continue;
                }
                instance
                    .set_parameter_audio(change.index as usize, change.value)
                    .map_err(|error| error.to_string())?;
            }
        }
        #[cfg(target_pointer_width = "64")]
        PluginBackend::Remote { .. } => {}
    }
    match plugin {
        PluginBackend::Vst2 { instance, time } => {
            use vst::{api, buffer::AudioBuffer, plugin::Plugin};
            vst2_events.clear();
            for event in slot.events.iter().take(event_count) {
                vst2_events.push(api::MidiEvent {
                    event_type: api::EventType::Midi,
                    byte_size: size_of::<api::MidiEvent>() as i32,
                    delta_frames: event.sample_offset.min(frames.saturating_sub(1) as u32) as i32,
                    flags: api::MidiEventFlags::REALTIME_EVENT.bits(),
                    note_length: 0,
                    note_offset: 0,
                    midi_data: event.data,
                    _midi_reserved: 0,
                    detune: 0,
                    note_off_velocity: 0,
                    _reserved1: 0,
                    _reserved2: 0,
                });
            }
            for (index, event) in vst2_events.iter_mut().enumerate() {
                vst2_batch.events[index] = event as *mut api::MidiEvent as *mut api::Event;
            }
            vst2_batch.num_events = vst2_events.len() as i32;
            if vst2_batch.num_events > 0 {
                let events =
                    unsafe { &*(vst2_batch as *const Vst2EventBatch as *const api::Events) };
                instance.process_events(events);
            }
            let input_ptrs: SmallVec<[*const f32; BRIDGE_MAX_CHANNELS]> = (0..input_channels)
                .map(|_| input_silence.as_ptr())
                .collect();
            let mut output_ptrs: SmallVec<[*mut f32; BRIDGE_MAX_CHANNELS]> = (0..output_channels)
                .map(|channel| unsafe {
                    slot.outputs.as_mut_ptr().add(channel * BRIDGE_MAX_FRAMES)
                })
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
            time.advance(frames);
            Ok(())
        }
        PluginBackend::Vst3 { instance, .. } => {
            use rack::PluginInstance as _;
            rack_events.clear();
            for event in slot.events.iter().take(event_count) {
                if let Some(mut converted) = midi_to_rack_event(event.data) {
                    converted.sample_offset = event.sample_offset;
                    rack_events.push(converted);
                }
            }
            if !rack_events.is_empty() {
                instance
                    .send_midi(rack_events)
                    .map_err(|error| error.to_string())?;
            }
            let inputs: SmallVec<[&[f32]; BRIDGE_MAX_CHANNELS]> = (0..input_channels)
                .map(|_| &input_silence[..frames])
                .collect();
            let mut outputs: SmallVec<[&mut [f32]; BRIDGE_MAX_CHANNELS]> = SmallVec::new();
            for channel in 0..output_channels {
                let ptr = unsafe { slot.outputs.as_mut_ptr().add(channel * BRIDGE_MAX_FRAMES) };
                outputs.push(unsafe { std::slice::from_raw_parts_mut(ptr, frames) });
            }
            instance
                .process(&inputs, &mut outputs, frames)
                .map_err(|error| error.to_string())
        }
        #[cfg(target_pointer_width = "64")]
        PluginBackend::Remote { .. } => {
            Err("Nested remote VST bridge is not supported".to_string())
        }
    }
}

fn set_mapping_error(shared: &mut BridgeShared, error: &str) {
    let bytes = error.as_bytes();
    let len = bytes.len().min(shared.error.len());
    shared.error[..len].copy_from_slice(&bytes[..len]);
    shared.error_len.store(len as u32, Ordering::Relaxed);
    shared.server_status.store(SERVER_FAILED, Ordering::Release);
}

fn set_command_error(shared: &mut BridgeShared, error: &str) {
    let bytes = error.as_bytes();
    let len = bytes.len().min(shared.error.len());
    shared.error[..len].copy_from_slice(&bytes[..len]);
    shared.error_len.store(len as u32, Ordering::Release);
}

#[cfg(target_pointer_width = "64")]
fn mapping_error(shared: &BridgeShared) -> String {
    let len = (shared.error_len.load(Ordering::Acquire) as usize).min(shared.error.len());
    if len == 0 {
        "VST x86 DSP worker failed without a diagnostic".to_string()
    } else {
        String::from_utf8_lossy(&shared.error[..len]).to_string()
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn fixed_bytes<const N: usize>(value: &str) -> [u8; N] {
    let mut output = [0u8; N];
    let len = value.len().min(N.saturating_sub(1));
    output[..len].copy_from_slice(&value.as_bytes()[..len]);
    output
}

fn fixed_string<const N: usize>(value: &[u8; N]) -> String {
    let len = value.iter().position(|byte| *byte == 0).unwrap_or(N);
    String::from_utf8_lossy(&value[..len]).into_owned()
}

#[cfg(target_pointer_width = "64")]
fn stable_token(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_bridge_layout_is_versioned_and_bounded() {
        println!(
            "bridge-layout:v{}:shared={}:slot={}:slots_offset={}:slot_outputs_offset={}",
            BRIDGE_VERSION,
            size_of::<BridgeShared>(),
            size_of::<BridgeSlot>(),
            std::mem::offset_of!(BridgeShared, slots),
            std::mem::offset_of!(BridgeSlot, outputs),
        );
        assert_eq!(std::mem::align_of::<BridgeShared>(), 64);
        assert_eq!(std::mem::align_of::<BridgeSlot>(), 64);
        assert!(size_of::<BridgeShared>() < 8 * 1024 * 1024);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn bridge_stall_budget_is_based_on_audio_time() {
        assert!(!bridge_stall_budget_exceeded(95_999, 48_000));
        assert!(bridge_stall_budget_exceeded(96_000, 48_000));
        assert!(!bridge_stall_budget_exceeded(88_199, 44_100));
        assert!(bridge_stall_budget_exceeded(88_200, 44_100));
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn parameter_backpressure_keeps_the_latest_value_without_growing() {
        use crate::audio::runtime_state::ParameterCommand;

        let mut pending = Vec::with_capacity(BRIDGE_MAX_PARAMETER_CHANGES);
        let first = ParameterCommand {
            index: 7,
            id: 42,
            flags: 0,
            step_count: 0,
            value: 0.25,
        };
        let latest = ParameterCommand {
            value: 0.75,
            ..first
        };
        merge_parameter_commands(&mut pending, &[first]);
        merge_parameter_commands(&mut pending, &[latest]);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].value, 0.75);
        assert_eq!(pending.capacity(), BRIDGE_MAX_PARAMETER_CHANGES);

        for index in 0..=BRIDGE_MAX_PARAMETER_CHANGES {
            merge_parameter_commands(
                &mut pending,
                &[ParameterCommand {
                    index: index + 100,
                    id: index as u32 + 100,
                    flags: 0,
                    step_count: 0,
                    value: index as f32,
                }],
            );
        }
        assert_eq!(pending.len(), BRIDGE_MAX_PARAMETER_CHANGES);
        assert_eq!(
            pending.last().unwrap().index,
            BRIDGE_MAX_PARAMETER_CHANGES + 100
        );
        assert_eq!(pending.capacity(), BRIDGE_MAX_PARAMETER_CHANGES);
    }

    #[test]
    fn panic_controller_batch_always_fits_in_one_bridge_slot() {
        let mut events = [BridgeMidiEvent::default(); BRIDGE_MAX_MIDI];
        let count = write_panic_events(&mut events);
        assert_eq!(
            count,
            16 * crate::audio::runtime_state::RESET_CONTROLLERS.len()
        );
        assert_eq!(events[0].data, [0xB0, 64, 0]);
        assert_eq!(events[count - 1].data, [0xBF, 123, 0]);
        assert!(count <= BRIDGE_MAX_MIDI);
    }

    #[test]
    fn dsp_runtime_reference_uses_the_selected_class_without_probing() {
        let path = Path::new(r"C:\VST\Shell.dll");
        let entry = runtime_plugin_reference(
            path,
            "vst-selected-shell".to_string(),
            Some("vst2shell:12345678".to_string()),
        );
        assert_eq!(entry.path, path.to_string_lossy());
        assert_eq!(entry.class_uid.as_deref(), Some("vst2shell:12345678"));
        assert_eq!(entry.id, "vst-selected-shell");
        assert_eq!(entry.host_abi_version, crate::types::VST_HOST_ABI_VERSION);
        assert!(entry.supported);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    #[ignore = "requires the locally installed Church Organ x86 VST fixture"]
    fn installed_church_organ_renders_through_x86_bridge() {
        let plugin =
            PathBuf::from(r"C:\Program Files\VstPlugins\Steinberg\VstPlugins\Church Organ.dll");
        if !plugin.is_file() {
            return;
        }
        let worker = std::env::var_os("OSCMIDI_VST_WORKER_X86_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("binaries")
                    .join("vst-host-worker-x86-x86_64-pc-windows-msvc.exe")
            });
        assert!(
            worker.is_file(),
            "prepare the x86 VST worker before this test"
        );
        unsafe { std::env::set_var("OSCMIDI_VST_WORKER_X86_PATH", &worker) };
        let probe = crate::plugin_probe::probe_plugins_isolated(&plugin, Duration::from_secs(30))
            .into_iter()
            .find(|entry| entry.supported)
            .expect("Church Organ probe");
        let mut remote = RemotePlugin::spawn(&plugin, 48_000, 512, &probe).expect("x86 bridge");
        let mut midi = VecDeque::new();
        midi.push_back(MidiPacket::from_bytes(&[0x90, 60, 110]).expect("note on"));
        let mut outputs = vec![vec![0.0f32; 512]; remote.output_channels()];
        let mut energy = 0.0f32;
        for _ in 0..32 {
            if remote.process(&mut midi, &[], &mut outputs, 512) {
                energy += outputs
                    .iter()
                    .flat_map(|channel| channel.iter())
                    .map(|sample| sample.abs())
                    .sum::<f32>();
            }
            std::thread::sleep(Duration::from_millis(12));
        }
        midi.push_back(MidiPacket::from_bytes(&[0x80, 60, 0]).expect("note off"));
        let _ = remote.process(&mut midi, &[], &mut outputs, 512);
        for sequence in 0..10_000u32 {
            let note = 36 + ((sequence / 2) % 48) as u8;
            let bytes: &[u8] = match sequence % 6 {
                0 => &[0x90, note, 100],
                1 => &[0x80, note, 0],
                2 => &[0xB0, 1, (sequence % 128) as u8],
                3 => &[0xC0, (sequence % 16) as u8],
                4 => &[0xE0, (sequence % 128) as u8, ((sequence / 128) % 128) as u8],
                _ => &[0xD0, (sequence % 128) as u8],
            };
            midi.push_back(MidiPacket::from_bytes(bytes).expect("stress MIDI event"));
        }
        for _ in 0..24 {
            let _ = remote.process(&mut midi, &[], &mut outputs, 512);
            std::thread::sleep(Duration::from_millis(12));
        }
        assert!(
            midi.is_empty(),
            "the x86 bridge did not drain 10,000 MIDI events"
        );
        remote.panic_all_notes();
        let _ = remote.process(&mut midi, &[], &mut outputs, 512);
        assert!(
            energy.is_finite() && energy > 0.001,
            "bridged VST rendered silence"
        );
        assert!(remote.telemetry().2, "x86 worker should remain alive");
    }
}
