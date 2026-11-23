# Design

## Context

See proposal.md for motivation and specs/ for the required behavior. Current state after `core-window-listing`:

- `keys::keymap::Keymap` is a flat `HashMap<(Scope, Chord), Invocation>` built from the static `DEFAULT_KEYMAP` table, with scopes `Global`, `List`, `Input`.
- `App` (UI thread, `Cell`/`RefCell` state) resolves keys in `translate_key` from the message loop and implements `cmd::registry::Target`. Every action is a `CommandDef` in a static table, compiled from strings by `registry::compile`.
- Worker results reach the UI thread through `mpsc` plus a `WM_APP_WAKE` message (`Event` enum in `app.rs`).
- `fs::model::Model` holds entries and a `visible` index (hidden filter plus sort). Native ListView type-to-find is served by `LVN_ODFINDITEMW`.
- `ui::window::run` creates the window, then `App::start` begins the first navigation.

## Goals / Non-Goals

**Goals:**
- One config path from text to behavior: `config.lua` is evaluated into a `Config` value (settings, keymap, openers, handlers) that is swapped atomically on reload.
- Lua is just another client of the command registry; no action exists only in Lua.
- No disk IO on the UI thread for config reloads; startup reads the config before the window exists.
- A misbehaving script can cost at most its time limit, never the window.

**Non-Goals:**
- Running Lua off the UI thread, or async Lua APIs (`lwfm.spawn`).
- Persisting runtime changes (toggle-hidden, sort clicks) back into the config.
- Sandboxing: the config runs with full user rights.

## Decisions

### D1. `mlua` 0.12 with `lua54` + `vendored`
`mlua` gives safe Rust bindings: values, functions, userdata, `scope` for non-`'static` borrows, and instruction-count hooks. The `windows` crate has no scripting runtime, and binding the Lua C API by hand would mean writing everything `mlua` provides (stack discipline, error and panic safety, GC-rooted references). `vendored` compiles Lua 5.4 from source with the MSVC toolchain already used, so there is no `lua54.dll` to ship. Alternatives: LuaJIT (5.1 dialect, harder MSVC build, speed not needed because Lua stays off hot paths); Luau (built-in sandboxing we do not need, less familiar). The `Lua` state is `!Send` and lives on the UI thread only.

### D2. Module layout
```
src/config/
  mod.rs        Config { settings, keymap, openers, handlers, lua }, load(path) -> Result<Loaded, ConfigError>
  discover.rs   lookup order (--config, LWFM_CONFIG, ~/.config, APPDATA), pure given an env snapshot
  schema.rs     table -> Settings validation, unknown-key warnings, "#rrggbb" parsing
  watch.rs      ReadDirectoryChangesW watcher thread
  explorer.lua  embedded preset (include_str!)
  starter.lua   embedded commented starter config
src/lua/
  mod.rs        state creation, package.path, time-limit hook, error formatting
  api.rs        `lwfm` module (preset, on), `fm` userdata (fields, command methods, cmd, notify)
src/keys/keymap.rs   per-scope tries of Action, lookup -> Match | Prefix(hints) | None
src/search.rs        query matching and match ranges for highlighting (pure)
src/ui/searchbox.rs  the find-as-you-type Edit control over the list
src/shell.rs         $f/$fs/$d expansion (pure), process launch, opener pattern matching (pure)
```

### D3. Threads and how results reach the UI thread

| work | thread | path back to the UI thread |
|---|---|---|
| startup config read and evaluation | main thread, before the window is created | direct return value (no window exists yet that could freeze) |
| watching the config dir, debounce, reading the changed file | `config-watch` worker (one, long-lived) | `Event::ConfigChanged { path, source: Result<String, String> }` via the existing channel and `WM_APP_WAKE` |
| evaluating a reloaded config, all Lua calls (bindings, openers, events) | UI thread | none needed |
| `edit-config`: creating the starter file | short-lived worker | `Event::Message` on error, then the editor command runs as a `shell` launch |
| `shell` / openers: `CreateProcessW`, waiting for exit of hidden processes | short-lived worker per launch | `Event::Message` with a non-zero exit code or a launch error |
| `--check-config` | main thread, no window | process exit code |

Lua's own `require` reads module files on the UI thread during a reload. This is accepted because modules live next to the config, are small, and are local; it is listed as a risk.

### D4. Config value and atomic swap
```
Loaded {
  lua: mlua::Lua,                  // state that owns all functions below
  settings: Settings,              // show_hidden, sort, colors, editor (Invocation)
  keymap: Keymap,                  // tries of Action
  openers: Vec<(Pattern, Action)>, // sorted by pattern length, longest first
  handlers: HashMap<Event, Vec<mlua::Function>>,
  warnings: Vec<String>,
}
App.config: RefCell<Rc<Loaded>>
```
Reload builds a complete new `Loaded` in a fresh `Lua` state, then replaces the `Rc`. Callers clone the `Rc` before running an action. A binding that triggers `reload-config` therefore keeps its own (old) state alive until it returns, so a borrow is never invalidated underneath a running Lua call. On failure the old `Rc` stays.

`Action = Commands(Vec<Invocation>) | Lua(mlua::Function)`, plus an optional `desc`. Command strings are compiled when the config loads, so a typo is a load error with the key text, not a runtime surprise.

### D5. Evaluation and the `lwfm` module
- `Lua::new()` (safe standard libraries, no `debug`). `package.path` is set to `<dir>\lua\?.lua;<dir>\lua\?\init.lua`. `package.loaded.lwfm` is set to the module table, so `require("lwfm")` never touches disk.
- `lwfm.preset("explorer")` evaluates the embedded `explorer.lua` chunk and returns its table. It is a fresh table on each call, so users mutate their copy.
- `lwfm.on(name, fn)` validates the name (`startup`, `dir_enter`) and appends to a Lua-side registry table that is read after the config returns.
- The chunk is loaded with the file's path as chunk name, so Lua errors read `config.lua:7: ...`. Errors are shortened to their first line for the status bar.

### D6. Settings validation as a schema walk
`schema.rs` walks the returned table against a static schema (key -> expected kind). Wrong kinds are errors with a dotted path. Keys not in the schema are collected as warnings. `keys` and `openers` are validated by the keymap and opener builders, which report the offending key text. The preset's table is valid by construction; a unit test loads it.

### D7. Keymap as per-scope tries
```
Keymap { scopes: HashMap<Scope, Node> }
Node { action: Option<Binding>, children: HashMap<Chord, Node> }
lookup(scope_chain, &[Chord]) -> Match(&Binding) | Prefix(Vec<(Chord, desc)>) | None
```
- Scopes: `Global`, `List`, `Search`, `Input`. The chain follows the focused control: `[List, Global]` for the list, `[Search, Global]` for the search input, `[Input, Global]` for the path bar.
- Building: the preset table is inserted first, then the user table. A user insert first removes every preset binding in that scope that is a prefix of the new key text or has it as a prefix, then inserts. The user table itself is checked for prefix conflicts and normalized duplicates, which are errors.
- Pending sequence: `App.pending: RefCell<Vec<Chord>>`. In `translate_key`, while pending is non-empty, every chord is consumed. Escape clears it; a match runs the action; a further prefix updates the hints; anything else clears it and is discarded. The hint text goes to the status bar message part and is restored when the sequence ends. No timer: this follows the no-timeout decision and adds no latency.

### D8. Find as you type with a search input
- **Control:** `ui::searchbox` is a hidden single-line Edit, a child of the main window (a sibling of the list, so its `EN_CHANGE` and `EN_KILLFOCUS` notifications reach the main window procedure). It is shown on top of the list's bottom-left corner, about 240 px wide at 96 DPI, and is repositioned in `layout`. A real Edit gives caret, selection, Backspace, AltGr and dead keys for free. The alternative, a text buffer drawn in the status bar, would need all of that hand-written.
- **Opening:** in `translate_key`, when the focus is the list, nothing is pending, and a `WM_CHAR` chord is unbound in `[List, Global]`, run `search <char>` and consume the key. `search <text>` shows the box, sets its text, puts the caret at the end and focuses it. Native `LVN_ODFINDITEMW` handling stays removed.
- **Jumping:** on `EN_CHANGE` the app reads the box text into a `search::Query`, scans `model.visible` in display order for the first name that matches, and selects and focuses it (`list::clear_selection`, `select`, `focus`, which scrolls it into view). It also counts the matches for the status bar. Empty text closes the box. `search-next` and `search-prev` scan forward or backward from the focused index with wrap-around. The model is not filtered: `Model::rebuild` no longer knows about search.
- **Matching:** `Query::find(name) -> Option<Range<usize>>` lowercases the decoded UTF-16 name with `char::to_lowercase` and keeps the UTF-16 offset of the original character for each lowered char, so the match range maps back to the displayed name even when lowercasing changes the length. It is pure Rust and unit-tested. The alternative `FindNLSStringEx` costs a Win32 call per entry and gives nothing extra for file names.
- **Highlighting:** while the box is open, `NM_CUSTOMDRAW` returns `CDRF_NOTIFYSUBITEMDRAW` at item stage. At subitem stage for column 0, when the name matches, the app paints the cell itself: it gets the label rect with `LVM_GETSUBITEMRECT(LVIR_LABEL)` and fills it with the row background (the system highlight color for selected rows). It then draws the text in three runs with `DrawTextW`: before the match, the match on a yellow background, and after the match. It returns `CDRF_SKIPDEFAULT`. Non-matching rows and other columns keep default drawing, so only visible rows pay the cost.
- **Closing:** `search-close` hides the box, clears its text without running the jump, returns focus to the list and invalidates the list to remove highlights. `EN_KILLFOCUS` (for example a click in the list) closes it the same way; a guard flag prevents re-entry when the app moves focus itself. Navigation commit closes it. `refresh`, `toggle-hidden` and `sort` only rebuild; the name-based selection restore keeps the match selected, the match count is recomputed, and the repaint updates the highlights.
- **Status:** `Search: <q>  <n> matches` (or `no matches`) in part 0 while the box is open.

### D9. Command prompt via the path bar
No new control. `prompt` focuses the path bar, sets the text to `:` and puts the caret at the end. `path-submit` checks for a leading `:`: it runs `registry::run_str` on the rest, keeps the text on error, and on success restores the directory text and focuses the list. Because commands run through the registry, Lua and keys have no special prompt path.

### D10. `shell` and openers
- Expansion (pure): scan for `$f`, `$fs`, `$d`, `$$`. Paths are wrapped in double quotes; embedded quotes cannot occur because `"` is invalid in Windows paths.
- Launch on a worker thread: `CreateProcessW` with application `%ComSpec%` (fallback `cmd.exe`) and command line `/d /s /c "<expanded>"`, `lpCurrentDirectory` = current dir. Hidden: `CREATE_NO_WINDOW`; then `WaitForSingleObject` plus `GetExitCodeProcess`, reporting non-zero codes. `--console`: `CREATE_NEW_CONSOLE` without waiting, because the user sees the console. `cmd /c` is used rather than `CreateProcessW` on the program directly, because many tools are `.cmd` shims (`code.cmd`) and users expect PATH and builtins to work.
- Openers: patterns are compiled to a simple case-insensitive glob matcher (`*`, `?`) and sorted by pattern length, descending. `open` on a file takes the first match; a command action runs through the registry with the focused file as context, and a Lua action runs as in D11. No match falls back to the existing `ShellExecuteExW` path.

### D11. The `fm` userdata and the time limit
- Lua calls use `lua.scope(|s| ...)` to create a scoped userdata borrowing `&App`, so `fm` cannot outlive the call and no `'static` or `Rc<App>` cycle is needed. The fields `dir`, `file` and `selection` are computed when the userdata is created.
- The `__index` metamethod resolves `notify`, `cmd`, and any registry command name with `_` mapped to `-`. A command method converts its arguments: strings become positional, and a trailing table becomes flags (`true` -> `--name`, string -> `--name=value`). It then validates through the same compile path as strings (a new `registry::compile_parts`), and runs with the result mapped to a Lua error on failure.
- Time limit: before each top-level call, set `HookTriggers::new().every_nth_instruction(10_000)` with a callback that errors once `Instant::now()` passes the deadline (50 ms for calls, 1 s for config evaluation); remove the hook after the call. Checking every 10k instructions costs well under 1% and catches loops within a few microseconds of the deadline. A Rust command called from Lua (e.g. `fm:cd`) is not interrupted, but those commands never block (D3).

### D12. Watcher
One `config-watch` thread opens the config directory with `FILE_LIST_DIRECTORY` and loops `ReadDirectoryChangesW` (synchronous, `bWatchSubtree = TRUE`, filter last-write and file-name). Events for the config file name or for anything under `lua\` start a 100 ms debounce. Then the thread reads the config file and sends `ConfigChanged`. Identical config content within 500 ms of the previous report comes from events buffered during the debounce and is dropped; changes under `lua\` always report, because the config text itself does not change. Watching the directory instead of the file handles save-by-rename. When the active config path changes (first `edit-config`), the old watcher is told to stop through a flag (its thread exits at its next wake-up), and a new one starts.

### D13. Command line
`main` parses `--config <path>`, `--check-config` and one optional directory. With `--check-config`, the GUI-subsystem binary calls `AttachConsole(ATTACH_PARENT_PROCESS)`, so `println!` reaches the calling terminal, then loads the config, prints the result and exits without creating a window.

### D14. Tracing
New trace lines: `config loaded in X ms (<path>)` at startup and reload, and `search: X ms (<n> matches)` per search text change. These measure the new performance requirements.

## Risks / Trade-offs

- [`require` of user modules reads files on the UI thread during reload; a module on a slow network share would stall the window] -> Modules are expected next to the config on a local disk; the time limit bounds Lua execution but not a blocked file read. Revisit by pre-reading `lua\` on the watcher thread if this becomes real.
- [Lua running on the UI thread can add up to 50 ms per call before it is cut off] -> This is the stated limit; heavy work belongs in `shell` commands, and an async API is deferred.
- [Hidden `shell` processes are waited on by one thread each; a long-running hidden process keeps its thread] -> Threads are cheap and exit with the process; lwfm exiting does not wait for them (`process::exit`).
- [`cmd /c` quoting rules are subtle] -> `/s` plus wrapping the whole line in quotes is the documented way to pass the line unchanged; unit tests cover expansion, and manual tests cover paths with spaces.
- [Typing now matches anywhere in the name and shows an input, unlike Explorer's silent prefix jump] -> Requested behavior (Directory Opus style); the proposal marks it as breaking.
- [Self-drawn Name cells while searching must match the theme's selection and text colors] -> Only matching cells in column 0 are self-drawn, and only while the input is open; everything else keeps default drawing.
- [Binary size grows by about 600 KB (measured: 300 KB -> 914 KB) and the build needs a C compiler for Lua] -> Accepted for a scripting runtime; the MSVC toolchain is already required for the Rust target.
- [Reload resets runtime toggles (hidden, sort) to configured values] -> Specified behavior; it keeps the config the single source of truth.

## Migration Plan

No user data exists yet. The hardcoded `DEFAULT_KEYMAP` is replaced by `explorer.lua` with the same bindings plus the new ones, so behavior without a config file stays the same except for find-as-you-type.

## Open Questions

- Exact content and comments of `starter.lua`. This is wording, not behavior; it must return the explorer preset and show one example per setting.
