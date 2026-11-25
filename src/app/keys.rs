//! Keyboard input: chords, key sequences and running what they are bound to.

use std::rc::Rc;

use lwfm_win::input::{KeyEvent, VirtualKey};

use super::App;
use crate::cmd::registry;
use crate::config::Loaded;
use crate::keys::chord::{Chord, Key, Mods, Named};
use crate::keys::keymap::{Action, Binding, Lookup, Scope};
use crate::keys::normalize;
use crate::lua::api::{self, Event as LuaEvent};

/// A key sequence in progress.
#[derive(Default)]
pub(super) struct KeySeq {
    /// Chords typed so far.
    pending: Vec<Chord>,
    /// Hint bar text while the sequence is pending; replaces the status message.
    pub(super) hint: Option<String>,
}

impl App {
    /// Resolves keyboard events against the keymap before any control sees
    /// them. Returns `true` when the event was consumed.
    pub(super) fn translate_key(&self, event: KeyEvent) -> bool {
        let (chord, typed) = match event {
            KeyEvent::Down { key, mods } => (
                normalize::from_keydown(key, mods, &VirtualKey::layout_char),
                false,
            ),
            KeyEvent::Char(c) => (normalize::from_char(c), true),
        };
        let Some(chord) = chord else { return false };
        let chain = self.scope_chain();
        let mut seq = self.keys.borrow().pending.clone();
        let pending = !seq.is_empty();
        if pending && chord == Chord::new(Mods::NONE, Key::Named(Named::Esc)) {
            self.end_sequence();
            return true;
        }
        seq.push(chord);
        let config = self.config();
        match config.keymap.lookup(&chain, &seq) {
            Lookup::Match(b) => {
                self.end_sequence();
                self.run_binding(&config, b);
                true
            }
            Lookup::Prefix(hints) => {
                let keys: Vec<String> = seq.iter().map(Chord::to_string).collect();
                let items: Vec<String> = hints.iter().map(|(c, l)| format!("{c} {l}")).collect();
                let hint = format!(
                    "{} ...   {}   (esc cancels)",
                    keys.join(" "),
                    items.join("   ")
                );
                *self.keys.borrow_mut() = KeySeq {
                    pending: seq,
                    hint: Some(hint),
                };
                self.update_status();
                true
            }
            Lookup::None if pending => {
                // A chord that continues no binding ends the sequence and is dropped.
                self.end_sequence();
                true
            }
            Lookup::None => {
                // Typing in the list opens the search input.
                if typed
                    && chain.contains(&Scope::List)
                    && let Key::Char(c) = chord.key
                {
                    let inv = registry::compile_parts("search", vec![c.to_string()], vec![]);
                    if let Err(e) = inv.and_then(|inv| inv.run(self)) {
                        self.set_message(e.0);
                    }
                    return true;
                }
                false
            }
        }
    }

    /// The keymap scopes to search, by which control has the focus.
    fn scope_chain(&self) -> Vec<Scope> {
        if self.ui.path.has_focus() {
            vec![Scope::Input, Scope::Global]
        } else if self.ui.search.has_focus() {
            vec![Scope::Search, Scope::Global]
        } else if self.ui.list.has_focus() {
            vec![Scope::List, Scope::Global]
        } else {
            vec![Scope::Global]
        }
    }

    pub(super) fn end_sequence(&self) {
        let had_hint = std::mem::take(&mut *self.keys.borrow_mut()).hint.is_some();
        if had_hint {
            self.update_status();
        }
    }

    /// Runs a key binding or opener. `config` keeps the Lua state alive even
    /// if the action reloads the configuration.
    pub(super) fn run_binding(&self, config: &Rc<Loaded>, b: &Binding) {
        match &b.action {
            Action::Commands(invs) => {
                for inv in invs {
                    if let Err(e) = inv.run(self) {
                        self.set_message(e.0);
                        break;
                    }
                }
            }
            Action::Lua(f) => {
                let state = self.fm_state();
                if let Err(e) = api::call(&config.lua, self, &state, f, &[]) {
                    self.set_message(e.to_string());
                }
            }
        }
    }

    pub(super) fn run_str(&self, s: &str) {
        if let Err(e) = registry::run_str(self, s) {
            self.set_message(e.to_string());
        }
    }

    /// Calls the Lua handlers registered for `event`.
    pub(super) fn fire(&self, event: LuaEvent, extra: &[String]) {
        let config = self.config();
        let Some(handlers) = config.handlers.get(&event) else {
            return;
        };
        for f in handlers {
            let state = self.fm_state();
            if let Err(e) = api::call(&config.lua, self, &state, f, extra) {
                self.set_message(e.to_string());
            }
        }
    }
}
