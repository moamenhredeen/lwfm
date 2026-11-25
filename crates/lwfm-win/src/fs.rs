//! File system access: listing a directory and watching one for changes.
//!
//! Both block, so they belong on worker threads. Paths are encoded without
//! loss and get the `\\?\` prefix when they are long.

use std::ffi::OsString;
use std::mem::offset_of;
use std::ops::ControlFlow;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{
    ERROR_DIRECTORY, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
    FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_FULL_DIR_INFO,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FIND_FIRST_EX_LARGE_FETCH, FileBasicInfo, FileFullDirectoryInfo,
    FileFullDirectoryRestartInfo, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, GetFileInformationByHandleEx, OPEN_EXISTING, ReadDirectoryChangesW,
    WIN32_FIND_DATAW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::PCWSTR;

use crate::error::{Error, ErrorKind};
use crate::handle::{FindHandle, OwnedHandle};
use crate::num::{join_u64, len_u32, size_u32, ticks_i64};
use crate::wide::api_path;

/// Size of the buffers the system fills with directory records.
const BUFFER_BYTES: usize = 64 * 1024;

/// One directory entry as the file system reports it. The name is borrowed
/// from the system's buffer and is only valid during the callback.
#[derive(Clone, Copy, Debug)]
pub struct RawEntry<'a> {
    /// UTF-16 name without terminator.
    pub name: &'a [u16],
    pub size: u64,
    /// Last write time as a FILETIME value (100 ns ticks since 1601, UTC).
    pub mtime: i64,
    pub is_dir: bool,
    /// Hidden or system.
    pub is_hidden: bool,
}

impl<'a> RawEntry<'a> {
    fn new(name: &'a [u16], attributes: u32, size: u64, mtime: i64) -> RawEntry<'a> {
        RawEntry {
            name,
            size,
            mtime,
            is_dir: attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
            is_hidden: attributes & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// One system call per buffer of entries, falling back to `Find` when
    /// the file system does not support it.
    Bulk,
    /// `FindFirstFileExW`/`FindNextFileW`, supported everywhere.
    Find,
}

type Sink<'a> = dyn FnMut(RawEntry<'_>) -> ControlFlow<()> + 'a;

/// Lists `dir`, passing every entry except `.` and `..` to `sink`. Stops
/// early (with `Ok`) when `sink` breaks.
///
/// # Errors
///
/// Fails when `dir` cannot be opened or read, or is not a directory.
pub fn list_dir(dir: &Path, method: Method, sink: &mut Sink<'_>) -> Result<(), Error> {
    match method {
        Method::Bulk => {
            let mut delivered = false;
            let mut counting = |e: RawEntry<'_>| {
                delivered = true;
                sink(e)
            };
            match list_bulk(dir, &mut counting) {
                Err(e) if e.kind() == ErrorKind::Unsupported && !delivered => list_find(dir, sink),
                r => r,
            }
        }
        Method::Find => list_find(dir, sink),
    }
}

fn is_dot(name: &[u16]) -> bool {
    matches!(name, [0x2E] | [0x2E, 0x2E])
}

fn open_directory(dir: &Path, overlapped: bool) -> Result<OwnedHandle, Error> {
    let path = api_path(dir, "");
    let flags = if overlapped {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED
    } else {
        FILE_FLAG_BACKUP_SEMANTICS
    };
    // SAFETY: `path` is NUL-terminated and outlives the call.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }?;
    Ok(OwnedHandle::new(handle))
}

fn list_bulk(dir: &Path, sink: &mut Sink<'_>) -> Result<(), Error> {
    let handle = open_directory(dir, false)?;

    // A file opened with backup semantics also succeeds; reject it here.
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: `basic` is a writable FILE_BASIC_INFO of the size passed.
    unsafe {
        GetFileInformationByHandleEx(
            handle.raw(),
            FileBasicInfo,
            (&raw mut basic).cast(),
            size_u32::<FILE_BASIC_INFO>(),
        )
    }?;
    if basic.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err(Error::from_win32(ERROR_DIRECTORY));
    }

    // u64 storage keeps the buffer 8-byte aligned, as the records require.
    let mut buf = vec![0u64; BUFFER_BYTES / 8];
    let mut class = FileFullDirectoryRestartInfo;
    loop {
        // SAFETY: `buf` is writable for BUFFER_BYTES bytes.
        let filled = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                class,
                buf.as_mut_ptr().cast(),
                len_u32(BUFFER_BYTES),
            )
        };
        class = FileFullDirectoryInfo;
        if let Err(e) = filled {
            let e = Error::from(e);
            return if e.is(ERROR_NO_MORE_FILES) {
                Ok(())
            } else {
                Err(e)
            };
        }
        let mut offset = 0usize;
        while let Some((entry, next)) = full_dir_record(&buf, offset) {
            if !is_dot(entry.name) && sink(entry).is_break() {
                return Ok(());
            }
            if next == 0 {
                break;
            }
            offset += next;
        }
    }
}

/// Reads the FILE_FULL_DIR_INFO record at byte `offset` of `buf`, returning
/// it with the distance to the next record (0 for the last). `None` when the
/// record would not lie inside the buffer.
fn full_dir_record(buf: &[u64], offset: usize) -> Option<(RawEntry<'_>, usize)> {
    let bytes = size_of_val(buf);
    if !offset.is_multiple_of(align_of::<FILE_FULL_DIR_INFO>())
        || offset.checked_add(size_of::<FILE_FULL_DIR_INFO>())? > bytes
    {
        return None;
    }
    let record = buf
        .as_ptr()
        .wrapping_byte_add(offset)
        .cast::<FILE_FULL_DIR_INFO>();
    // SAFETY: the record lies inside `buf` (checked above) at an offset that
    // satisfies its alignment; the memory is initialized (zeroed, then
    // written by the system) and every bit pattern is valid for this
    // plain-data structure. `buf` is not written while the reference lives.
    let record = unsafe { &*record };

    let name_offset = offset + offset_of!(FILE_FULL_DIR_INFO, FileName);
    let name_len = usize::try_from(record.FileNameLength).ok()? / 2;
    if name_offset.checked_add(name_len.checked_mul(2)?)? > bytes {
        return None;
    }
    // The pointer is derived from the buffer, not from `record`, so that it
    // may address the name beyond the one-element array in the structure.
    let name = buf.as_ptr().wrapping_byte_add(name_offset).cast::<u16>();
    // SAFETY: `name_len` UTF-16 units starting at `name_offset` lie inside
    // `buf` (checked above); the offset is even, so the pointer is aligned
    // for u16; the memory is initialized and not written during the borrow.
    let name = unsafe { std::slice::from_raw_parts(name, name_len) };

    let entry = RawEntry::new(
        name,
        record.FileAttributes,
        u64::try_from(record.EndOfFile).unwrap_or(0),
        record.LastWriteTime,
    );
    Some((entry, usize::try_from(record.NextEntryOffset).ok()?))
}

fn list_find(dir: &Path, sink: &mut Sink<'_>) -> Result<(), Error> {
    let pattern = api_path(dir, "\\*");
    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: `pattern` is NUL-terminated; `data` is a writable
    // WIN32_FIND_DATAW, the structure `FindExInfoBasic` fills.
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            (&raw mut data).cast(),
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match handle.map_err(Error::from) {
        Ok(h) => FindHandle::new(h),
        // A drive root with no entries reports "file not found".
        Err(e) if e.is(ERROR_FILE_NOT_FOUND) => return Ok(()),
        Err(e) => return Err(e),
    };
    loop {
        let len = data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(data.cFileName.len());
        let name = &data.cFileName[..len];
        if !is_dot(name) {
            let entry = RawEntry::new(
                name,
                data.dwFileAttributes,
                join_u64(data.nFileSizeHigh, data.nFileSizeLow),
                ticks_i64(
                    data.ftLastWriteTime.dwHighDateTime,
                    data.ftLastWriteTime.dwLowDateTime,
                ),
            );
            if sink(entry).is_break() {
                return Ok(());
            }
        }
        // SAFETY: the search handle is valid and `data` is writable.
        if let Err(e) = unsafe { FindNextFileW(handle.raw(), &raw mut data) } {
            let e = Error::from(e);
            return if e.is(ERROR_NO_MORE_FILES) {
                Ok(())
            } else {
                Err(e)
            };
        }
    }
}

/// Watches a directory tree and reports changed paths, relative to the
/// directory, from its own thread. Dropping it stops the thread promptly,
/// also while it waits for the next change.
pub struct DirWatcher {
    shared: Arc<WatchShared>,
}

struct WatchShared {
    stop: AtomicBool,
    stop_event: OwnedHandle,
}

impl DirWatcher {
    /// Starts watching `dir` and its subdirectories for created, renamed,
    /// deleted and written files. `on_change` runs on the watcher thread
    /// with the changed paths; an empty slice means changes were lost and
    /// anything may have changed. Changes that happen while `on_change`
    /// runs are reported afterwards.
    ///
    /// # Errors
    ///
    /// Fails when `dir` cannot be opened or the thread cannot be started.
    pub fn start(
        dir: &Path,
        on_change: impl FnMut(&[PathBuf]) + Send + 'static,
    ) -> Result<DirWatcher, Error> {
        let directory = open_directory(dir, true)?;
        let io_event = manual_reset_event()?;
        let shared = Arc::new(WatchShared {
            stop: AtomicBool::new(false),
            stop_event: manual_reset_event()?,
        });
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("dir-watch".into())
            .spawn(move || watch_loop(&directory, &io_event, &thread_shared, on_change))
            .map_err(|e| {
                e.raw_os_error()
                    .and_then(|code| u32::try_from(code).ok())
                    .map_or(Error::last(), Error::from_code)
            })?;
        Ok(DirWatcher { shared })
    }
}

impl Drop for DirWatcher {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        // SAFETY: the event handle is valid for as long as `shared` lives.
        let _ = unsafe { SetEvent(self.shared.stop_event.raw()) };
    }
}

fn manual_reset_event() -> Result<OwnedHandle, Error> {
    // SAFETY: no pointer arguments other than the absent name and attributes.
    let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }?;
    Ok(OwnedHandle::new(event))
}

fn watch_loop(
    directory: &OwnedHandle,
    io_event: &OwnedHandle,
    shared: &WatchShared,
    mut on_change: impl FnMut(&[PathBuf]),
) {
    let mut buf = vec![0u64; BUFFER_BYTES / 8];
    let filter =
        FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE;
    while !shared.stop.load(Ordering::Acquire) {
        let mut overlapped = OVERLAPPED {
            hEvent: io_event.raw(),
            ..Default::default()
        };
        // SAFETY: `buf` is writable for BUFFER_BYTES bytes. `buf` and
        // `overlapped` stay alive and untouched until the operation has
        // completed or been cancelled, which `GetOverlappedResult` below
        // waits for before this iteration ends.
        let started = unsafe {
            ReadDirectoryChangesW(
                directory.raw(),
                buf.as_mut_ptr().cast(),
                len_u32(BUFFER_BYTES),
                true,
                filter,
                None,
                Some(&raw mut overlapped),
                None,
            )
        };
        if started.is_err() {
            return;
        }
        let events = [io_event.raw(), shared.stop_event.raw()];
        // SAFETY: both handles are valid events owned by this thread or by
        // `shared`, which outlives the call.
        let woke = unsafe { WaitForMultipleObjects(&events, false, INFINITE) };
        let stopping = woke != WAIT_OBJECT_0;
        if stopping {
            // SAFETY: cancels the read started above on the same handle.
            let _ = unsafe { CancelIoEx(directory.raw(), Some(&raw const overlapped)) };
        }
        let mut bytes = 0u32;
        // SAFETY: `overlapped` belongs to the read started above; waiting
        // here guarantees the system no longer uses it or `buf` afterwards.
        let finished = unsafe {
            GetOverlappedResult(directory.raw(), &raw const overlapped, &raw mut bytes, true)
        };
        if stopping || finished.is_err() {
            return;
        }
        // Zero bytes means the system's buffer overflowed and changes were lost.
        let changed = changed_paths(&buf, usize::try_from(bytes).unwrap_or(0));
        on_change(&changed);
    }
}

/// Decodes the chain of FILE_NOTIFY_INFORMATION records in the first
/// `filled` bytes of `buf`.
fn changed_paths(buf: &[u64], filled: usize) -> Vec<PathBuf> {
    let filled = filled.min(size_of_val(buf));
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset
        .checked_add(size_of::<FILE_NOTIFY_INFORMATION>())
        .is_some_and(|end| end <= filled)
        && offset.is_multiple_of(align_of::<FILE_NOTIFY_INFORMATION>())
    {
        let record = buf
            .as_ptr()
            .wrapping_byte_add(offset)
            .cast::<FILE_NOTIFY_INFORMATION>();
        // SAFETY: the record lies inside the filled part of `buf` (checked
        // by the loop condition) at an aligned offset; the memory is
        // initialized and every bit pattern is valid for this structure.
        let record = unsafe { &*record };
        let name_offset = offset + offset_of!(FILE_NOTIFY_INFORMATION, FileName);
        let name_len = usize::try_from(record.FileNameLength).unwrap_or(usize::MAX) / 2;
        let Some(name_end) = name_len
            .checked_mul(2)
            .and_then(|n| n.checked_add(name_offset))
        else {
            break;
        };
        if name_end > filled {
            break;
        }
        let name = buf.as_ptr().wrapping_byte_add(name_offset).cast::<u16>();
        // SAFETY: `name_len` UTF-16 units from `name_offset` lie inside the
        // filled part of `buf` (checked above); the offset is even, so the
        // pointer is aligned; the pointer is derived from the buffer itself.
        let name = unsafe { std::slice::from_raw_parts(name, name_len) };
        out.push(PathBuf::from(OsString::from_wide(name)));
        let next = usize::try_from(record.NextEntryOffset).unwrap_or(0);
        if next == 0 {
            break;
        }
        offset += next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    use windows::Win32::Storage::FileSystem::SetFileAttributesW;

    use crate::wide::wide;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("lwfm-win-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(name: &str) -> TempDir {
        let t = TempDir::new(name);
        std::fs::create_dir(t.0.join("sub")).unwrap();
        std::fs::write(t.0.join("a.txt"), vec![0u8; 12288]).unwrap();
        let hidden = t.0.join("secret.txt");
        std::fs::write(&hidden, b"x").unwrap();
        let w = wide(&hidden);
        // SAFETY: `w` is NUL-terminated and outlives the call.
        unsafe { SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_ATTRIBUTE_HIDDEN) }.unwrap();
        t
    }

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct Seen {
        name: String,
        size: u64,
        has_mtime: bool,
        is_dir: bool,
        is_hidden: bool,
    }

    fn collect(dir: &Path, method: Method) -> Vec<Seen> {
        let mut all = Vec::new();
        list_dir(dir, method, &mut |e| {
            all.push(Seen {
                name: String::from_utf16(e.name).unwrap(),
                size: e.size,
                has_mtime: e.mtime > 0,
                is_dir: e.is_dir,
                is_hidden: e.is_hidden,
            });
            ControlFlow::Continue(())
        })
        .unwrap();
        all.sort();
        all
    }

    fn check(entries: &[Seen]) {
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "secret.txt", "sub"]);
        assert_eq!(entries[0].size, 12288);
        assert!(!entries[0].is_dir && entries[0].has_mtime);
        assert!(entries[1].is_hidden);
        assert!(entries[2].is_dir);
    }

    #[test]
    fn bulk_listing() {
        let t = fixture("bulk");
        check(&collect(&t.0, Method::Bulk));
    }

    #[test]
    fn find_listing_matches_bulk() {
        let t = fixture("find");
        let find = collect(&t.0, Method::Find);
        check(&find);
        assert_eq!(find, collect(&t.0, Method::Bulk));
    }

    #[test]
    fn listing_errors() {
        let t = fixture("errors");
        let mut sink = |_: RawEntry<'_>| ControlFlow::Continue(());
        let missing = list_dir(&t.0.join("nope"), Method::Bulk, &mut sink).unwrap_err();
        assert_eq!(missing.kind(), ErrorKind::NotFound);
        let file = list_dir(&t.0.join("a.txt"), Method::Bulk, &mut sink).unwrap_err();
        assert_eq!(file.kind(), ErrorKind::NotADirectory);
    }

    #[test]
    fn listing_stops_when_the_sink_breaks() {
        let t = fixture("break");
        for method in [Method::Bulk, Method::Find] {
            let mut seen = 0;
            list_dir(&t.0, method, &mut |_| {
                seen += 1;
                ControlFlow::Break(())
            })
            .unwrap();
            assert_eq!(seen, 1, "{method:?}");
        }
    }

    #[test]
    fn large_directory_is_listed_completely() {
        let t = TempDir::new("large");
        for i in 0..3000 {
            std::fs::write(t.0.join(format!("f{i}")), b"").unwrap();
        }
        assert_eq!(collect(&t.0, Method::Bulk).len(), 3000);
    }

    /// Reports through the channel when the watcher thread has dropped its
    /// callback, which it does when it ends.
    struct Ended(mpsc::Sender<()>);

    impl Drop for Ended {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    #[test]
    fn watcher_reports_changes_in_subdirectories() {
        let t = TempDir::new("watch");
        std::fs::create_dir(t.0.join("lua")).unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = DirWatcher::start(&t.0, move |paths| {
            for p in paths {
                let _ = tx.send(p.clone());
            }
        })
        .unwrap();
        std::fs::write(t.0.join("lua").join("mod.lua"), b"return 1").unwrap();
        let seen = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(seen.starts_with("lua"), "{seen:?}");
    }

    #[test]
    fn dropped_watcher_ends_its_thread_while_waiting() {
        let t = TempDir::new("watch-drop");
        let (tx, rx) = mpsc::channel();
        let ended = Ended(tx);
        let watcher = DirWatcher::start(&t.0, move |_| {
            let _keep = &ended;
        })
        .unwrap();
        // Let the thread reach its wait for the first change.
        std::thread::sleep(Duration::from_millis(100));
        assert!(rx.try_recv().is_err(), "thread ended too early");
        drop(watcher);
        rx.recv_timeout(Duration::from_secs(1))
            .expect("watcher thread did not end within one second");
    }
}
