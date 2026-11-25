//! Process-level helpers: console attachment, debugger output and timing.

use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    SetStdHandle,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::core::{PCWSTR, w};

use crate::num::join_u64;
use crate::wide::wide;

/// Sends stdout and stderr to the terminal that started this GUI-subsystem
/// process, unless stdout is already redirected.
pub fn attach_parent_console() {
    // SAFETY: no pointer arguments.
    let redirected =
        unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }.is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
    if redirected {
        return;
    }
    // SAFETY: no pointer arguments.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return;
    }
    // SAFETY: the file name is a NUL-terminated literal.
    let console = unsafe {
        CreateFileW(
            w!("CONOUT$"),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    // The handle is deliberately not closed: it is the process's stdout and
    // stderr from here on.
    if let Ok(console) = console {
        // SAFETY: `console` is a valid handle that stays open.
        let _ = unsafe { SetStdHandle(STD_OUTPUT_HANDLE, console) };
        // SAFETY: as above.
        let _ = unsafe { SetStdHandle(STD_ERROR_HANDLE, console) };
    }
}

/// Writes `text` to an attached debugger.
pub fn debug_output(text: &str) {
    let text = wide(text);
    // SAFETY: `text` is NUL-terminated and outlives the call.
    unsafe { OutputDebugStringW(PCWSTR(text.as_ptr())) };
}

/// Time since the process was created, as recorded by the kernel. Zero when
/// the creation time cannot be read.
#[must_use]
pub fn since_process_start() -> Duration {
    let ticks = |ft: FILETIME| join_u64(ft.dwHighDateTime, ft.dwLowDateTime);
    let (mut created, mut exit, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo handle needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: all four out parameters are writable FILETIME values.
    let read = unsafe {
        GetProcessTimes(
            process,
            &raw mut created,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if read.is_err() {
        return Duration::ZERO;
    }
    // SAFETY: no arguments.
    let now = unsafe { GetSystemTimePreciseAsFileTime() };
    Duration::from_nanos(ticks(now).saturating_sub(ticks(created)) * 100)
}
