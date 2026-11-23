# Proposal

## Why

lwfm's keymap and settings are hardcoded, so there is no way to customize it, and the core promise of the project (customizable through a text file instead of an options dialog) is unmet. The list also still relies on the ListView's native prefix type-to-find, which is weaker than the search-as-you-type users expect. This change makes Lua (`config.lua`) the single configuration and scripting language, built on the command registry from `core-window-listing`, and adds the pieces that make a scriptable file manager useful day to day: external tools, openers, events, a command prompt and filtering by typing.

## What Changes

- **Lua runtime:** embed Lua 5.4 (via `mlua`, vendored) on the UI thread. `config.lua` is a script that returns a configuration table, WezTerm style.
- **Config discovery and lifecycle:** `--config <path>`, then `%LWFM_CONFIG%`, then `%USERPROFILE%\.config\lwfm\config.lua`, then `%APPDATA%\lwfm\config.lua`. Without a file, the built-in defaults apply. Errors are reported with file and line; a broken config never prevents startup, and a broken reload keeps the previous config. Edits are picked up automatically (hot reload). New commands `edit-config` (creates a commented starter file if none exists and opens it in the configured editor) and `reload-config`. New CLI flag `--check-config` validates and exits with a status code.
- **Settings:** `show_hidden`, `sort`, `colors` (directory and hidden-item text), `editor`, `keys`, `openers`. Unknown keys produce a warning naming them.
- **Keymap from config:** the built-in Explorer keymap becomes an embedded Lua preset (`lwfm.preset("explorer")`, the only preset in this change). The user's bindings override it per scope and chord. Binding values can be a command string, a list of command strings run in order, a Lua function, or a table with `run` and `desc`. `"noop"` unbinds.
- **Key sequences and hint bar:** bindings of several chords in a row (`"ctrl+g d"`). While a sequence is pending, the status bar lists the possible next keys with their descriptions; Escape cancels. A binding that is a prefix of another binding in the same layer is a load error; there is no timeout.
- **Find as you type:** in the list, typing a printable character that is not bound opens a small search input over the list and selects the first entry whose name contains the typed text (case-insensitive substring). The list is not filtered; the matched text is highlighted in every matching name. Down/Up jump to the next/previous match, Enter opens the selected entry, Escape closes the input and keeps the selection. This replaces the native prefix type-to-find. **BREAKING** (behavior): typing in the list matches anywhere in the name and shows an input, instead of jumping by prefix silently.
- **Lua scripting API:** functions receive an `fm` object with the current state (`dir`, `file`, `selection`) and one method per registry command (`fm:cd(...)`, `fm:sort("size", { reverse = true })`), plus `fm:cmd(str)` and `fm:notify(msg)`. Events: `lwfm.on("startup", fn)` and `lwfm.on("dir_enter", fn)`. User modules load with `require` from `<config dir>\lua\`. Runaway scripts are aborted after a time limit and reported, never freezing the window.
- **External commands:** a `shell` command runs a command line with `$f` (focused file), `$fs` (selection), `$d` (current directory) substituted and quoted, without blocking the UI. `--console` gives the process a console window for terminal programs. **Openers:** `open` on a file uses the most specific matching pattern from `openers` (a command string or Lua function) before falling back to the Windows association.
- **Command prompt:** `:` in the list (or typing a leading `:` in the path bar) turns the path bar into a command input. Enter runs the command string, Escape cancels.
- **New registry commands:** `shell`, `search`, `search-next`, `search-prev`, `search-close`, `prompt`, `edit-config`, `reload-config`.

### Non-goals (deferred to later changes)

- Commander keymap preset (F5 copy to other pane and friends) -> `dual-pane-layout`, once the commands it binds exist.
- File operation commands that keys and Lua can call (copy, move, delete, rename, mkdir) -> `file-operations`.
- Async job API for Lua (`lwfm.spawn` with callbacks), custom columns and previewers, more events (selection, task done) -> a later scripting change after `file-operations`.
- Plugin manager or sandboxing of third-party Lua code -> not planned; config code runs with full user rights like Neovim.
- Configurable columns, fonts and layout, dark mode -> deferred; dark mode has no change assigned yet.
- Fuzzy matching, recursive search, and filtering the list to matches -> deferred, no change assigned.
- IPC / remote control (`lwfm --remote`) -> deferred, no change assigned.
- Vim preset -> dropped by decision; users can bind keys themselves in `config.lua`.

## Capabilities

### New Capabilities
- `lua-config`: config file discovery, evaluation of `config.lua`, the explorer preset, settings and their validation, error reporting, hot reload, `edit-config`, `reload-config`, `--check-config`.
- `lua-scripting`: the `fm` object and its command methods, Lua function bindings, events, `require` of user modules, time limit for scripts.
- `list-search`: find as you type: the search input, jumping to matches, highlighting, keys while searching, interaction with navigation and selection.
- `external-commands`: the `shell` command with `$f`/`$fs`/`$d` substitution and console option, and openers for `open`.
- `command-prompt`: the `:` prompt in the path bar.

### Modified Capabilities
- `key-input`: keymap comes from configuration (preset plus user overrides) instead of a fixed table; a new `search` scope for the search input; key sequences with hint bar; unbound printable keys in the list open the search input instead of reaching native type-to-find; the default keymap gains `ctrl+,`, `:` and search keys.
- `command-registry`: the built-in command set grows by the new commands listed above.
- `navigation`: `open` on a file consults configured openers before the Windows association.

## Impact

- New dependency: `mlua` (features `lua54`, `vendored`), which compiles Lua from C sources with the MSVC toolchain. Measured binary growth is about 600 KB (300 KB -> 914 KB release).
- New `windows` crate features: `Win32_System_Console` (`AttachConsole` for `--check-config` output from a GUI-subsystem binary). Process creation for `shell` uses `Win32_System_Threading`, which is already enabled.
- Code: new `config/` module (discovery, loading, preset, schema validation, watcher), `lua/` bindings, keymap rework into per-scope tries with Lua actions, the search input control with match highlighting, path bar prompt mode. `keys::keymap::DEFAULT_KEYMAP` is replaced by the embedded `explorer.lua` preset.
- Startup gains a config evaluation step; it must stay inside the 100 ms startup budget.
