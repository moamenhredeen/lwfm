//! The error type of every fallible call in this crate.

use std::fmt;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BAD_NET_NAME, ERROR_BAD_NETPATH, ERROR_CANCELLED, ERROR_DIRECTORY,
    ERROR_FILE_NOT_FOUND, ERROR_HOST_UNREACHABLE, ERROR_INVALID_FUNCTION, ERROR_INVALID_LEVEL,
    ERROR_INVALID_NAME, ERROR_INVALID_PARAMETER, ERROR_NETWORK_UNREACHABLE, ERROR_NOT_READY,
    ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND, ERROR_SEM_TIMEOUT, ERROR_UNEXP_NET_ERR, WIN32_ERROR,
};
use windows::core::HRESULT;

/// What went wrong, as far as callers need to tell cases apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    NotFound,
    AccessDenied,
    NotADirectory,
    NotReady,
    NetworkUnreachable,
    /// The file system or device does not support the request.
    Unsupported,
    /// The user cancelled the operation.
    Cancelled,
    Other,
}

/// A failed system call. Displays as the system's message for the error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Error {
    hresult: i32,
}

impl Error {
    /// The error for a Win32 error code (a `GetLastError` value).
    #[must_use]
    pub fn from_code(code: u32) -> Error {
        Error::from_win32(WIN32_ERROR(code))
    }

    pub(crate) fn from_win32(code: WIN32_ERROR) -> Error {
        Error {
            hresult: code.to_hresult().0,
        }
    }

    pub(crate) fn last() -> Error {
        Error::from(windows::core::Error::from_thread())
    }

    fn win32(self) -> Option<WIN32_ERROR> {
        WIN32_ERROR::from_error(&windows::core::Error::from_hresult(HRESULT(self.hresult)))
    }

    pub(crate) fn is(self, code: WIN32_ERROR) -> bool {
        self.win32() == Some(code)
    }

    #[must_use]
    pub fn kind(self) -> ErrorKind {
        match self.win32() {
            Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_INVALID_NAME) => {
                ErrorKind::NotFound
            }
            Some(ERROR_ACCESS_DENIED) => ErrorKind::AccessDenied,
            Some(ERROR_DIRECTORY) => ErrorKind::NotADirectory,
            Some(ERROR_NOT_READY) => ErrorKind::NotReady,
            Some(
                ERROR_BAD_NETPATH
                | ERROR_BAD_NET_NAME
                | ERROR_NETWORK_UNREACHABLE
                | ERROR_HOST_UNREACHABLE
                | ERROR_UNEXP_NET_ERR
                | ERROR_SEM_TIMEOUT,
            ) => ErrorKind::NetworkUnreachable,
            Some(
                ERROR_INVALID_PARAMETER
                | ERROR_INVALID_FUNCTION
                | ERROR_NOT_SUPPORTED
                | ERROR_INVALID_LEVEL,
            ) => ErrorKind::Unsupported,
            Some(ERROR_CANCELLED) => ErrorKind::Cancelled,
            _ => ErrorKind::Other,
        }
    }
}

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Error {
        Error {
            hresult: e.code().0,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = windows::core::Error::from_hresult(HRESULT(self.hresult)).message();
        f.write_str(message.trim_end())
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        let kind = |code: WIN32_ERROR| Error::from_code(code.0).kind();
        assert_eq!(kind(ERROR_ACCESS_DENIED), ErrorKind::AccessDenied);
        assert_eq!(kind(ERROR_FILE_NOT_FOUND), ErrorKind::NotFound);
        assert_eq!(kind(ERROR_PATH_NOT_FOUND), ErrorKind::NotFound);
        assert_eq!(kind(ERROR_DIRECTORY), ErrorKind::NotADirectory);
        assert_eq!(kind(ERROR_BAD_NETPATH), ErrorKind::NetworkUnreachable);
        assert_eq!(kind(ERROR_SEM_TIMEOUT), ErrorKind::NetworkUnreachable);
        assert_eq!(kind(ERROR_NOT_SUPPORTED), ErrorKind::Unsupported);
        assert_eq!(kind(ERROR_CANCELLED), ErrorKind::Cancelled);
        assert_eq!(Error::from_code(1234).kind(), ErrorKind::Other);
    }

    #[test]
    fn displays_the_system_message() {
        let text = Error::from_code(ERROR_ACCESS_DENIED.0).to_string();
        assert!(!text.is_empty() && !text.ends_with('\n'), "{text:?}");
    }
}
