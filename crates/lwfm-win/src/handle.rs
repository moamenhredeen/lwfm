//! Owners for system resources: each releases its resource when dropped.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::FindClose;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};

/// A kernel handle closed with `CloseHandle`.
pub(crate) struct OwnedHandle(HANDLE);

// SAFETY: a kernel handle is a process-wide identifier, not a pointer to
// thread-local data; the kernel objects used here (files, processes, events)
// may be used from any thread.
unsafe impl Send for OwnedHandle {}
// SAFETY: as above; the handle value itself is never mutated after creation.
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    /// Takes ownership of `handle`, which must be valid and not owned elsewhere.
    pub(crate) fn new(handle: HANDLE) -> OwnedHandle {
        OwnedHandle(handle)
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and owned by `self`, so it is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A search handle from `FindFirstFileExW`, closed with `FindClose`.
pub(crate) struct FindHandle(HANDLE);

impl FindHandle {
    pub(crate) fn new(handle: HANDLE) -> FindHandle {
        FindHandle(handle)
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for FindHandle {
    fn drop(&mut self) {
        // SAFETY: the search handle is valid and owned by `self`.
        let _ = unsafe { FindClose(self.0) };
    }
}

/// A single-threaded COM apartment on the current thread, left again on drop
/// only when entering it succeeded.
pub(crate) struct ComApartment {
    entered: bool,
}

impl ComApartment {
    pub(crate) fn enter() -> ComApartment {
        // SAFETY: no pointer arguments. The matching `CoUninitialize` runs in
        // `Drop` on this same thread: the type holds no `Send` impl and is
        // only ever created as a local.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        ComApartment {
            entered: hr.is_ok(),
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.entered {
            // SAFETY: balances the successful `CoInitializeEx` in `enter`.
            unsafe { CoUninitialize() };
        }
    }
}
