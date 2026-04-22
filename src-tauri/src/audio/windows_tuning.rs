use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::{background_log, FrontendLogger};

use super::runtime_state::AudioCallbackState;

#[cfg(target_os = "windows")]
use windows::core::w;
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

#[cfg(target_os = "windows")]
pub(super) fn apply_audio_thread_priority(state: &mut AudioCallbackState) {
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
pub(super) fn apply_audio_thread_priority(_state: &mut AudioCallbackState) {}

#[cfg(target_os = "windows")]
pub(super) fn apply_audio_process_tuning(logger: &FrontendLogger) {
    static PROCESS_TUNING_APPLIED: AtomicBool = AtomicBool::new(false);
    if PROCESS_TUNING_APPLIED.swap(true, Ordering::Relaxed) {
        return;
    }

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
pub(super) fn apply_audio_process_tuning(_logger: &FrontendLogger) {}
