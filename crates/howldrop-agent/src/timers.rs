//! Windows: timers to the millisecond, for AWDL's 16 TU slots.

/// Timers to the millisecond for this process. Windows wakes them on its
/// 15.6 ms tick otherwise, most of an AWDL slot (16 TU) late, and the node's
/// frames, the peer's TCP ACKs among them, with them. Windows 11 also lets
/// a program without a window in sight fall back to the coarse tick unless
/// it asks not to.
#[allow(unsafe_code)]
pub fn fine_timers() {
    use windows_sys::Win32::Media::timeBeginPeriod;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: plain Win32 calls on this process; `state` outlives the call
    // and its size is given. The period lasts until the process ends.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        timeBeginPeriod(1);
    }
}
