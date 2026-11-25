//! Opt-in timing output (`LWFM_TRACE=1`) to stderr and the debugger, used to
//! check the startup and listing budgets.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lwfm_win::sys;

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
    sys::debug_output(&line);
}

/// Time since the process was created, as recorded by the kernel.
pub fn since_process_start() -> Duration {
    sys::since_process_start()
}

pub fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}
