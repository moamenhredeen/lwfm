//! Directory enumeration off the UI thread.
//!
//! Each request runs on its own thread (an unreachable share can block while
//! it is opened for many seconds, and must not delay later requests).
//! Results are sent in batches tagged with a generation; the worker stops as
//! soon as the shared generation moves on.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lwfm_win::fs::{self, RawEntry};
use lwfm_win::{Error, ErrorKind};

pub use lwfm_win::fs::Method;

use super::entry::Entry;

const BATCH_ENTRIES: usize = 1024;
const BATCH_INTERVAL: Duration = Duration::from_millis(30);
/// The batch age is checked once per this many entries.
const CHECK_EVERY: usize = 256;

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

/// Spawns a worker that enumerates `path` and reports through `send`, which
/// returns `false` once the receiver is gone. Fails when the thread cannot
/// be started; nothing is reported through `send` in that case.
pub fn spawn(
    path: PathBuf,
    generation: u64,
    current: Arc<AtomicU64>,
    send: impl Fn(FsMsg) -> bool + Send + 'static,
) -> std::io::Result<()> {
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
            Err(e) => FsMsg::Error {
                generation,
                message: describe_error(e, &path),
            },
        };
        if !stale() {
            send(msg);
        }
    };
    std::thread::Builder::new()
        .name("enumerate".into())
        .spawn(worker)
        .map(drop)
}

/// Enumerates `dir`, delivering entries in batches. Stops early (with `Ok`)
/// when `sink` returns `false`.
pub fn enumerate(
    dir: &Path,
    method: Method,
    sink: &mut dyn FnMut(Vec<Entry>) -> bool,
) -> Result<(), Error> {
    let mut batcher = Batcher {
        sink,
        buf: Vec::new(),
        last: Instant::now(),
        cancelled: false,
    };
    fs::list_dir(dir, method, &mut |raw| batcher.push(raw))?;
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
    /// Adds an entry and sends the batch when it is large or old enough.
    /// Breaks when enumeration should stop.
    fn push(&mut self, raw: RawEntry<'_>) -> ControlFlow<()> {
        self.buf.push(Entry::from(raw));
        let len = self.buf.len();
        if len >= BATCH_ENTRIES
            || (len.is_multiple_of(CHECK_EVERY) && self.last.elapsed() >= BATCH_INTERVAL)
        {
            self.flush();
        }
        if self.cancelled {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
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

/// A one-line, user-facing description of an enumeration error.
pub fn describe_error(error: Error, path: &Path) -> String {
    let p = path.display();
    let what = match error.kind() {
        ErrorKind::AccessDenied => "Access denied",
        ErrorKind::NotFound => "Not found",
        ErrorKind::NotADirectory => "Not a directory",
        ErrorKind::NotReady => "Device not ready",
        ErrorKind::NetworkUnreachable => "Network location unreachable",
        ErrorKind::Unsupported | ErrorKind::Cancelled | ErrorKind::Other => {
            return format!("Cannot read {p}: {error}");
        }
    };
    format!("{what}: {p}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("lwfm-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }

        fn with_files(name: &str, count: usize) -> TempDir {
            let t = TempDir::new(name);
            for i in 0..count {
                std::fs::write(t.0.join(format!("f{i}")), b"").unwrap();
            }
            t
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn entries_are_converted() {
        let t = TempDir::new("convert");
        std::fs::create_dir(t.0.join("sub")).unwrap();
        std::fs::write(t.0.join("a.txt"), vec![0u8; 12288]).unwrap();
        let mut all = Vec::new();
        enumerate(&t.0, Method::Bulk, &mut |b| {
            all.extend(b);
            true
        })
        .unwrap();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<String> = all
            .iter()
            .map(|e| String::from_utf16(&e.name).unwrap())
            .collect();
        assert_eq!(names, ["a.txt", "sub"]);
        assert_eq!(all[0].size, 12288);
        assert!(!all[0].is_dir() && all[0].mtime > 0);
        assert!(all[1].is_dir());
    }

    #[test]
    fn errors() {
        let t = TempDir::new("errors");
        std::fs::write(t.0.join("a.txt"), b"x").unwrap();
        let mut sink = |_: Vec<Entry>| true;
        let missing = enumerate(&t.0.join("nope"), Method::Bulk, &mut sink).unwrap_err();
        assert_eq!(
            describe_error(missing, Path::new(r"C:\nope")),
            r"Not found: C:\nope"
        );
        let file = enumerate(&t.0.join("a.txt"), Method::Bulk, &mut sink).unwrap_err();
        assert_eq!(
            describe_error(file, Path::new(r"C:\a.txt")),
            r"Not a directory: C:\a.txt"
        );
        // ERROR_ACCESS_DENIED, ERROR_BAD_NETPATH and an error without a
        // short description (ERROR_SHARING_VIOLATION).
        assert_eq!(
            describe_error(Error::from_code(5), Path::new(r"C:\x")),
            r"Access denied: C:\x"
        );
        assert!(describe_error(Error::from_code(53), Path::new(r"\\h\s")).starts_with("Network"));
        assert!(
            describe_error(Error::from_code(32), Path::new(r"C:\x"))
                .starts_with(r"Cannot read C:\x: ")
        );
    }

    #[test]
    fn large_directory_arrives_in_batches() {
        let t = TempDir::with_files("batches", 5000);
        let (tx, rx) = mpsc::channel();
        let current = Arc::new(AtomicU64::new(1));
        spawn(t.0.clone(), 1, current, move |m| tx.send(m).is_ok()).unwrap();
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
        let t = TempDir::with_files("stale", 3000);
        let (tx, rx) = mpsc::channel();
        let current = Arc::new(AtomicU64::new(1));
        let gen_handle = current.clone();
        spawn(t.0.clone(), 1, current, move |m| {
            // The user navigates elsewhere as soon as the first batch arrives.
            gen_handle.store(2, Ordering::Release);
            tx.send(m).is_ok()
        })
        .unwrap();
        let msgs: Vec<FsMsg> = rx.iter().collect();
        assert_eq!(
            msgs.len(),
            1,
            "only the batch in flight may arrive: {msgs:?}"
        );
    }
}
