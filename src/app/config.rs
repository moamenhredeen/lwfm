//! The active configuration: applying it, watching its file and reloading.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use super::{App, Event};
use crate::config::{self, Loaded, watch};
use crate::trace;

pub(super) struct ConfigState {
    pub(super) loaded: Rc<Loaded>,
    /// The config file in use or watched (it may have failed to load).
    pub(super) file: Option<PathBuf>,
    watcher: Option<watch::Watcher>,
    /// `$f` for the next `shell` run (the config file for `edit-config`).
    pub(super) file_override: Option<PathBuf>,
}

impl ConfigState {
    pub(super) fn new(loaded: Loaded, file: Option<PathBuf>) -> ConfigState {
        ConfigState {
            loaded: Rc::new(loaded),
            file,
            watcher: None,
            file_override: None,
        }
    }
}

impl App {
    /// Makes the listing follow the configured view settings, replacing
    /// what was changed at runtime.
    fn apply_settings(&self) {
        let config = self.config();
        self.listing_mut(|l| {
            l.model.show_hidden = config.settings.show_hidden;
            l.model.sort = config.settings.sort;
        });
    }

    pub(super) fn watch_config(&self, path: PathBuf) {
        let watched = self.with_config(|c| c.watcher.as_ref().is_some_and(|w| w.path == path));
        if watched {
            return;
        }
        let waker = self.waker.clone();
        let watcher = watch::watch(path, move |p, source| {
            waker.send(Event::ConfigChanged(p, source));
        });
        self.with_config(|c| c.watcher = watcher);
    }

    /// New content of the config file arrived (or reading it failed).
    pub(super) fn on_config_changed(&self, path: PathBuf, source: Result<String, String>) {
        let started = Instant::now();
        let loaded = source.and_then(|src| config::load(Some(&path), Some(&src)));
        trace::log(|| {
            format!(
                "config loaded in {} ({})",
                trace::ms(started.elapsed()),
                path.display()
            )
        });
        match loaded {
            Ok(l) => {
                let msg = if l.warnings.is_empty() {
                    "Config reloaded".to_string()
                } else {
                    format!("Config reloaded with warnings: {}", l.warnings.join("; "))
                };
                // The previous configuration is dropped outside the borrow:
                // it owns a Lua state, whose finalizers are foreign code.
                let previous = self.with_config(|c| {
                    c.file = Some(path.clone());
                    std::mem::replace(&mut c.loaded, Rc::new(l))
                });
                drop(previous);
                self.end_sequence();
                self.apply_settings();
                self.rebuild();
                self.watch_config(path);
                self.set_message(msg);
            }
            Err(e) => self.set_message(e),
        }
    }

    /// `edit-config` made sure the file exists: open it in the editor.
    pub(super) fn on_config_ready(&self, path: PathBuf, created: bool) {
        if created {
            self.on_config_changed(path.clone(), Ok(config::STARTER.to_string()));
            self.set_message(format!("Created {}", path.display()));
        }
        let config = self.config();
        self.with_config(|c| c.file_override = Some(path));
        let result = config.settings.editor.run(self);
        self.with_config(|c| c.file_override = None);
        if let Err(e) = result {
            let editor = config.settings.editor.def.name;
            self.set_message(format!("editor ({editor}): {e}"));
        }
    }
}
