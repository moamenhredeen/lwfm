//! The application as the target of commands and as the host of Lua calls.

use std::os::windows::ffi::OsStrExt;

use super::list::os_name;
use super::nav::{Nav, NavKind};
use super::{App, Event};
use crate::cmd::registry::{self, CmdError, CmdResult, Target};
use crate::config::{self, discover};
use crate::fs::model::Sort;
use crate::fs::path;
use crate::lua::api::Host;
use crate::shell;

impl Host for App {
    fn notify(&self, msg: &str) {
        self.set_message(msg.to_string());
    }
}

impl Target for App {
    fn cd(&self, input: &str) -> CmdResult {
        self.cd_with(input, false);
        Ok(())
    }

    fn up(&self) -> CmdResult {
        let Some(cwd) = self.cwd() else {
            return Ok(());
        };
        let Some(parent) = path::parent(&cwd) else {
            return Ok(());
        };
        let mut nav = Nav::new(NavKind::New);
        nav.select = cwd.file_name().map(|n| n.encode_wide().collect());
        self.navigate(parent, nav);
        Ok(())
    }

    fn back(&self) -> CmdResult {
        let Some(target) = self.listing(|l| l.history.back_target().cloned()) else {
            return Ok(());
        };
        let mut nav = Nav::new(NavKind::Back);
        nav.select = target.selected;
        self.navigate(target.dir, nav);
        Ok(())
    }

    fn forward(&self) -> CmdResult {
        let Some(target) = self.listing(|l| l.history.forward_target().cloned()) else {
            return Ok(());
        };
        let mut nav = Nav::new(NavKind::Forward);
        nav.select = target.selected;
        self.navigate(target.dir, nav);
        Ok(())
    }

    fn open(&self) -> CmdResult {
        let Some(cwd) = self.cwd() else {
            return Ok(());
        };
        let Some(i) = self.ui.list.focused() else {
            return Ok(());
        };
        let entry = self.listing(|l| l.model.entry(i).map(|e| (os_name(&e.name), e.is_dir())));
        let Some((name, is_dir)) = entry else {
            return Ok(());
        };
        let target = cwd.join(&name);
        if is_dir {
            self.navigate(target, Nav::new(NavKind::New));
            return Ok(());
        }
        let config = self.config();
        let file_name = name.to_string_lossy();
        let opener = shell::best_opener(config.openers.iter().map(|(p, _)| p.as_str()), &file_name);
        match opener {
            Some(k) => self.run_binding(&config, &config.openers[k].1),
            None => self.shell_open(target, cwd),
        }
        Ok(())
    }

    fn refresh(&self) -> CmdResult {
        if let Some(cwd) = self.cwd() {
            self.navigate(cwd, Nav::new(NavKind::Refresh));
        }
        Ok(())
    }

    fn toggle_hidden(&self) -> CmdResult {
        self.listing_mut(|l| l.model.show_hidden = !l.model.show_hidden);
        self.rebuild();
        Ok(())
    }

    fn sort(&self, sort: Sort) -> CmdResult {
        self.listing_mut(|l| l.model.sort = sort);
        self.rebuild();
        Ok(())
    }

    fn focus_path(&self) -> CmdResult {
        self.ui.path.focus_control();
        self.ui.path.select_all();
        Ok(())
    }

    fn focus_list(&self) -> CmdResult {
        self.ui.list.focus_control();
        Ok(())
    }

    fn path_submit(&self) -> CmdResult {
        let text = self.ui.path.text();
        if let Some(command) = text.trim_start().strip_prefix(':') {
            if command.trim().is_empty() {
                return self.path_cancel();
            }
            registry::run_str(self, command)?;
            // Success: show the directory again (unless the command moved focus there).
            if self.ui.path.has_focus() {
                self.path_cancel()?;
            }
            return Ok(());
        }
        if text.trim().is_empty() {
            return Ok(());
        }
        self.cd_with(&text, true);
        Ok(())
    }

    fn path_cancel(&self) -> CmdResult {
        let cwd = self.cwd().map(|d| d.display().to_string());
        self.ui.path.set_text(&cwd.unwrap_or_default());
        self.focus_list()
    }

    fn quit(&self) -> CmdResult {
        self.ui.window.close();
        Ok(())
    }

    fn shell(&self, command: &str, console: bool) -> CmdResult {
        let dir = self.base_dir();
        // `edit-config` runs the editor on the config file, not on the listing.
        let (focused, selection) = match self.with_config(|c| c.file_override.clone()) {
            Some(file) => (Some(file), Vec::new()),
            None => (self.focused_path(), self.selected_paths()),
        };
        let ctx = shell::Context {
            dir: &dir,
            focused: focused.as_deref(),
            selection: &selection,
        };
        let line = shell::expand(command, &ctx).map_err(CmdError)?;
        let waker = self.waker.clone();
        shell::launch(line, dir.clone(), console, move |msg| {
            waker.send(Event::Message(msg));
        });
        Ok(())
    }

    fn search(&self, text: &str) -> CmdResult {
        self.open_search(text);
        Ok(())
    }

    fn search_next(&self) -> CmdResult {
        self.step_match(true);
        Ok(())
    }

    fn search_prev(&self) -> CmdResult {
        self.step_match(false);
        Ok(())
    }

    fn search_close(&self) -> CmdResult {
        self.close_search();
        Ok(())
    }

    fn prompt(&self) -> CmdResult {
        self.ui.path.focus_control();
        self.ui.path.set_text(":");
        self.ui.path.set_caret(1);
        Ok(())
    }

    fn edit_config(&self) -> CmdResult {
        let path = self
            .with_config(|c| c.file.clone())
            .or_else(|| self.config_env.default_location())
            .ok_or_else(|| CmdError("no location for a config file (APPDATA is not set)".into()))?;
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let created = !path.exists();
            if created {
                let write = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, config::STARTER));
                if let Err(e) = write {
                    waker.send(Event::Message(format!(
                        "Cannot create {}: {e}",
                        path.display()
                    )));
                    return;
                }
            }
            waker.send(Event::ConfigReady { path, created });
        });
        Ok(())
    }

    fn reload_config(&self) -> CmdResult {
        let known = self.with_config(|c| c.file.clone());
        let env = self.config_env.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let found = known.or_else(|| match discover::discover(&env, &|p| p.is_file()) {
                discover::Found::File(p) => Some(p),
                _ => None,
            });
            match found {
                Some(path) => {
                    let source = config::read_file(&path);
                    waker.send(Event::ConfigChanged(path, source))
                }
                None => waker.send(Event::Message("No config file to reload".into())),
            };
        });
        Ok(())
    }
}
