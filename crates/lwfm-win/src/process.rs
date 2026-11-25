//! Starting other programs. Everything here blocks or may block, so it
//! belongs on worker threads.

use std::ffi::OsString;
use std::path::Path;

use windows::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    GetExitCodeProcess, INFINITE, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, PWSTR};

use crate::error::{Error, ErrorKind};
use crate::handle::{ComApartment, OwnedHandle};
use crate::num::size_u32;
use crate::wide::wide;
use crate::window::WindowToken;

/// A started process.
pub struct Child {
    process: OwnedHandle,
}

impl Child {
    /// Waits for the process to end and returns its exit code.
    ///
    /// # Errors
    ///
    /// Fails when the exit code cannot be read.
    pub fn wait(self) -> Result<u32, Error> {
        // SAFETY: the process handle is valid and owned by `self`.
        unsafe { WaitForSingleObject(self.process.raw(), INFINITE) };
        let mut code = 0u32;
        // SAFETY: the process handle is valid and `code` is writable.
        unsafe { GetExitCodeProcess(self.process.raw(), &raw mut code) }?;
        Ok(code)
    }
}

/// Runs `line` through the command interpreter (`%ComSpec% /d /s /c`) with
/// `dir` as working directory. With `console` the process gets its own new
/// console window, otherwise none.
///
/// # Errors
///
/// Fails when the interpreter cannot be started.
pub fn spawn_cmd(line: &str, dir: &Path, console: bool) -> Result<Child, Error> {
    let mut command = OsString::from("\"");
    command.push(std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into()));
    command.push("\" /d /s /c \"");
    command.push(line);
    command.push("\"");
    let mut command = wide(command);
    let dir = wide(dir);
    let startup = STARTUPINFOW {
        cb: size_u32::<STARTUPINFOW>(),
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    let flags = CREATE_UNICODE_ENVIRONMENT
        | if console {
            CREATE_NEW_CONSOLE
        } else {
            CREATE_NO_WINDOW
        };
    // SAFETY: `command` is a writable NUL-terminated buffer (the call may
    // modify it), `dir` is NUL-terminated, and `startup` and `info` are
    // valid for the duration of the call.
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            flags,
            None,
            PCWSTR(dir.as_ptr()),
            &raw const startup,
            &raw mut info,
        )
    }?;
    // The primary thread's handle is not needed; closing it here keeps the
    // thread object from outliving the thread.
    drop(OwnedHandle::new(info.hThread));
    Ok(Child {
        process: OwnedHandle::new(info.hProcess),
    })
}

/// Opens `file` with its Windows association, with `dir` as working
/// directory and `owner` as the parent of any dialog the shell shows.
/// Returns once the shell has started the program. Cancelling such a dialog
/// is not an error.
///
/// # Errors
///
/// Fails when the shell cannot open the file.
pub fn shell_open(owner: WindowToken, file: &Path, dir: &Path) -> Result<(), Error> {
    let _com = ComApartment::enter();
    let file = wide(file);
    let dir = wide(dir);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_u32::<SHELLEXECUTEINFOW>(),
        fMask: SEE_MASK_NOASYNC,
        hwnd: owner.hwnd(),
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is fully initialized with its size set, and the strings
    // it points to are NUL-terminated and outlive the call.
    match unsafe { ShellExecuteExW(&raw mut info) }.map_err(Error::from) {
        Err(e) if e.kind() != ErrorKind::Cancelled => Err(e),
        _ => Ok(()),
    }
}
