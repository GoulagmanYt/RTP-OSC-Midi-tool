use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::{background_log, FrontendLogger};

use super::runtime_state::AudioCallbackState;

#[cfg(target_os = "windows")]
use std::sync::atomic::AtomicU8;

#[cfg(target_os = "windows")]
static AUDIO_MMCSS_ENABLED: AtomicU8 = AtomicU8::new(0);
#[cfg(target_os = "windows")]
static AUDIO_POWER_THROTTLING_DISABLED: AtomicU8 = AtomicU8::new(0);

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
use windows::Win32::Media::timeBeginPeriod;
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority, GetCurrentProcess, GetCurrentThread,
    ProcessPowerThrottling, SetPriorityClass, SetProcessInformation, SetThreadInformation,
    SetThreadPriority, SetThreadPriorityBoost, ThreadPowerThrottling, ABOVE_NORMAL_PRIORITY_CLASS,
    AVRT_PRIORITY_CRITICAL, HIGH_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY_TIME_CRITICAL,
};

/// Windows error code for ERROR_ALREADY_REGISTERED (0x800704DA = HRESULT, raw = 1242).
/// When the ASIO driver (e.g. Voicemeeter) has already registered the callback thread
/// with MMCSS, our call fails with this error — but the thread IS getting MMCSS scheduling.
#[cfg(target_os = "windows")]
const ERROR_ALREADY_REGISTERED_RAW: u32 = 1242;

#[cfg(target_os = "windows")]
fn is_already_registered_error(err: &windows::core::Error) -> bool {
    // The windows crate wraps WIN32 errors as HRESULT. Check both the raw code and
    // the HRESULT-wrapped form.
    let code = err.code().0 as u32;
    // Raw WIN32 error code
    code == ERROR_ALREADY_REGISTERED_RAW
        // HRESULT form: FACILITY_WIN32 (7) << 16 | 0x80000000 | error_code
        || code == (0x80070000 | ERROR_ALREADY_REGISTERED_RAW)
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
                if let Err(error) = AvSetMmThreadPriority(handle, AVRT_PRIORITY_CRITICAL) {
                    background_log(
                        "warn",
                        format!("Failed to raise MMCSS audio thread priority: {error}"),
                    );
                }
            }
            Err((pro_audio_error, audio_error)) => {
                // Check if the thread is ALREADY registered by the ASIO driver.
                // This is common with Voicemeeter, ASIO4ALL, etc. In this case,
                // the thread IS getting MMCSS scheduling from the driver.
                if is_already_registered_error(&pro_audio_error)
                    || is_already_registered_error(&audio_error)
                {
                    store_bool(&AUDIO_MMCSS_ENABLED, true);
                    background_log(
                        "info",
                        "MMCSS: audio thread already registered by ASIO driver (OK)",
                    );
                } else {
                    store_bool(&AUDIO_MMCSS_ENABLED, false);
                    background_log(
                        "warn",
                        format!(
                            "Failed to enable MMCSS for audio thread (Pro Audio: {pro_audio_error}; Audio: {audio_error})"
                        ),
                    );
                    // Fallback: set thread to TIME_CRITICAL priority manually.
                    if let Err(error) =
                        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL)
                    {
                        background_log(
                            "warn",
                            format!(
                                "Failed to apply TIME_CRITICAL fallback to audio thread: {error}"
                            ),
                        );
                    }
                }
            }
        }
        if let Err(error) = SetThreadPriorityBoost(GetCurrentThread(), false) {
            background_log(
                "warn",
                format!("Failed to enable dynamic priority boosts for audio thread: {error}"),
            );
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
            background_log(
                "warn",
                "Failed to disable thread power throttling for audio thread",
            );
        }
    }
    state.mmcss_applied = true;
}

#[cfg(not(target_os = "windows"))]
pub(super) fn apply_audio_thread_priority(_state: &mut AudioCallbackState) {}

#[cfg(target_os = "windows")]
pub(super) fn apply_audio_process_tuning(logger: &FrontendLogger) {
    static PROCESS_TUNING_APPLIED: AtomicBool = AtomicBool::new(false);
    if PROCESS_TUNING_APPLIED.swap(true, Ordering::Relaxed) {
        return;
    }

    // Request 1ms timer resolution. This is CRITICAL for audio apps — without it,
    // Windows uses a 15.6ms default resolution and coarsens scheduling when the app
    // loses focus, causing the ASIO callback to miss deadlines.
    unsafe {
        timeBeginPeriod(1);
    }
    logger.debug("Requested 1ms system timer resolution (timeBeginPeriod)");

    apply_process_priority(logger);
}

/// Re-apply process priority class and power throttling.
/// Called once at startup and again whenever the window loses focus,
/// to counteract Windows downgrading the process scheduling.
#[cfg(target_os = "windows")]
pub fn apply_process_priority(logger: &FrontendLogger) {
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS).is_err() {
            if SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS).is_err() {
                logger.warn("Failed to raise process priority class for audio stability");
            } else {
                logger.warn("HIGH priority failed; raised process priority to ABOVE_NORMAL");
            }
        } else {
            logger.debug("Raised process priority to HIGH");
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
        // Re-assert HIGH_PRIORITY_CLASS. Windows may have adjusted the effective
        // scheduling parameters when the process lost the foreground window.
        if SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS).is_err() {
            let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
        }

        // Re-assert power throttling disabled. On Windows 11, focus loss can
        // trigger EcoQoS evaluation that we need to counteract.
        let throttle = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: 0,
        };
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &throttle as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub(super) fn apply_audio_process_tuning(_logger: &FrontendLogger) {}

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
    fn windows_audio_tuning_uses_high_process_and_time_critical_fallback() {
        let source = include_str!("windows_tuning.rs");
        let high_class = ["HIGH_PRIORITY", "_CLASS"].concat();
        let time_critical = ["THREAD_PRIORITY", "_TIME_CRITICAL"].concat();
        let thread_boost = ["SetThreadPriority", "Boost"].concat();

        assert!(source.contains(&high_class));
        assert!(source.contains(&time_critical));
        assert!(source.contains(&thread_boost));
    }

    #[test]
    fn windows_audio_tuning_calls_time_begin_period() {
        let source = include_str!("windows_tuning.rs");
        let begin_period = ["timeBegin", "Period"].concat();
        assert!(source.contains(&begin_period));
    }

    #[test]
    fn windows_audio_tuning_handles_already_registered_mmcss() {
        let source = include_str!("windows_tuning.rs");
        let already_registered = ["already_registered", "_error"].concat();
        assert!(source.contains(&already_registered));
    }
}
