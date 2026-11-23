//! Opt-in timing output (`LWFM_TRACE=1`) to stderr and the debugger, used to
//! check the startup and listing budgets.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::core::PCWSTR;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let on = std::env::var_os("LWFM_TRACE").is_some_and(|v| v == "1");
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Logs the message built by `f` when tracing is enabled.
pub fn log(f: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let line = format!("[lwfm] {}\n", f());
    eprint!("{line}");
    let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
    unsafe { OutputDebugStringW(PCWSTR(wide.as_ptr())) };
}

fn ticks(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// Time since the process was created, as recorded by the kernel.
pub fn since_process_start() -> Duration {
    let (mut created, mut exit, mut kernel, mut user) = Default::default();
    unsafe {
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .is_err()
        {
            return Duration::ZERO;
        }
        let now = GetSystemTimePreciseAsFileTime();
        Duration::from_nanos(ticks(now).saturating_sub(ticks(created)) * 100)
    }
}

pub fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}
