# Design

## Context

See proposal.md for motivation. Facts that shape the approach:

- Win32 is used in `main.rs`, `trace.rs`, `shell.rs`, `config/watch.rs`, `fs/enumerate.rs`, `fs/entry.rs`, `fs/model.rs`, `keys/normalize.rs`, `ui/*` and `app.rs`. The first eight are leaf calls with no window; the last two are window plumbing.
- The window procedure is re-entrant: `SendMessageW` to the list calls back synchronously (`LVN_GETDISPINFO`, `LVN_ITEMCHANGED`), and setting edit text raises `EN_CHANGE`. Application state therefore lives in `Cell`/`RefCell` and borrows must not be held across such calls.
- `keys/normalize.rs` already takes the layout lookup as a function argument, and `config/discover.rs` takes the file-exists check the same way. Injection of system calls is an established pattern here.
- Performance budgets: first painted listing under 100 ms, 10,000 entries painted under 50 ms, search update under 50 ms.
- Decisions taken with the user before this change: stay on raw Win32; sub crate instead of winsafe; use-case traits for controls; one change.

## Goals / Non-Goals

**Goals:**

- `lwfm` compiles with `#![forbid(unsafe_code)]` and without the `windows` crate.
- Every `unsafe` block is in `lwfm-win`, does one thing and carries a `// SAFETY:` comment.
- No user-visible behavior change except the two spec deltas.
- No regression of the performance budgets.

**Non-Goals:**

- A general Win32 wrapper. `lwfm-win` wraps only what lwfm uses and is shaped by lwfm's use cases.
- An escape hatch. The crate offers no `raw_hwnd()` or `send_message()`; a missing capability is added to the crate.
- Changing the command registry, the keymap, Lua or the navigation model.

## Decisions

### D1. Workspace layout

The root `Cargo.toml` is both workspace root and the `lwfm` package; `crates/lwfm-win` is the only other member. `src/` stays where it is.

Alternative: move the binary to `crates/lwfm`. Rejected: it renames every file for no gain and makes history harder to follow.

### D2. Boundary rule and how it is enforced

1. `lwfm` has no `windows` dependency. A `windows` type cannot appear in application code because it cannot be named.
2. `lwfm` declares `#![forbid(unsafe_code)]`.
3. `lwfm-win` re-exports nothing from `windows` and uses its own small types in signatures: `Rect`, `Size`, `Color`, `Dpi`, `ListId`, `EditId`, `TimerId`, `LocalTime`, `Error`. One deliberate exception: `impl From<windows::core::Error> for Error`, so that `?` works inside the crate. It is a public trait impl, but the application cannot name its source type without depending on `windows`.
4. `lwfm-win` enables `clippy::undocumented_unsafe_blocks` and `clippy::multiple_unsafe_ops_per_block` as errors.

Alternative: a `win` module inside the single crate. Rejected: nothing would stop a later change from importing `windows` elsewhere.

### D3. Crate modules and thread affinity

| Module | Content | Thread |
|---|---|---|
| `window` | class registration, window creation, message loop, `Handler`, `WindowEvent`, timers, title, client size, `WindowToken` | UI thread |
| `controls` | `ListView` + `ListSource`, `Edit` + `EditEvent`, `StatusBar` | UI thread |
| `gdi` | font, brush and device-context guards, text measuring and drawing, system colors (crate-private except `Color`) | UI thread |
| `input` | `KeyEvent`, `ModState`, layout character lookup | UI thread |
| `thread` | `UiWaker` | any thread |
| `fs` | directory listing (bulk with find fallback), `DirWatcher`, long-path encoding | worker threads |
| `process` | `spawn_cmd`, `Child::wait`, `shell_open` | worker threads |
| `text` | natural name comparison, FILETIME to `LocalTime` | any thread |
| `sys` | console attach, debugger output, time since process start | any thread |

UI-thread types hold a raw window handle and are therefore not `Send`; the compiler keeps them on the UI thread. `UiWaker` and `WindowToken` are the only window-related types that are `Send`; they wrap the handle in a newtype with an `unsafe impl Send` justified by `PostMessageW` and `ShellExecuteExW` accepting a window handle from any thread.

No work changes thread in this change. Enumeration, config watching, process waiting, shell open and config file IO stay on worker threads spawned by the application with `std::thread`; they call blocking functions of `lwfm-win` and report through the application's `mpsc` channel followed by `UiWaker::wake()`, which posts one coalesced wake message. The UI thread receives `WindowEvent::Wake` and drains the channel. The generation counter stays in the application.

### D4. Window and handler

```
let window = Window::create("lwfm")?;
let list   = ListView::new(&window, ListId(1), &COLUMNS)?;
...
let app = App::new(window, controls, startup);
app.window().run(&app)          // returns the exit code
```

The window's user data holds a reference-counted `WindowState` (font, DPI, registered controls), attached in `Window::create` and released on `WM_NCDESTROY`. `run` stores a pointer to the handler in that state for the duration of the loop and clears it before returning; the handler is borrowed for exactly that time, so the application needs no `Rc` and no thread-local. While no handler is attached the mechanical handling still runs and events are dropped.

```rust
pub trait Handler {
    fn on_event(&self, event: WindowEvent);
    /// Called before a keyboard message is dispatched; `true` consumes it.
    fn on_key(&self, key: KeyEvent) -> bool;
    fn list_source(&self, id: ListId) -> &dyn ListSource;
    fn on_edit(&self, id: EditId, event: EditEvent);
}

pub enum WindowEvent {
    Resized, DpiChanged { old: Dpi, new: Dpi }, Timer(TimerId),
    Wake, SettingsChanged, Destroyed,
}
```

The crate does the mechanical part of each message itself before raising the event: on a DPI change it applies the suggested window rectangle, recreates the font, reassigns it to all controls and rescales the list columns; on a settings change it recreates the font; on activation it restores focus to the child that had it. The application's `last_focus` field and its `apply_font` function disappear. The application only lays out its controls.

Alternative considered: one typed enum for all messages including list notifications. Rejected in exploration: the application would still have to drive the custom-draw stages and do the painting.

### D5. List by use case

```rust
pub trait ListSource {
    fn cell(&self, row: usize, column: usize, out: &mut CellBuf<'_>);
    fn row_color(&self, row: usize) -> Option<Color> { None }
    /// UTF-16 range of the Name cell to highlight.
    fn highlight(&self, row: usize) -> Option<Range<usize>> { None }
    fn on_activate(&self, row: usize) {}
    fn on_column_click(&self, column: usize) {}
    fn on_selection_changed(&self) {}
}
```

- `CellBuf` writes UTF-16 or `&str` straight into the buffer the control supplied. The per-cell `Vec<u16>` of `cell_text` goes away.
- The crate owns the `NM_CUSTOMDRAW` stages and return codes. It asks for sub-item notifications only while some row has reported a highlight since the last invalidation request (`ListView::set_highlighting(bool)`, set by the application when search opens and closes), so the common case stays on the default paint path.
- Match painting (today `App::paint_match`) moves into the crate. The brush is created once and reused. In a high-contrast theme the system highlight background and text colors are used; otherwise the current yellow with the row's text color.
- Routing is by `Handler::list_source(id)`. A second pane later returns a different source for a different id; no ownership cycle between list and source.

`ListView` methods mirror today's `ui::list` functions: `set_count`, `focused`, `selected`, `selected_count`, `select`, `focus`, `select_only`, `clear_selection`, `invalidate`, `redraw_now`, `has_focus`, `focus_control`, `set_bounds`.

### D6. Edit and status bar

`Edit` replaces `ui::pathbar` and `ui::searchbox` (which today share `pathbar::text` and duplicate `create`): `text`, `set_text`, `select_all`, `set_caret`, `show_on_top`, `hide`, `is_visible`, `has_focus`, `focus_control`, `set_bounds`. `EditEvent` is `Changed` or `LostFocus`. `StatusBar` keeps `layout` and `set_part`.

### D7. No echo of programmatic changes

Notification callbacks (`on_edit`, `on_selection_changed`, `on_activate`, `on_column_click`) report user-caused changes only. While a mutating crate method runs, a per-window quiet counter (RAII guard) suppresses them. Data callbacks (`cell`, `row_color`, `highlight`) are never suppressed and may be called synchronously inside any crate method.

One exception, found while switching the application: `EditEvent::LostFocus` is reported whatever moved the focus, including `focus_control()` of another control, because the application relies on it (running `focus-path` while the search input is open must close the search, as before). Only `Edit::hide()` suppresses it. The application tracks whether the search is open in its own state, so the focus loss it causes itself while closing is ignored without a guard flag around the control calls.

Consequences in the application:

- `search_guard` is replaced by `search_open`, which is state (is the search active) and not a re-entrancy guard. `App::search` calls its own "search text changed" function after `set_text` instead of relying on `EN_CHANGE`.
- `rebuild`, `on_search_changed` and `close_search` already refresh the status bar themselves. `step_match` does not and must do so after `select_only`.
- Restoring a large selection no longer refreshes the status bar once per item.

Alternative: keep echo and let the application guard. Rejected: every new control interaction would need another guard flag.

### D8. Keys

The message loop offers `KeyEvent` to `Handler::on_key` before dispatch:

```rust
pub enum KeyEvent {
    Down { key: KeyCode, mods: ModState },   // WM_KEYDOWN, WM_SYSKEYDOWN
    Char(char),                              // WM_CHAR
}
pub enum KeyCode { Named(NamedKey), Other(VirtualKey) }
```

`NamedKey` is a neutral enum (Enter, Esc, Tab, arrows, F1..F24, ...); `VirtualKey` is an opaque newtype with `layout_char()`. `keys/normalize.rs` keeps the AltGr and dead-key logic and its tests, and maps `NamedKey` to the keymap's `Named`. The virtual-key table moves into the crate.

Alternative: export virtual-key numbers as `u16` constants. Rejected: it leaks the Win32 numbering into the application for no saving.

### D9. Leaf functions

- `fs::list_dir(path, &mut dyn FnMut(RawEntry<'_>) -> ControlFlow<()>) -> Result<(), Error>`: bulk enumeration with fallback to find, long-path encoding inside. `RawEntry` borrows the name and exposes `size`, `mtime`, `is_dir`, `is_hidden`. The application's `Entry` stores two booleans instead of the raw attribute bits. Batching, generation checks and `FsMsg` stay in `src/fs/enumerate.rs`.
- `fs::DirWatcher::start(dir, on_change: impl FnMut(&[PathBuf]) + Send) -> Result<DirWatcher, Error>`: the crate owns the watcher thread and the directory handle. The read is overlapped and the thread waits on it together with a stop event that `Drop` signals; the thread then cancels its own read with `CancelIoEx` and waits for the cancellation before it frees the buffer, so it ends promptly and without a race between `Drop` and the start of a read. Debounce, relevance filter, re-read and duplicate suppression stay in `src/config/watch.rs`, running inside the callback on the watcher thread as today.
- `process::spawn_cmd(line, dir, console) -> Result<Child, Error>` and `Child::wait() -> Result<u32, Error>`; `process::shell_open(owner: WindowToken, file, dir) -> Result<(), Error>` initialises COM with a guard that uninitialises only after a successful initialisation, and treats user cancellation as success. All take `&Path`/`&OsStr` and encode without loss.
- `text::compare_natural(&[u16], &[u16]) -> Ordering`, `text::to_local(i64) -> Option<LocalTime>`.
- `sys::attach_parent_console()`, `sys::debug_output(&str)`, `sys::since_process_start()`.
- `Error` carries the Win32 code, implements `std::error::Error` with the system message as `Display`, and offers `kind()` (`NotFound`, `AccessDenied`, `NotADirectory`, `NotReady`, `NetworkUnreachable`, `Unsupported`, `Other`). `describe_error` in the application matches on the kind.

### D10. Resource ownership inside the crate

Every handle, find handle, font, brush and COM apartment is owned by a guard type with `Drop` (`windows::core::Owned` where the handle type supports it, a private guard otherwise). The window owns its font and the list its highlight brush; both are released when the window is destroyed.

### D11. Application structure

`src/app.rs` becomes `src/app/` with `mod.rs` (struct, construction, `Handler`), `list.rs` (`ListSource`, cell text), `keys.rs`, `nav.rs` (navigation, load events, selection restore), `search.rs`, `config.rs`, `commands.rs` (`Target`, `Host`), `status.rs`.

State is grouped; each group is one `RefCell` (or plain data where it never changes):

| Group | Fields today |
|---|---|
| `ConfigState` | `config`, `config_file`, `watcher`, `file_override` |
| `KeySeq` | `pending`, `hint` |
| `Listing` | `cwd`, `model`, `history`, `restore`, load state |
| `SearchState` | `search`, `search_matches` |
| `RebuildTimer` | `last_rebuild`, `timer_armed` |
| `TraceState` | `startup_traced`, `done_at`, `startup_fired` |

`load: Option<Load>`, `loading`, `loaded` and `Load::committed` become:

```rust
enum LoadState {
    Idle,
    Pending { path: PathBuf, nav: Nav },             // nothing received yet
    Streaming { nav_kind: NavKind, received: usize }, // committed, still loading
}
```

Borrow rule, unchanged in substance and now documented on the types: `Listing` and `SearchState` are read by `ListSource` callbacks, so a mutable borrow of either must end before any crate method is called. The callbacks use `try_borrow`; a conflict draws an empty cell in release builds and fails a `debug_assert!` in debug builds, so a violation is found in development instead of being hidden.

`Startup` is passed by value to `App::new`, which removes the throwaway `config::defaults()` call (a full Lua state) from the start path.

### D12. Errors

No error crate is added; the types are small and hand-written.

- `ParseError`, `CmdError`, `ChordError`: implement `std::error::Error`.
- `config`: `Result<_, String>` becomes `Result<_, ConfigError>` with variants `Read { path, source }`, `Script(String)` and `Setting { path, expected }`. `Display` reproduces today's texts exactly, because they are asserted by tests and described in the lua-config spec.
- `keys::keymap`: `Result<_, String>` becomes `Result<_, KeymapError>`, wrapped by `ConfigError`.
- Enumeration: `Result<(), u32>` becomes `Result<(), lwfm_win::Error>`.
- Thread spawn failure for enumeration is reported as a load error instead of a panic.

### D13. Lints and toolchain

`rust-toolchain.toml` pins the nightly in use. A `[workspace.lints]` table enables `clippy::pedantic` as warnings with a short allow list decided during the cleanup task (candidates: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`), plus the unsafe lints of D2 for `lwfm-win`. Integer conversions use `From`/`TryFrom` or one documented helper per pattern (`loword`, `hiword`, FILETIME to `i64`). Wildcard imports of `windows` modules are replaced by explicit lists.

### D14. Tests

- Existing unit tests move with their code (`to_api_wide`, enumeration against a temp directory).
- `lwfm-win` gets hidden-window tests: a list with a counting `ListSource` receives `cell` calls after `set_count`; programmatic `select` and `Edit::set_text` raise no notification (D7); a dropped `DirWatcher` ends its thread within one second.
- Application tests keep using the `Recorder` target; `normalize` tests switch to `KeyCode`.
- Window behavior beyond that stays manually verified against the spec scenarios.

### D15. New crate dependencies

None from crates.io. `lwfm-win` is a workspace member; `windows` 0.62 moves from the binary to it with the same feature list. `embed-manifest` and `mlua` stay with the binary.

### D16. Order of work

One change, eight task groups, each leaving the tree green (`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test`) and committed separately, so the history can be bisected. The `windows` dependency stays in the binary until the last Win32 call has moved (group 5).

## Risks / Trade-offs

- [Callbacks cross a crate boundary and a `dyn` call on the hottest path, cell text] → thin LTO is already enabled; the per-cell allocation is removed at the same time; budgets are measured before and after (tasks 1.4 and 8.1).
- [Suppressing echoed notifications (D7) could drop a status or search update that today happens by accident] → task 3.6 lists every caller of a mutating list or edit method and checks it against the list-search and main-window scenarios.
- [`ListSource` turns out too narrow for a later feature] → both crates are ours; adding a defaulted trait method is a small, local change.
- [Very large diff in one change] → eight separately committed, green task groups; groups 1 to 5 move code with minimal edits, restructuring of `App` starts only in group 6.
- [`try_borrow` fallback keeps hiding borrow conflicts in release builds] → `debug_assert!` in the same place, and all manual verification is done with a debug build first.
- [`WM_SETTINGCHANGE` is broadcast often and for unrelated settings] → the font is recreated only when the reported metrics differ from the current ones.

## Migration Plan

No data or configuration migration. Rollback is a revert of the task-group commits in reverse order; each group is independently revertible down to group 1.

## Open Questions

- Final clippy allow list (D13): decided while fixing the warnings; does not affect structure.
- Whether `windows::core::Owned` covers GDI handles in 0.62 or a private guard is needed (D10): either satisfies the design.

## Measurements

Release build, `LWFM_TRACE=1`, starter config, local SSD, median of 5 runs (single runs vary by about 20 %; the first run after a build is an outlier and is included).

| Measure | Budget | Baseline (before, `d0a25f1`) |
|---|---|---|
| Config load (starter config) | < 10 ms | 0.8 ms |
| Start to first painted listing, 1,000 entries | < 100 ms | 77.3 ms |
| Enumeration complete to painted, 10,000 entries | < 50 ms | 21.2 ms |
| Search update, 10,000 entries | < 50 ms | 2.5 ms |
| Release binary size | - | 915,456 bytes |
