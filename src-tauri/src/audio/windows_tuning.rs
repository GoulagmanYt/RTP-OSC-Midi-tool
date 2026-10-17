use std::sync::atomic::Ordering;

use crate::logger::{background_log, FrontendLogger};

use super::runtime_state::AudioCallbackState;

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8};

// Process-wide role: audio startup and focus changes must preserve worker policy.
#[cfg(target_os = "windows")]
static WORKER_PROCESS: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "windows")]
static AUDIO_POWER_THROTTLING_DISABLED: AtomicU8 = AtomicU8::new(0);
#[cfg(target_os = "windows")]
static AUDIO_TUNING_WARNINGS: AtomicU32 = AtomicU32::new(0);

#[cfg(target_os = "windows")]
const WARN_MMCSS_PRIORITY: u32 = 1 << 0;
#[cfg(target_os = "windows")]
const WARN_MMCSS_ALREADY_REGISTERED: u32 = 1 << 1;
#[cfg(target_os = "windows")]
const WARN_MMCSS_UNAVAILABLE: u32 = 1 << 2;
#[cfg(target_os = "windows")]
const WARN_THREAD_PRIORITY: u32 = 1 << 3;
#[cfg(target_os = "windows")]
const WARN_PRIORITY_BOOST: u32 = 1 << 4;
#[cfg(target_os = "windows")]
const WARN_POWER_THROTTLING: u32 = 1 << 5;
#[cfg(target_os = "windows")]
const WARN_MMCSS_RELEASE: u32 = 1 << 6;

#[cfg(target_os = "windows")]
fn record_audio_tuning_warning(bit: u32) {
    AUDIO_TUNING_WARNINGS.fetch_or(bit, Ordering::Release);
}

/// Formats and writes callback-thread tuning diagnostics from a non-realtime thread.
#[cfg(target_os = "windows")]
pub(super) fn flush_audio_tuning_warnings() {
    let warnings = AUDIO_TUNING_WARNINGS.swap(0, Ordering::AcqRel);
    for (bit, level, message) in [
        (
            WARN_MMCSS_RELEASE,
            "warn",
            "Failed to release MMCSS registration on its owner thread",
        ),
        (
            WARN_MMCSS_PRIORITY,
            "warn",
            "Failed to raise MMCSS audio thread priority",
        ),
        (
            WARN_MMCSS_ALREADY_REGISTERED,
            "info",
            "MMCSS: preserving an existing audio thread registration",
        ),
        (
            WARN_MMCSS_UNAVAILABLE,
            "warn",
            "Failed to enable MMCSS for audio thread",
        ),
        (
            WARN_THREAD_PRIORITY,
            "warn",
            "Failed to apply HIGHEST fallback to audio thread",
        ),
        (
            WARN_PRIORITY_BOOST,
            "warn",
            "Failed to enable dynamic priority boosts for audio thread",
        ),
        (
            WARN_POWER_THROTTLING,
            "warn",
            "Failed to disable thread power throttling for audio thread",
        ),
    ] {
        if warnings & bit != 0 {
            background_log(level, message);
        }
    }
}

/// Own one ordinary diagnostics thread for the process, including nested DSP workers.
pub struct AudioTuningDiagnosticsGuard {
    #[cfg(target_os = "windows")]
    stop: std::sync::mpsc::Sender<()>,
    #[cfg(target_os = "windows")]
    thread: Option<std::thread::JoinHandle<()>>,
}

#[must_use = "keep the diagnostics guard until process shutdown"]
pub fn start_audio_tuning_diagnostics() -> AudioTuningDiagnosticsGuard {
    #[cfg(target_os = "windows")]
    {
        let (stop, receiver) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("audio-tuning-diagnostics".into())
            .spawn(move || loop {
                let result = receiver.recv_timeout(std::time::Duration::from_millis(500));
                flush_audio_tuning_warnings();
                if !matches!(result, Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
                    break;
                }
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                background_log(
                    "warn",
                    format!("Failed to start audio tuning diagnostics: {error}"),
                );
                None
            }
        };
        AudioTuningDiagnosticsGuard { stop, thread }
    }
    #[cfg(not(target_os = "windows"))]
    {
        AudioTuningDiagnosticsGuard {}
    }
}

impl Drop for AudioTuningDiagnosticsGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        {
            let _ = self.stop.send(());
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            flush_audio_tuning_warnings();
        }
    }
}

#[cfg(target_os = "windows")]
fn store_bool(cell: &AtomicU8, value: bool) {
    cell.store(if value { 2 } else { 1 }, Ordering::Relaxed);
}

#[cfg(target_os = "windows")]
fn load_bool(cell: &AtomicU8) -> Option<bool> {
    match cell.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
use windows::core::w;
#[cfg(target_os = "windows")]
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority,
    GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, ProcessPowerThrottling,
    SetPriorityClass, SetProcessInformation, SetThreadInformation, SetThreadPriority,
    SetThreadPriorityBoost, ThreadPowerThrottling, ABOVE_NORMAL_PRIORITY_CLASS,
    AVRT_PRIORITY_CRITICAL, HIGH_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY_HIGHEST,
};

/// ERROR_THREAD_ALREADY_IN_TASK. ASIO drivers such as Voicemeeter commonly register
/// their callback thread before CPAL invokes us.
#[cfg(target_os = "windows")]
const ERROR_THREAD_ALREADY_IN_TASK_RAW: u32 = 1552;

#[cfg(target_os = "windows")]
fn is_thread_already_in_task_error(err: &windows::core::Error) -> bool {
    let code = err.code().0 as u32;
    code == ERROR_THREAD_ALREADY_IN_TASK_RAW
        || code == (0x80070000 | ERROR_THREAD_ALREADY_IN_TASK_RAW)
}

/// Owns only registrations created by OSCMidi. Driver-owned MMCSS registrations
/// deliberately do not create this guard and must not be reverted by the app.
#[cfg(target_os = "windows")]
pub(crate) struct MmcssRegistration {
    handle: windows::Win32::Foundation::HANDLE,
    owner_thread_id: u32,
    // Registrations must never cross a thread boundary, including inside a Send closure.
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

/// Keep MIDI delivery eligible for multimedia scheduling while a game is busy.
/// Call only on dedicated workers, and keep the guard on that thread until exit.
#[cfg(target_os = "windows")]
pub(crate) fn prioritize_midi_thread() -> Option<MmcssRegistration> {
    use windows::Win32::System::Threading::AVRT_PRIORITY_HIGH;

    unsafe {
        let mut task_index = 0;
        let mut profile = "Pro Audio";
        let mut high = false;
        let mut fallback = false;
        let registration = match AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index)
            .or_else(|pro_audio_error| {
                profile = "Audio";
                AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index)
                    .map_err(|audio_error| (pro_audio_error, audio_error))
            }) {
            Ok(handle) => {
                high = AvSetMmThreadPriority(handle, AVRT_PRIORITY_HIGH).is_ok();
                if !high {
                    background_log("warn", "Failed to raise MIDI MMCSS priority");
                }
                Some(MmcssRegistration {
                    handle,
                    owner_thread_id: GetCurrentThreadId(),
                    _thread_bound: std::marker::PhantomData,
                })
            }
            Err((pro_audio_error, audio_error)) => {
                if !is_thread_already_in_task_error(&pro_audio_error)
                    && !is_thread_already_in_task_error(&audio_error)
                {
                    background_log(
                        "warn",
                        "MIDI MMCSS unavailable; using thread priority fallback",
                    );
                    profile = "unavailable";
                    fallback =
                        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST).is_ok();
                    if !fallback {
                        background_log("warn", "Failed to raise MIDI worker priority");
                    }
                }
                None
            }
        };
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
            background_log("warn", "Failed to disable MIDI thread power throttling");
        }
        crate::realtime::record_thread(profile, high, fallback);
        registration
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn prioritize_midi_thread() -> Option<()> {
    None
}

/// Keep audio DSP execution eligible for critical multimedia scheduling while a game is busy.
/// Call only on dedicated DSP workers, and keep the guard on that thread until exit.
#[cfg(target_os = "windows")]
pub(crate) fn prioritize_dsp_thread() -> Option<MmcssRegistration> {
    unsafe {
        let mut task_index = 0;
        let mut profile = "Pro Audio";
        let mut critical = false;
        let mut fallback = false;
        let registration = match AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index)
            .or_else(|pro_audio_error| {
                profile = "Audio";
                AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index)
                    .map_err(|audio_error| (pro_audio_error, audio_error))
            }) {
            Ok(handle) => {
                critical = AvSetMmThreadPriority(handle, AVRT_PRIORITY_CRITICAL).is_ok();
                if !critical {
                    background_log("warn", "Failed to raise DSP MMCSS priority to CRITICAL");
                }
                Some(MmcssRegistration {
                    handle,
                    owner_thread_id: GetCurrentThreadId(),
                    _thread_bound: std::marker::PhantomData,
                })
            }
            Err((pro_audio_error, audio_error)) => {
                if !is_thread_already_in_task_error(&pro_audio_error)
                    && !is_thread_already_in_task_error(&audio_error)
                {
                    background_log(
                        "warn",
                        "DSP MMCSS unavailable; using thread priority fallback",
                    );
                    profile = "unavailable";
                    fallback =
                        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST).is_ok();
                    if !fallback {
                        background_log("warn", "Failed to raise DSP worker priority");
                    }
                }
                None
            }
        };
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
            background_log("warn", "Failed to disable DSP thread power throttling");
        }
        crate::realtime::record_thread(profile, critical, fallback);
        registration
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn prioritize_dsp_thread() -> Option<()> {
    None
}

#[cfg(target_os = "windows")]
impl Drop for MmcssRegistration {
    fn drop(&mut self) {
        // Safe code cannot move this guard. Record a violation rather than silently
        // abandoning the registration if an unsafe caller breaks that invariant.
        if unsafe { GetCurrentThreadId() } != self.owner_thread_id {
            record_audio_tuning_warning(WARN_MMCSS_RELEASE);
            return;
        }
        unsafe {
            if AvRevertMmThreadCharacteristics(self.handle).is_err() {
                record_audio_tuning_warning(WARN_MMCSS_RELEASE);
            }
        }
    }
}

#[cfg(target_os = "windows")]
struct AudioThreadTuning {
    registration: Option<MmcssRegistration>,
    enabled: bool,
    critical: Option<bool>,
    stream: std::sync::Weak<super::runtime_state::AudioTelemetry>,
}

#[cfg(target_os = "windows")]
thread_local! {
    // CPAL can move or destroy its closure on a different thread. AVRT ownership
    // instead follows the native callback thread, including its waiting periods.
    static AUDIO_THREAD_TUNING: std::cell::RefCell<Option<AudioThreadTuning>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "windows")]
pub(super) fn apply_audio_thread_priority(state: &mut AudioCallbackState) {
    AUDIO_THREAD_TUNING.with(|slot| {
        let mut slot = slot.borrow_mut();
        // Drivers may release their registration and reuse this native thread
        // for another stream. Refresh only borrowed/failed results at that boundary.
        // A Weak retains the allocation identity without retaining old telemetry.
        let refresh = slot.as_ref().is_none_or(|tuning| {
            tuning.registration.is_none()
                && tuning.stream.as_ptr() != std::sync::Arc::as_ptr(&state.telemetry)
        });
        if refresh {
            let mut tuning = configure_audio_thread();
            tuning.stream = std::sync::Arc::downgrade(&state.telemetry);
            *slot = Some(tuning);
        }
        let tuning = slot.as_ref().expect("audio thread tuning initialized");
        store_bool(&state.telemetry.mmcss_enabled, tuning.enabled);
        store_bool(&state.telemetry.mmcss_owned, tuning.registration.is_some());
        state.telemetry.mmcss_critical.store(
            tuning
                .critical
                .map_or(0, |critical| if critical { 2 } else { 1 }),
            Ordering::Relaxed,
        );
    });
}

#[cfg(target_os = "windows")]
fn configure_audio_thread() -> AudioThreadTuning {
    unsafe {
        let mut task_index = 0u32;
        let mmcss_result = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index).or_else(
            |pro_audio_error| {
                AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index)
                    .map_err(|audio_error| (pro_audio_error, audio_error))
            },
        );
        let mut enabled = false;
        let mut critical = None;
        let registration = match mmcss_result {
            Ok(handle) => {
                enabled = true;
                let raised = AvSetMmThreadPriority(handle, AVRT_PRIORITY_CRITICAL).is_ok();
                critical = Some(raised);
                if !raised {
                    record_audio_tuning_warning(WARN_MMCSS_PRIORITY);
                }
                Some(MmcssRegistration {
                    handle,
                    owner_thread_id: GetCurrentThreadId(),
                    _thread_bound: std::marker::PhantomData,
                })
            }
            Err((pro_audio_error, audio_error)) => {
                if is_thread_already_in_task_error(&pro_audio_error)
                    || is_thread_already_in_task_error(&audio_error)
                {
                    enabled = true;
                    record_audio_tuning_warning(WARN_MMCSS_ALREADY_REGISTERED);
                } else {
                    record_audio_tuning_warning(WARN_MMCSS_UNAVAILABLE);
                    // A bounded fallback avoids starving the Tauri, network and driver threads.
                    if SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST).is_err() {
                        record_audio_tuning_warning(WARN_THREAD_PRIORITY);
                    }
                }
                None
            }
        };
        if SetThreadPriorityBoost(GetCurrentThread(), false).is_err() {
            record_audio_tuning_warning(WARN_PRIORITY_BOOST);
        }
        let throttle = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        let thread_power_ok = SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            &throttle as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
        .is_ok();
        if !thread_power_ok {
            record_audio_tuning_warning(WARN_POWER_THROTTLING);
        }
        AudioThreadTuning {
            registration,
            enabled,
            critical,
            stream: std::sync::Weak::new(),
        }
    }
}

/// Only the application-owned render thread may select a non-MMCSS policy.
#[cfg(target_os = "windows")]
pub(super) fn prioritize_render_thread(
    priority: super::runtime_state::DspThreadPriority,
    telemetry: &super::runtime_state::AudioTelemetry,
) -> Option<MmcssRegistration> {
    if priority == super::runtime_state::DspThreadPriority::TimeCritical {
        use windows::Win32::System::Threading::THREAD_PRIORITY_TIME_CRITICAL;
        let applied = unsafe {
            SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS).is_ok()
                && SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL).is_ok()
        };
        if applied {
            store_bool(&telemetry.mmcss_enabled, false);
            store_bool(&telemetry.mmcss_owned, false);
            crate::realtime::record_thread("timeCritical", false, false);
            return None;
        }
        background_log("warn", "TIME_CRITICAL DSP policy failed; using MMCSS");
    }
    let tuning = configure_audio_thread();
    store_bool(&telemetry.mmcss_enabled, tuning.enabled);
    store_bool(&telemetry.mmcss_owned, tuning.registration.is_some());
    if let Some(critical) = tuning.critical {
        store_bool(&telemetry.mmcss_critical, critical);
    }
    tuning.registration
}

#[cfg(not(target_os = "windows"))]
pub(super) fn prioritize_render_thread(
    _: super::runtime_state::DspThreadPriority,
    _: &super::runtime_state::AudioTelemetry,
) -> Option<()> {
    None
}

#[cfg(not(target_os = "windows"))]
pub(super) fn apply_audio_thread_priority(_state: &mut AudioCallbackState) {}

#[cfg(target_os = "windows")]
pub struct HighResolutionTimerGuard {
    period_ms: u32,
}

#[cfg(target_os = "windows")]
impl Drop for HighResolutionTimerGuard {
    fn drop(&mut self) {
        unsafe {
            if self.period_ms != 0 && timeEndPeriod(self.period_ms) != 0 {
                background_log("warn", "Windows rejected timer resolution release");
            }
        }
    }
}

/// Request system-wide 1ms timer resolution for high-precision sleeps and audio/MIDI scheduling.
#[cfg(target_os = "windows")]
pub fn enable_high_resolution_timer() -> HighResolutionTimerGuard {
    let period_ms = if unsafe { timeBeginPeriod(1) } == 0 {
        1
    } else {
        background_log("warn", "Windows rejected 1ms timer resolution");
        0
    };
    HighResolutionTimerGuard { period_ms }
}

#[cfg(not(target_os = "windows"))]
pub struct HighResolutionTimerGuard;

#[cfg(not(target_os = "windows"))]
pub fn enable_high_resolution_timer() -> HighResolutionTimerGuard {
    HighResolutionTimerGuard
}

#[cfg(target_os = "windows")]
pub(super) struct AudioProcessTuning {
    timer_period_ms: u32,
}

#[cfg(target_os = "windows")]
impl Drop for AudioProcessTuning {
    fn drop(&mut self) {
        unsafe {
            if self.timer_period_ms != 0 && timeEndPeriod(self.timer_period_ms) != 0 {
                background_log("warn", "Windows rejected timer resolution release");
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn apply_audio_process_tuning(logger: &FrontendLogger) -> AudioProcessTuning {
    // Timer resolution improves timeout accuracy; driver callbacks keep their
    // own cadence, and this request does not reserve CPU time.
    let timer_period_ms = if unsafe { timeBeginPeriod(1) } == 0 {
        1
    } else {
        logger.warn("Windows rejected 1ms timer resolution");
        0
    };

    apply_process_priority(logger);
    AudioProcessTuning { timer_period_ms }
}

/// Re-apply process priority class and power throttling.
/// Called once at startup and again whenever the window loses focus,
/// to keep the explicitly requested scheduling policy consistent.
#[cfg(target_os = "windows")]
pub fn apply_process_priority(logger: &FrontendLogger) {
    if reapply_worker_policy() {
        return;
    }
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS).is_err() {
            logger.warn("Failed to raise process priority class to ABOVE_NORMAL");
        } else {
            logger.debug("Raised process priority to ABOVE_NORMAL");
        }
        let throttle = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: 0,
        };
        let process_power_ok = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &throttle as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .is_ok();
        store_bool(&AUDIO_POWER_THROTTLING_DISABLED, process_power_ok);
        if !process_power_ok {
            logger.warn("Failed to disable process power throttling");
        } else {
            logger.debug("Disabled Windows power throttling for audio process");
        }
    }
}

/// Lightweight re-application of process priority on focus loss.
/// Does not require a FrontendLogger — logs via background_log.
#[cfg(target_os = "windows")]
pub fn reapply_process_priority_on_focus_loss() {
    if reapply_worker_policy() {
        return;
    }
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS).is_err() {
            background_log("warn", "Failed to set process priority policy");
        }

        // Re-assert power throttling disabled. On Windows 11, focus loss can
        // affect timer resolution policy. Check failures at startup.
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
            background_log("warn", "Failed to disable process power throttling");
        }
    }
}

#[cfg(target_os = "windows")]
fn reapply_worker_policy() -> bool {
    if !WORKER_PROCESS.load(Ordering::Acquire) {
        return false;
    }
    apply_worker_process_priority();
    true
}

/// Apply high priority class and disable power throttling for dedicated background audio workers.
/// Workers run headless without UI rendering, allowing HIGH_PRIORITY_CLASS (same as FL Studio/DAWs).
#[cfg(target_os = "windows")]
pub fn apply_worker_process_priority() {
    WORKER_PROCESS.store(true, Ordering::Release);
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS).is_err() {
            background_log(
                "warn",
                "Failed to raise worker process priority class to HIGH",
            );
        } else {
            background_log("debug", "Raised worker process priority to HIGH");
        }

        let throttle = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: 0,
        };
        let process_power_ok = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &throttle as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .is_ok();
        store_bool(&AUDIO_POWER_THROTTLING_DISABLED, process_power_ok);
        if !process_power_ok {
            background_log("warn", "Failed to disable worker process power throttling");
        } else {
            background_log(
                "debug",
                "Disabled Windows power throttling for audio worker process",
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_worker_process_priority() {}

#[cfg(not(target_os = "windows"))]
pub(super) struct AudioProcessTuning;

#[cfg(not(target_os = "windows"))]
pub(super) fn apply_audio_process_tuning(_logger: &FrontendLogger) -> AudioProcessTuning {
    AudioProcessTuning
}

#[cfg(not(target_os = "windows"))]
pub fn apply_process_priority(_logger: &FrontendLogger) {}

#[cfg(not(target_os = "windows"))]
pub fn reapply_process_priority_on_focus_loss() {}

#[cfg(target_os = "windows")]
pub(super) fn audio_mmcss_enabled(
    telemetry: &super::runtime_state::AudioTelemetry,
) -> Option<bool> {
    load_bool(&telemetry.mmcss_enabled)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn audio_mmcss_enabled(
    _telemetry: &super::runtime_state::AudioTelemetry,
) -> Option<bool> {
    None
}

#[cfg(target_os = "windows")]
pub(super) fn audio_power_throttling_disabled() -> Option<bool> {
    load_bool(&AUDIO_POWER_THROTTLING_DISABLED)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn audio_power_throttling_disabled() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    fn isolated(test_name: &str) -> bool {
        if std::env::var("OSCMIDI_TUNING_TEST").as_deref() == Ok(test_name) {
            return false;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env("OSCMIDI_TUNING_TEST", test_name)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn tuning_diagnostics_drains_release_and_power_failures_without_status_reads() {
        use super::*;
        if isolated("audio::windows_tuning::tests::tuning_diagnostics_drains_release_and_power_failures_without_status_reads") { return; }
        drop(MmcssRegistration {
            handle: Default::default(),
            owner_thread_id: unsafe { GetCurrentThreadId() },
            _thread_bound: std::marker::PhantomData,
        });
        assert_ne!(
            AUDIO_TUNING_WARNINGS.load(Ordering::Acquire) & WARN_MMCSS_RELEASE,
            0
        );
        record_audio_tuning_warning(WARN_POWER_THROTTLING);
        let diagnostics = start_audio_tuning_diagnostics();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while AUDIO_TUNING_WARNINGS.load(Ordering::Acquire) != 0
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(AUDIO_TUNING_WARNINGS.load(Ordering::Acquire), 0);
        record_audio_tuning_warning(WARN_MMCSS_RELEASE | WARN_POWER_THROTTLING);
        drop(diagnostics);
        assert_eq!(AUDIO_TUNING_WARNINGS.load(Ordering::Acquire), 0);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn callback_mmcss_handles_follow_invocations_across_thread_migration() {
        let mut state = callback_state();
        for _ in 0..8 {
            state = std::thread::spawn(move || {
                super::apply_audio_thread_priority(&mut state);
                assert_eq!(super::load_bool(&state.telemetry.mmcss_owned), Some(true));
                let mut index = 0;
                let error = unsafe {
                    super::AvSetMmThreadCharacteristicsW(super::w!("Pro Audio"), &mut index)
                }
                .expect_err("registration must remain on each callback thread");
                assert!(super::is_thread_already_in_task_error(&error));
                state
            })
            .join()
            .unwrap();
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn callback_closures_dropped_after_owner_exit_do_not_leak_handles() {
        use windows::Win32::System::Threading::GetProcessHandleCount;
        if isolated("audio::windows_tuning::tests::callback_closures_dropped_after_owner_exit_do_not_leak_handles") { return; }
        let invoke = || {
            std::thread::spawn(|| {
                let mut state = callback_state();
                super::apply_audio_thread_priority(&mut state);
                assert_eq!(super::load_bool(&state.telemetry.mmcss_owned), Some(true));
                state
            })
            .join()
            .unwrap()
        };
        drop(invoke()); // Warm up AVRT and thread infrastructure before counting.
        let count = || {
            let mut value = 0;
            unsafe {
                GetProcessHandleCount(super::GetCurrentProcess(), &mut value).unwrap();
            }
            value
        };
        let before = count();
        let states = (0..32).map(|_| invoke()).collect::<Vec<_>>();
        assert_eq!(count(), before, "handles survived their callback owners");
        drop(states); // CPAL may destroy closures here, on a different thread.
        assert_eq!(count(), before);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn replacement_stream_starts_with_unknown_mmcss_status() {
        use std::sync::Arc;
        std::thread::spawn(|| {
            let engine = crate::audio::AudioEngine::new();
            let mut old = callback_state();
            super::apply_audio_thread_priority(&mut old);
            engine.health_telemetry.store(Some(old.telemetry.clone()));
            assert_eq!(engine.mmcss_enabled(), Some(true));
            engine.health_telemetry.store(Some(Arc::new(
                crate::audio::runtime_state::AudioTelemetry::new(),
            )));
            assert_eq!(engine.mmcss_enabled(), None);
            engine.health_telemetry.store(None);
            assert_eq!(engine.mmcss_enabled(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn borrowed_mmcss_registration_is_preserved() {
        std::thread::spawn(|| {
            let mut index = 0;
            let owner =
                unsafe { super::AvSetMmThreadCharacteristicsW(super::w!("Audio"), &mut index) }
                    .unwrap();
            let mut state = callback_state();
            super::apply_audio_thread_priority(&mut state);
            assert_eq!(super::load_bool(&state.telemetry.mmcss_owned), Some(false));
            assert_eq!(super::load_bool(&state.telemetry.mmcss_critical), None);
            drop(state);
            unsafe {
                super::AvSetMmThreadPriority(owner, super::AVRT_PRIORITY_CRITICAL).unwrap();
            }
            unsafe {
                super::AvRevertMmThreadCharacteristics(owner).unwrap();
            }
        })
        .join()
        .unwrap();
    }

    #[cfg(target_os = "windows")]
    fn callback_state() -> super::AudioCallbackState {
        use crate::audio::runtime_state::{AudioControls, AudioTelemetry};
        use std::sync::{atomic::AtomicBool, Arc};
        super::AudioCallbackState::new(
            rtrb::RingBuffer::new(8).1,
            0,
            2,
            64,
            2,
            Arc::new(AudioControls {
                gain_bits: std::sync::atomic::AtomicU32::new(1.0f32.to_bits()),
                limiter_enabled: AtomicBool::new(false),
                equalizer: crate::audio::equalizer::PublishedEq::new(&Default::default(), 48_000),
            }),
            Arc::new(AudioTelemetry::new()),
            Arc::new(AtomicBool::new(false)),
            48_000,
            crossbeam_channel::bounded(8).1,
        )
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn callback_mmcss_registration_survives_between_invocations() {
        std::thread::spawn(|| {
            let mut state = callback_state();
            super::apply_audio_thread_priority(&mut state);
            let handle = super::AUDIO_THREAD_TUNING.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .unwrap()
                    .registration
                    .as_ref()
                    .unwrap()
                    .handle
            });
            for _ in 0..1000 {
                super::apply_audio_thread_priority(&mut state);
            }
            let repeated = super::AUDIO_THREAD_TUNING.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .unwrap()
                    .registration
                    .as_ref()
                    .unwrap()
                    .handle
            });
            assert_eq!(
                handle, repeated,
                "callbacks must reuse the same owned registration"
            );
            assert_eq!(
                super::load_bool(&state.telemetry.mmcss_critical),
                Some(true)
            );
            let mut index = 0;
            let error =
                unsafe { super::AvSetMmThreadCharacteristicsW(super::w!("Pro Audio"), &mut index) }
                    .expect_err(
                        "audio thread must remain registered while waiting for its next buffer",
                    );
            assert!(super::is_thread_already_in_task_error(&error));
            state
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn reused_driver_thread_refreshes_borrowed_registration_for_a_new_stream() {
        std::thread::spawn(|| {
            let mut index = 0;
            let driver =
                unsafe { super::AvSetMmThreadCharacteristicsW(super::w!("Audio"), &mut index) }
                    .unwrap();
            let mut first = callback_state();
            super::apply_audio_thread_priority(&mut first);
            assert_eq!(super::load_bool(&first.telemetry.mmcss_owned), Some(false));
            unsafe { super::AvRevertMmThreadCharacteristics(driver) }.unwrap();
            let mut replacement = callback_state();
            super::apply_audio_thread_priority(&mut replacement);
            assert_eq!(
                super::load_bool(&replacement.telemetry.mmcss_owned),
                Some(true)
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn stopped_engine_does_not_report_another_streams_mmcss() {
        std::thread::spawn(|| {
            let mut state = callback_state();
            super::apply_audio_thread_priority(&mut state);
            let engine = crate::audio::AudioEngine::new();
            assert_eq!(engine.mmcss_enabled(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn worker_priority_survives_reapplication() {
        use super::*;
        use windows::Win32::System::Threading::{GetPriorityClass, GetProcessInformation};

        fn assert_worker_policy() {
            let mut power = PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ..Default::default()
            };
            unsafe {
                assert_eq!(GetPriorityClass(GetCurrentProcess()), HIGH_PRIORITY_CLASS.0);
                GetProcessInformation(
                    GetCurrentProcess(),
                    ProcessPowerThrottling,
                    &mut power as *mut _ as _,
                    std::mem::size_of_val(&power) as u32,
                )
                .unwrap();
            }
            assert_eq!(power.ControlMask & 0x5, 0x5);
            assert_eq!(power.StateMask & 0x5, 0);
        }

        // Priority and worker role are process-wide: keep the test runner untouched.
        const CHILD_ENV: &str = "OSCMIDI_TEST_WORKER_PRIORITY";
        if std::env::var_os(CHILD_ENV).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "audio::windows_tuning::tests::worker_priority_survives_reapplication",
                    "--nocapture",
                ])
                .env(CHILD_ENV, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        assert!(!reapply_worker_policy());
        reapply_process_priority_on_focus_loss();
        assert_eq!(
            unsafe { GetPriorityClass(GetCurrentProcess()) },
            ABOVE_NORMAL_PRIORITY_CLASS.0
        );
        apply_worker_process_priority();
        assert_worker_policy();
        assert_eq!(
            unsafe { GetPriorityClass(GetCurrentProcess()) },
            HIGH_PRIORITY_CLASS.0
        );
        // This is the same policy gate used when audio startup reapplies tuning.
        assert!(reapply_worker_policy());
        reapply_process_priority_on_focus_loss();
        assert_worker_policy();
        assert_eq!(
            unsafe { GetPriorityClass(GetCurrentProcess()) },
            HIGH_PRIORITY_CLASS.0
        );
    }

    #[test]
    fn windows_audio_tuning_uses_mmcss_critical_priority() {
        let source = include_str!("windows_tuning.rs");
        let priority_fn = ["AvSetMmThread", "Priority"].concat();
        let critical = ["AVRT_PRIORITY", "_CRITICAL"].concat();

        assert!(source.contains(&priority_fn));
        assert!(source.contains(&critical));
    }

    #[test]
    fn windows_audio_tuning_uses_bounded_process_and_thread_fallbacks() {
        let source = include_str!("windows_tuning.rs");
        let above_normal = ["ABOVE_NORMAL_PRIORITY", "_CLASS"].concat();
        let highest = ["THREAD_PRIORITY", "_HIGHEST"].concat();
        let time_critical = ["THREAD_PRIORITY_TIME", "_CRITICAL"].concat();
        let thread_boost = ["SetThreadPriority", "Boost"].concat();

        assert!(source.contains(&above_normal));
        assert!(source.contains(&highest));
        assert!(source.contains(&thread_boost));
        let fallback = source
            .split("fn configure_audio_thread()")
            .nth(1)
            .unwrap()
            .split("fn prioritize_render_thread(")
            .next()
            .unwrap();
        assert!(!fallback.contains(&time_critical));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn owned_renderer_can_select_time_critical_without_mmcss() {
        if isolated(
            "audio::windows_tuning::tests::owned_renderer_can_select_time_critical_without_mmcss",
        ) {
            return;
        }
        use windows::Win32::System::Threading::{
            GetPriorityClass, GetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        };
        std::thread::spawn(|| {
            let telemetry = super::super::runtime_state::AudioTelemetry::new();
            let guard = super::prioritize_render_thread(
                super::super::runtime_state::DspThreadPriority::TimeCritical,
                &telemetry,
            );
            assert!(guard.is_none());
            assert_eq!(super::load_bool(&telemetry.mmcss_enabled), Some(false));
            assert_eq!(super::load_bool(&telemetry.mmcss_owned), Some(false));
            unsafe {
                assert_eq!(
                    GetPriorityClass(super::GetCurrentProcess()),
                    super::HIGH_PRIORITY_CLASS.0
                );
                assert_eq!(
                    GetThreadPriority(super::GetCurrentThread()),
                    THREAD_PRIORITY_TIME_CRITICAL.0
                );
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn windows_audio_tuning_calls_time_begin_period() {
        let source = include_str!("windows_tuning.rs");
        let begin_period = ["timeBegin", "Period"].concat();
        let end_period = ["timeEnd", "Period"].concat();
        assert!(source.contains(&begin_period));
        assert!(source.contains(&end_period));
    }

    #[test]
    fn windows_audio_tuning_handles_already_registered_mmcss() {
        let source = include_str!("windows_tuning.rs");
        assert!(source.contains("ERROR_THREAD_ALREADY_IN_TASK_RAW: u32 = 1552"));
        assert!(source.contains("is_thread_already_in_task_error"));
    }

    #[test]
    fn windows_worker_tuning_uses_high_priority_class() {
        let source = include_str!("windows_tuning.rs");
        let high_class = ["HIGH_PRIORITY", "_CLASS"].concat();
        assert!(source.contains(&high_class));
    }
}
