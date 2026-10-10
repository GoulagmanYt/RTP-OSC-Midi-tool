use std::sync::atomic::Ordering;

use crate::logger::{background_log, FrontendLogger};

use super::runtime_state::AudioCallbackState;

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicU32, AtomicU8};

#[cfg(target_os = "windows")]
static AUDIO_MMCSS_ENABLED: AtomicU8 = AtomicU8::new(0);
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
            "MMCSS: audio thread already registered by ASIO driver (OK)",
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
        // MMCSS registrations are thread-affine. CPAL is allowed to destroy its
        // callback closure from another thread, in which case Windows releases
        // the registration when the callback thread terminates.
        if unsafe { GetCurrentThreadId() } != self.owner_thread_id {
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
pub(super) fn apply_audio_thread_priority(state: &mut AudioCallbackState) {
    if state.mmcss_applied {
        return;
    }
    unsafe {
        let mut task_index = 0u32;
        let mmcss_result = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index).or_else(
            |pro_audio_error| {
                AvSetMmThreadCharacteristicsW(w!("Audio"), &mut task_index)
                    .map_err(|audio_error| (pro_audio_error, audio_error))
            },
        );
        match mmcss_result {
            Ok(handle) => {
                store_bool(&AUDIO_MMCSS_ENABLED, true);
                if AvSetMmThreadPriority(handle, AVRT_PRIORITY_CRITICAL).is_err() {
                    record_audio_tuning_warning(WARN_MMCSS_PRIORITY);
                }
                state.mmcss_registration = Some(MmcssRegistration {
                    handle,
                    owner_thread_id: GetCurrentThreadId(),
                });
            }
            Err((pro_audio_error, audio_error)) => {
                if is_thread_already_in_task_error(&pro_audio_error)
                    || is_thread_already_in_task_error(&audio_error)
                {
                    store_bool(&AUDIO_MMCSS_ENABLED, true);
                    record_audio_tuning_warning(WARN_MMCSS_ALREADY_REGISTERED);
                } else {
                    store_bool(&AUDIO_MMCSS_ENABLED, false);
                    record_audio_tuning_warning(WARN_MMCSS_UNAVAILABLE);
                    // A bounded fallback avoids starving the Tauri, network and driver threads.
                    if SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST).is_err() {
                        record_audio_tuning_warning(WARN_THREAD_PRIORITY);
                    }
                }
            }
        }
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
    }
    state.mmcss_applied = true;
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

/// Apply high priority class and disable power throttling for dedicated background audio workers.
/// Workers run headless without UI rendering, allowing HIGH_PRIORITY_CLASS (same as FL Studio/DAWs).
#[cfg(target_os = "windows")]
pub fn apply_worker_process_priority() {
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
pub(super) fn audio_mmcss_enabled() -> Option<bool> {
    load_bool(&AUDIO_MMCSS_ENABLED)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn audio_mmcss_enabled() -> Option<bool> {
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
        assert!(!source.contains(&time_critical));
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
