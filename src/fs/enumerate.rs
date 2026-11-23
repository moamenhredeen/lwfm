//! Directory enumeration off the UI thread.
//!
//! Each request runs on its own thread (an unreachable share can block in
//! `CreateFileW` for many seconds, and must not delay later requests).
//! Results are sent in batches tagged with a generation; the worker stops as
//! soon as the shared generation moves on.

use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

use super::entry::Entry;
use super::path::to_api_wide;

const BATCH_ENTRIES: usize = 1024;
const BATCH_INTERVAL: Duration = Duration::from_millis(30);
const BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum FsMsg {
    Batch {
        generation: u64,
        entries: Vec<Entry>,
    },
    Done {
        generation: u64,
    },
    Error {
        generation: u64,
        message: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// `GetFileInformationByHandleEx(FileFullDirectoryInfo)`, falling back
    /// to `Find` when the file system does not support it.
    Bulk,
    #[cfg_attr(not(test), allow(dead_code))]
    Find,
}

/// Spawns a worker that enumerates `path` and reports through `send`, which
/// returns `false` once the receiver is gone.
pub fn spawn(
    path: PathBuf,
    generation: u64,
    current: Arc<AtomicU64>,
    send: impl Fn(FsMsg) -> bool + Send + 'static,
) {
    let stale = move || current.load(Ordering::Acquire) != generation;
    let worker = move || {
        let mut sink = |entries: Vec<Entry>| {
            !stale()
                && send(FsMsg::Batch {
                    generation,
                    entries,
                })
        };
        let msg = match enumerate(&path, Method::Bulk, &mut sink) {
            Ok(()) => FsMsg::Done { generation },
            Err(code) => FsMsg::Error {
                generation,
                message: describe_error(code, &path),
            },
        };
        if !stale() {
            send(msg);
        }
    };
    std::thread::Builder::new()
        .name("enumerate".into())
        .spawn(worker)
        .expect("failed to spawn enumeration thread");
}

/// Enumerates `dir`, delivering entries in batches. Stops early (with `Ok`)
/// when `sink` returns `false`. Errors are Win32 error codes.
pub fn enumerate(
    dir: &Path,
    method: Method,
    sink: &mut dyn FnMut(Vec<Entry>) -> bool,
) -> Result<(), u32> {
    let mut batcher = Batcher {
        sink,
        buf: Vec::new(),
        last: Instant::now(),
        cancelled: false,
    };
    match method {
        Method::Bulk => match enumerate_bulk(dir, &mut batcher) {
            Err(code) if is_unsupported(code) && batcher.buf.is_empty() => {
                enumerate_find(dir, &mut batcher)?
            }
            r => r?,
        },
        Method::Find => enumerate_find(dir, &mut batcher)?,
    }
    batcher.flush();
    Ok(())
}

struct Batcher<'a> {
    sink: &'a mut dyn FnMut(Vec<Entry>) -> bool,
    buf: Vec<Entry>,
    last: Instant,
    cancelled: bool,
}

impl Batcher<'_> {
    fn push(&mut self, e: Entry) {
        self.buf.push(e);
    }

    /// Sends the buffer when it is large or old enough. Returns `false` when
    /// enumeration should stop.
    fn maybe_flush(&mut self) -> bool {
        if self.buf.len() >= BATCH_ENTRIES || self.last.elapsed() >= BATCH_INTERVAL {
            self.flush();
        }
        !self.cancelled
    }

    fn flush(&mut self) {
        if self.cancelled {
            return;
        }
        let batch = std::mem::take(&mut self.buf);
        self.last = Instant::now();
        if !(self.sink)(batch) {
            self.cancelled = true;
        }
    }
}

fn is_dot(name: &[u16]) -> bool {
    matches!(name, [0x2E] | [0x2E, 0x2E])
}

fn is_unsupported(code: u32) -> bool {
    [
        ERROR_INVALID_PARAMETER,
        ERROR_INVALID_FUNCTION,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_LEVEL,
    ]
    .iter()
    .any(|e| e.0 == code)
}

fn win32_code(e: windows::core::Error) -> u32 {
    WIN32_ERROR::from_error(&e)
        .map(|w| w.0)
        .unwrap_or(e.code().0 as u32)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn enumerate_bulk(dir: &Path, out: &mut Batcher) -> Result<(), u32> {
    let wide = to_api_wide(dir, "");
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(win32_code)?;
    let handle = Handle(handle);

    // A file opened with backup semantics also succeeds; reject it here.
    let mut basic = FILE_BASIC_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle.0,
            FileBasicInfo,
            &mut basic as *mut _ as *mut c_void,
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    }
    .map_err(win32_code)?;
    if basic.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err(ERROR_DIRECTORY.0);
    }

    // u64 storage keeps the buffer 8-byte aligned, as the records require.
    let mut buf = vec![0u64; BUFFER_BYTES / 8];
    let mut class = FileFullDirectoryRestartInfo;
    loop {
        let r = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                class,
                buf.as_mut_ptr() as *mut c_void,
                BUFFER_BYTES as u32,
            )
        };
        class = FileFullDirectoryInfo;
        if let Err(e) = r {
            let code = win32_code(e);
            return if code == ERROR_NO_MORE_FILES.0 {
                Ok(())
            } else {
                Err(code)
            };
        }
        let base = buf.as_ptr() as *const u8;
        let mut offset = 0usize;
        loop {
            // SAFETY: the OS filled the buffer with a chain of FILE_FULL_DIR_INFO
            // records, each starting at an 8-byte aligned offset within it.
            let (next, entry) = unsafe {
                let rec = base.add(offset) as *const FILE_FULL_DIR_INFO;
                let name_len = (*rec).FileNameLength as usize / 2;
                let name_ptr =
                    base.add(offset + offset_of!(FILE_FULL_DIR_INFO, FileName)) as *const u16;
                let name = std::slice::from_raw_parts(name_ptr, name_len);
                let entry = (!is_dot(name)).then(|| Entry {
                    name: name.into(),
                    attrs: (*rec).FileAttributes,
                    size: (*rec).EndOfFile as u64,
                    mtime: (*rec).LastWriteTime,
                });
                ((*rec).NextEntryOffset as usize, entry)
            };
            if let Some(e) = entry {
                out.push(e);
            }
            if next == 0 {
                break;
            }
            offset += next;
        }
        if !out.maybe_flush() {
            return Ok(());
        }
    }
}

fn enumerate_find(dir: &Path, out: &mut Batcher) -> Result<(), u32> {
    let wide = to_api_wide(dir, "\\*");
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(wide.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut c_void,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match handle {
        Ok(h) => h,
        // A drive root with no entries reports "file not found".
        Err(e) if win32_code(e.clone()) == ERROR_FILE_NOT_FOUND.0 => return Ok(()),
        Err(e) => return Err(win32_code(e)),
    };
    struct FindHandle(HANDLE);
    impl Drop for FindHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = FindClose(self.0);
            }
        }
    }
    let handle = FindHandle(handle);
    let mut count = 0usize;
    loop {
        let len = data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(data.cFileName.len());
        let name = &data.cFileName[..len];
        if !is_dot(name) {
            out.push(Entry {
                name: name.into(),
                attrs: data.dwFileAttributes,
                size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
                mtime: ((data.ftLastWriteTime.dwHighDateTime as i64) << 32)
                    | data.ftLastWriteTime.dwLowDateTime as i64,
            });
        }
        count += 1;
        if count % 256 == 0 && !out.maybe_flush() {
            return Ok(());
        }
        if let Err(e) = unsafe { FindNextFileW(handle.0, &mut data) } {
            let code = win32_code(e);
            return if code == ERROR_NO_MORE_FILES.0 {
                Ok(())
            } else {
                Err(code)
            };
        }
    }
}

/// A one-line, user-facing description of an enumeration error.
pub fn describe_error(code: u32, path: &Path) -> String {
    let p = path.display();
    let what = match WIN32_ERROR(code) {
        ERROR_ACCESS_DENIED => "Access denied",
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_INVALID_NAME => "Not found",
        ERROR_DIRECTORY => "Not a directory",
        ERROR_NOT_READY => "Device not ready",
        ERROR_BAD_NETPATH
        | ERROR_BAD_NET_NAME
        | ERROR_NETWORK_UNREACHABLE
        | ERROR_HOST_UNREACHABLE
        | ERROR_UNEXP_NET_ERR
        | ERROR_SEM_TIMEOUT => "Network location unreachable",
        _ => {
            return format!(
                "Cannot read {p}: {}",
                windows::core::Error::from(WIN32_ERROR(code)).message()
            );
        }
    };
    format!("{what}: {p}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::mpsc;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("lwfm-test-{name}-{}", std::process::id()));
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
        let w: Vec<u16> = hidden.as_os_str().encode_wide().chain([0]).collect();
        unsafe { SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_ATTRIBUTE_HIDDEN).unwrap() };
        t
    }

    fn collect(dir: &Path, method: Method) -> Vec<Entry> {
        let mut all = Vec::new();
        enumerate(dir, method, &mut |b| {
            all.extend(b);
            true
        })
        .unwrap();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        all
    }

    fn check(entries: &[Entry]) {
        let names: Vec<String> = entries
            .iter()
            .map(|e| String::from_utf16(&e.name).unwrap())
            .collect();
        assert_eq!(names, ["a.txt", "secret.txt", "sub"]);
        assert_eq!(entries[0].size, 12288);
        assert!(!entries[0].is_dir() && entries[0].mtime > 0);
        assert!(entries[1].is_hidden());
        assert!(entries[2].is_dir());
    }

    #[test]
    fn bulk_enumeration() {
        let t = fixture("bulk");
        check(&collect(&t.0, Method::Bulk));
    }

    #[test]
    fn find_fallback_enumeration() {
        let t = fixture("find");
        let find = collect(&t.0, Method::Find);
        check(&find);
        assert_eq!(find, collect(&t.0, Method::Bulk));
    }

    #[test]
    fn errors() {
        let t = fixture("errors");
        let mut sink = |_: Vec<Entry>| true;
        let missing = enumerate(&t.0.join("nope"), Method::Bulk, &mut sink).unwrap_err();
        assert_eq!(
            describe_error(missing, Path::new(r"C:\nope")),
            r"Not found: C:\nope"
        );
        let file = enumerate(&t.0.join("a.txt"), Method::Bulk, &mut sink).unwrap_err();
        assert_eq!(file, ERROR_DIRECTORY.0);
        assert_eq!(
            describe_error(ERROR_ACCESS_DENIED.0, Path::new(r"C:\x")),
            r"Access denied: C:\x"
        );
        assert!(describe_error(ERROR_BAD_NETPATH.0, Path::new(r"\\h\s")).starts_with("Network"));
    }

    #[test]
    fn large_directory_arrives_in_batches() {
        let t = TempDir::new("batches");
        for i in 0..5000 {
            std::fs::write(t.0.join(format!("f{i}")), b"").unwrap();
        }
        let (tx, rx) = mpsc::channel();
        let current = Arc::new(AtomicU64::new(1));
        spawn(t.0.clone(), 1, current, move |m| tx.send(m).is_ok());
        let mut batches = 0;
        let mut total = 0;
        for msg in rx {
            match msg {
                FsMsg::Batch { entries, .. } => {
                    batches += 1;
                    total += entries.len();
                }
                FsMsg::Done { .. } => break,
                FsMsg::Error { message, .. } => panic!("{message}"),
            }
        }
        assert_eq!(total, 5000);
        assert!(batches > 1, "expected several batches, got {batches}");
    }

    #[test]
    fn stale_generation_stops_sending() {
        let t = TempDir::new("stale");
        for i in 0..3000 {
            std::fs::write(t.0.join(format!("f{i}")), b"").unwrap();
        }
        let (tx, rx) = mpsc::channel();
        let current = Arc::new(AtomicU64::new(1));
        let gen_handle = current.clone();
        spawn(t.0.clone(), 1, current, move |m| {
            // The user navigates elsewhere as soon as the first batch arrives.
            gen_handle.store(2, Ordering::Release);
            tx.send(m).is_ok()
        });
        let msgs: Vec<FsMsg> = rx.iter().collect();
        assert_eq!(
            msgs.len(),
            1,
            "only the batch in flight may arrive: {msgs:?}"
        );
    }
}
