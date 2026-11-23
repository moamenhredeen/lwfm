# Tasks

## 1. Setup

- [x] 1.1 Add `mlua` 0.12 with features `lua54` and `vendored`, and the `windows` feature `Win32_System_Console`; verify `cargo build` and `cargo build --release` succeed and record the release binary size
- [x] 1.2 Create the module skeleton from design D2 (`config/`, `lua/`, `search.rs`, `shell.rs`); verify `cargo build` and `cargo test` pass

## 2. Pure cores

- [x] 2.1 Implement `config::discover` (lookup order over an injected env/exists snapshot, missing explicit path reported); verify unit tests for each scenario in lua-config "Config discovery"
- [x] 2.2 Implement `search::Query` matching with `find(name)` returning the matched UTF-16 range (correct even when lowercasing changes the length), and remove the query filter and the `unfiltered` count from `Model::rebuild`; verify unit tests for case-insensitive substring, the ranges in `Reports` and `prep.txt` for `rep`, a length-changing lowercase character, and no match
- [x] 2.3 Implement `shell` expansion (`$f`, `$fs`, `$d`, `$$`, quoting, error when nothing focused) and the opener glob matcher (`*`, `?`, case-insensitive, longest pattern wins); verify unit tests for each external-commands scenario
- [x] 2.4 Add `registry::compile_parts(name, positional, flags)` sharing validation with `compile`, and add commands `shell`, `search`, `search-next`, `search-prev`, `search-close`, `prompt`, `edit-config`, `reload-config` to the registry and `Target`; verify registry unit tests (signatures, `search rep`, `shell` without argument errors)

## 3. Keymap rework

- [x] 3.1 Rework `keys::keymap` into per-scope tries with `Binding { action, desc }`, add the `Search` scope, and lookup returning `Match | Prefix(hints) | None` over a caller-given scope chain; verify unit tests for single chords, two-chord sequences, prefix hints, and scope chains
- [x] 3.2 Implement keymap building from preset plus user layers: normalized-duplicate and prefix-conflict errors within the user layer, user bindings shadowing conflicting preset bindings, `noop` removal, chained command arrays; verify unit tests for each key-input "Configurable keymap" and "Key sequences" conflict scenario

## 4. Lua runtime and config loading

- [x] 4.1 Implement `lua` state creation (safe libs, `package.path` to `<dir>\lua`, `lwfm` in `package.loaded`), the instruction-count time limit (50 ms calls, 1 s config), and first-line error formatting; verify unit tests: an endless loop is aborted within the limit, `require` finds `lua\mod.lua` in a temp dir
- [x] 4.2 Write `explorer.lua` (default keymap from the key-input spec, including the search input bindings `esc`, `enter`, `down`, `up`, plus default settings) and `lwfm.preset`; verify a unit test that the preset loads, validates, and yields every binding in the "Default keymap" table
- [x] 4.3 Implement `config::schema` validation (types, sort keys, `#rrggbb` colors, editor command, dotted error paths, unknown-key warnings) and `lwfm.on` with event-name validation; verify unit tests for each lua-config validation scenario and the unknown-event scenario
- [x] 4.4 Implement `config::load(path)` producing `Loaded` (settings, keymap, openers, handlers, warnings) and the wrong-return-value / unknown-preset errors; verify unit tests running small config scripts from a temp dir, including `config.lua:7` line numbers in errors
- [x] 4.5 Write `starter.lua` and verify a unit test that it loads with no errors or warnings

## 5. Lua API

- [x] 5.1 Implement the scoped `fm` userdata (`dir`, `file`, `selection`, `notify`, `cmd`, command methods with `-` -> `_` and trailing flag table) calling through `compile_parts`; verify unit tests against a recording `Target` for `fm:sort("size", { reverse = true })`, `fm:toggle_hidden()`, and a command error surfacing as a Lua error

## 6. App integration

- [x] 6.1 Parse `--config`, `--check-config` and the start directory in `main`; load the config before creating the window, apply settings (hidden, sort, colors) and keymap, fall back to defaults with the error in the status bar; verify manually with no config, a valid config, a syntax-error config, and `--config` pointing at a missing file
- [x] 6.2 Implement `--check-config` with `AttachConsole(ATTACH_PARENT_PROCESS)` and exit codes; verify from PowerShell: valid config prints `config OK:` with exit 0, broken config prints the error with exit 1
- [x] 6.3 Hold the config as `RefCell<Rc<Loaded>>`, run bindings by cloning the `Rc`, and execute `Action::Commands` chains and `Action::Lua` via `fm`; report Lua errors and timeouts in the status bar; verify manually with a function binding, an `error("boom")` binding, and an endless-loop binding (window stays responsive)
- [x] 6.4 Implement pending key sequences in `translate_key` with the hint text in the status bar, Escape cancel and discard of non-matching chords; verify manually with `ctrl+g d` / `ctrl+g p` bindings: hints shown, Escape cancels, D navigates
- [x] 6.5 Implement find-as-you-type behavior: unbound printable chars in the list run `search <char>`, `EN_CHANGE` jumps to the first match, `search-next`/`search-prev` with wrap-around, Enter/Escape/empty text/focus loss close the input, status shows the match count or no matches, navigation closes it and refresh/toggle-hidden/sort keep it; verify each list-search scenario manually in a fixture directory
- [x] 6.6 Implement the `:` prompt (`prompt` command, `path-submit` handling of a leading `:`); verify each command-prompt scenario manually
- [x] 6.7 Implement `shell` launching on worker threads (`cmd.exe /d /s /c`, hidden with exit-code reporting, `--console`) and openers in `open`; verify manually: `shell "code $d"` opens without a console, `shell --console "cmd /k echo $f"` shows a console, `shell "doesnotexist"` reports a non-zero exit, an `*.md` opener runs instead of the association
- [x] 6.8 Fire `startup` once after the first complete listing and `dir_enter` after each completed navigation (not refresh); verify manually with handlers that call `fm:notify` and the Downloads sort example
- [x] 6.9 Implement the `config-watch` thread (directory watch with subtree, 100 ms debounce, read on the worker, `ConfigChanged` event), reload on the UI thread with atomic swap, `Config reloaded` / error messages, and `reload-config`; verify manually: saving from Notepad and from VS Code reloads within one second, a broken save keeps old bindings, editing `lua\*.lua` reloads
- [x] 6.10 Implement `edit-config` (create `%APPDATA%\lwfm\config.lua` from `starter.lua` on a worker if no config exists, switch the watcher to it, run the `editor` command with `$f`); verify manually with no config present (`ctrl+,` creates and opens it in Notepad) and with an existing config
- [x] 6.11 Add `ui::searchbox` (hidden child Edit over the list's bottom-left corner, positioned in `layout`, font and size follow DPI) and match highlighting in the Name column via subitem custom draw (yellow match background, correct for selected rows, default drawing elsewhere); verify with screenshots that all matches are highlighted and highlights disappear when the input closes

## 7. Performance and wrap-up

- [x] 7.1 Add the trace lines `config loaded in X ms` and `search: X ms`; measure with a release build: starter config load under 10 ms, cold start to first listing of a 1,000-entry directory under 100 ms, 10,000-entry listing painted under 50 ms after enumeration, and search update (jump + highlight) for 10,000 entries under 50 ms
- [x] 7.2 Re-run the core-window-listing manual checks affected by the keymap rework (navigation keys, path bar, AltGr in path bar, dead keys, Alt+F4) plus AltGr and dead keys in the search input, and verify they pass
- [x] 7.3 Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test`; verify all pass
