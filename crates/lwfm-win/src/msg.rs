//! Sending window messages and reading their parameters.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

/// Sends a message whose parameters are plain values.
pub(crate) fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: the parameters are plain values for this message, so the
    // receiver dereferences nothing; an invalid window makes the call fail.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }.0
}

/// Sends a message whose `lParam` points to `data`, which the receiver reads.
/// `T` must be the structure that `msg` documents.
pub(crate) fn send_ref<T>(hwnd: HWND, msg: u32, wparam: usize, data: &T) -> isize {
    let lparam = LPARAM(std::ptr::from_ref(data).addr().cast_signed());
    // SAFETY: `data` is a live `T` for the whole synchronous call and the
    // receiver only reads it.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(lparam)) }.0
}

/// Sends a message whose `lParam` points to `data`, which the receiver may
/// write. `T` must be the structure that `msg` documents.
pub(crate) fn send_mut<T>(hwnd: HWND, msg: u32, wparam: usize, data: &mut T) -> isize {
    let lparam = LPARAM(std::ptr::from_mut(data).addr().cast_signed());
    // SAFETY: `data` is a live, exclusively borrowed `T` for the whole
    // synchronous call.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(lparam)) }.0
}

/// Sends a message whose `lParam` points to the first element of `data`,
/// which the receiver reads. The element type and any required terminator
/// must be what `msg` documents.
pub(crate) fn send_slice<T>(hwnd: HWND, msg: u32, wparam: usize, data: &[T]) -> isize {
    let lparam = LPARAM(data.as_ptr().addr().cast_signed());
    // SAFETY: `data` is live for the whole synchronous call and the receiver
    // only reads it.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(lparam)) }.0
}

/// The structure a notification's `lParam` points to.
///
/// # Safety
///
/// `lparam` must point to a live `T` that is not written through another
/// path for `'a`: the message being handled must document `T` as its
/// parameter, and `'a` must not outlive the handling of that message.
pub(crate) unsafe fn param_ref<'a, T>(lparam: LPARAM) -> &'a T {
    let ptr = std::ptr::with_exposed_provenance::<T>(lparam.0.cast_unsigned());
    // SAFETY: guaranteed by the caller.
    unsafe { &*ptr }
}

/// Like [`param_ref`], for structures the handler fills in.
///
/// # Safety
///
/// As for [`param_ref`], and nothing else may access the `T` during `'a`.
pub(crate) unsafe fn param_mut<'a, T>(lparam: LPARAM) -> &'a mut T {
    let ptr = std::ptr::with_exposed_provenance_mut::<T>(lparam.0.cast_unsigned());
    // SAFETY: guaranteed by the caller.
    unsafe { &mut *ptr }
}
