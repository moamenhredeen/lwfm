//! Watches the config directory (including `lua\`) and reports the new
//! config source after a change. Runs on its own thread; the UI thread only
//! receives the file content.

use std::ffi::c_void;
use std::mem::offset_of;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

const DEBOUNCE: Duration = Duration::from_millis(100);
/// Identical content within this window comes from events buffered during the debounce.
const DUPLICATE_WINDOW: Duration = Duration::from_millis(500);

/// Stops reporting when dropped. The thread exits at its next wake-up.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    pub path: PathBuf,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Whether a change notification for `rel` (relative to the config dir)
/// affects the config: the file itself or anything under `lua\`.
fn relevant(rel: &str, file_name: &str) -> bool {
    let rel = rel.to_lowercase();
    rel == file_name.to_lowercase() || rel.starts_with("lua\\")
}

/// Starts watching the directory of `path`. `on_change` receives the file
/// path and its new content (or a read error).
pub fn watch(
    path: PathBuf,
    on_change: impl Fn(PathBuf, Result<String, String>) + Send + 'static,
) -> Option<Watcher> {
    let dir = path.parent()?.to_path_buf();
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = Watcher {
        stop: stop.clone(),
        path: path.clone(),
    };
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            let dir_w: Vec<u16> = dir
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .chain([0])
                .collect();
            let Ok(handle) = (unsafe {
                CreateFileW(
                    PCWSTR(dir_w.as_ptr()),
                    FILE_LIST_DIRECTORY.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    None,
                )
            }) else {
                return;
            };
            let mut buf = vec![0u64; 8 * 1024];
            let mut last_sent: Option<(Instant, Result<String, String>)> = None;
            while !stop.load(Ordering::Acquire) {
                let mut bytes = 0u32;
                let ok = unsafe {
                    ReadDirectoryChangesW(
                        handle,
                        buf.as_mut_ptr() as *mut c_void,
                        (buf.len() * 8) as u32,
                        true,
                        FILE_NOTIFY_CHANGE_LAST_WRITE
                            | FILE_NOTIFY_CHANGE_FILE_NAME
                            | FILE_NOTIFY_CHANGE_SIZE,
                        Some(&mut bytes),
                        None,
                        None,
                    )
                };
                if ok.is_err() {
                    break;
                }
                // Zero bytes means the buffer overflowed; treat it as a module change.
                let mut hit = bytes == 0;
                let mut module = bytes == 0;
                let base = buf.as_ptr() as *const u8;
                let mut offset = 0usize;
                while (offset as u32) < bytes {
                    // SAFETY: the OS wrote a chain of FILE_NOTIFY_INFORMATION records.
                    let (next, name) = unsafe {
                        let rec = base.add(offset) as *const FILE_NOTIFY_INFORMATION;
                        let len = (*rec).FileNameLength as usize / 2;
                        let p = base.add(offset + offset_of!(FILE_NOTIFY_INFORMATION, FileName))
                            as *const u16;
                        (
                            (*rec).NextEntryOffset as usize,
                            String::from_utf16_lossy(std::slice::from_raw_parts(p, len)),
                        )
                    };
                    if relevant(&name, &file_name) {
                        hit = true;
                        module |= name.to_lowercase().starts_with("lua\\");
                    }
                    if next == 0 {
                        break;
                    }
                    offset += next;
                }
                if !hit {
                    continue;
                }
                // Let editors finish writing (save via temp file + rename).
                std::thread::sleep(DEBOUNCE);
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let content = super::read_file(&path);
                // Events buffered during the debounce report the same config
                // content again; module edits change other files, so they always reload.
                let duplicate = last_sent
                    .as_ref()
                    .is_some_and(|(at, c)| at.elapsed() < DUPLICATE_WINDOW && *c == content);
                if module || !duplicate {
                    last_sent = Some((Instant::now(), content.clone()));
                    on_change(path.clone(), content);
                }
            }
            unsafe {
                let _ = CloseHandle(handle);
            }
        })
        .ok()?;
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevant_paths() {
        assert!(relevant("config.lua", "config.lua"));
        assert!(relevant("CONFIG.LUA", "config.lua"));
        assert!(relevant(r"lua\bookmarks.lua", "config.lua"));
        assert!(!relevant("config.lua~", "config.lua"));
        assert!(!relevant("other.lua", "config.lua"));
    }
}
