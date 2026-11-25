//! Watches the config directory (including `lua\`) and reports the new
//! config source after a change. Runs on the watcher's own thread; the UI
//! thread only receives the file content.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use lwfm_win::fs::DirWatcher;

use super::ConfigError;

const DEBOUNCE: Duration = Duration::from_millis(100);
/// Identical content within this window comes from events buffered during the debounce.
const DUPLICATE_WINDOW: Duration = Duration::from_millis(500);

/// Stops watching and reporting when dropped.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    pub path: PathBuf,
    _dir: DirWatcher,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // A change being debounced right now must not be reported any more.
        self.stop.store(true, Ordering::Release);
    }
}

/// Whether a changed path (relative to the config dir) is under `lua\`.
fn is_module(rel: &str) -> bool {
    rel.to_lowercase().starts_with("lua\\")
}

/// Whether a change notification for `rel` (relative to the config dir)
/// affects the config: the file itself or anything under `lua\`.
fn relevant(rel: &str, file_name: &str) -> bool {
    rel.to_lowercase() == file_name.to_lowercase() || is_module(rel)
}

/// Starts watching the directory of `path`. `on_change` receives the file
/// path and its new content (or a read error).
pub fn watch(
    path: PathBuf,
    on_change: impl Fn(PathBuf, Result<String, ConfigError>) + Send + 'static,
) -> Option<Watcher> {
    let dir = path.parent()?.to_path_buf();
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let file = path.clone();
    // What was reported last and when; a read error is kept as its text.
    let mut last_sent: Option<(Instant, Result<String, String>)> = None;
    let dir_watcher = DirWatcher::start(&dir, move |changed| {
        let rel = |p: &PathBuf| Path::to_string_lossy(p).into_owned();
        // No paths means changes were lost; treat it as a module change.
        let module = changed.is_empty() || changed.iter().any(|p| is_module(&rel(p)));
        let hit = module || changed.iter().any(|p| relevant(&rel(p), &file_name));
        if !hit {
            return;
        }
        // Let editors finish writing (save via temp file + rename).
        std::thread::sleep(DEBOUNCE);
        if stopped.load(Ordering::Acquire) {
            return;
        }
        let content = super::read_file(&file);
        // Events buffered during the debounce report the same config
        // content again; module edits change other files, so they always reload.
        let seen = content
            .as_ref()
            .map(String::clone)
            .map_err(ToString::to_string);
        let duplicate = last_sent
            .as_ref()
            .is_some_and(|(at, c)| at.elapsed() < DUPLICATE_WINDOW && *c == seen);
        if module || !duplicate {
            last_sent = Some((Instant::now(), seen));
            on_change(file.clone(), content);
        }
    })
    .ok()?;
    Some(Watcher {
        stop,
        path,
        _dir: dir_watcher,
    })
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
