# Tasks

Every group ends green (`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test`) and is committed on its own (design D16).

## 1. Workspace and guard rails

- [x] 1.1 Add `rust-toolchain.toml` pinning the nightly reported by `rustc --version`; verify `cargo build` uses it (`rustup show active-toolchain`)
- [x] 1.2 Make the root `Cargo.toml` a workspace with member `crates/lwfm-win` (empty library, `windows` 0.62 with the current feature list); the binary depends on it and keeps its own `windows` dependency for now; verify `cargo build` and `cargo test` pass
- [x] 1.3 Add `[workspace.lints]` (deny `unsafe_op_in_unsafe_fn`) and, in `lwfm-win`, `clippy::undocumented_unsafe_blocks` and `clippy::multiple_unsafe_ops_per_block` as errors; `clippy::pedantic` joins the table in 7.3, because `-D warnings` would turn it into errors before the cleanup; verify `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 1.4 Record the baseline with a release build and `LWFM_TRACE=1`: config load time, start to first painted listing (1,000 entries), enumeration complete to painted (10,000 entries), search update (10,000 entries), release binary size; verify the numbers are written into this change's `design.md` under a "Measurements" heading

## 2. Leaf modules in lwfm-win

- [x] 2.1 Add `lwfm_win::Error` (code, `kind()`, system message as `Display`, `std::error::Error`) and the private handle guards (file handle, find handle, COM apartment); verify unit tests for `kind()` mapping of access denied, not found, not a directory and network errors
- [x] 2.2 Move long-path encoding and directory listing (bulk with find fallback) into `lwfm_win::fs::list_dir` yielding `RawEntry`; keep batching, generation and `FsMsg` in `src/fs/enumerate.rs`; change `Entry` to `is_dir`/`is_hidden` booleans; report a failed thread spawn as a load error; verify the moved enumeration and `to_api_wide` tests pass and `src/fs/` no longer imports `windows`
- [x] 2.3 Add `lwfm_win::fs::DirWatcher` (own thread, `CancelIoEx` on drop, lossless path encoding) and reduce `src/config/watch.rs` to debounce, relevance, re-read and duplicate suppression; verify a crate test that a dropped watcher's thread ends within one second, and manually that saving `config.lua` and a file under `lua\` still reloads within one second
- [x] 2.4 Add `lwfm_win::process::{spawn_cmd, Child::wait, shell_open}` with lossless encoding of the working directory and `ComSpec` and correct COM pairing; rewrite `shell::launch` and `App::shell_open` on top; verify each external-commands scenario manually (`shell "code $d"`, `shell --console`, failing command, opener, association fallback)
- [x] 2.5 Add `lwfm_win::text::{compare_natural, to_local, LocalTime}` and `lwfm_win::sys::{attach_parent_console, debug_output, since_process_start}`; switch `fs/model.rs`, `fs/entry.rs`, `trace.rs` and `main.rs` to them; verify `cargo test` passes and `lwfm --check-config` still prints to the calling terminal with exit codes 0 and 1
- [x] 2.6 Verify with `grep -rn "windows::" src` that only `src/app.rs`, `src/ui/` and `src/keys/normalize.rs` still import `windows`; commit

## 3. Controls

- [x] 3.1 Add crate types `Rect`, `Size`, `Color`, `Dpi`, `ListId`, `EditId` and the private GDI guards (font, brush, device-context state) and system-color helpers including high-contrast detection; verify unit tests for `Color` to and from `#rrggbb` component order
- [x] 3.2 Implement `ListView` with the methods of design D5 and the `ListSource` trait with `CellBuf`; the crate handles `LVN_GETDISPINFO`, `LVN_COLUMNCLICK`, `NM_DBLCLK`, `LVN_ITEMCHANGED`/`LVN_ODSTATECHANGED`; verify a hidden-window test that `set_count(3)` followed by a redraw calls `cell` for rows 0..3 (a hidden window does not paint, so the test requests the cell text through the control instead of redrawing)
- [ ] 3.3 Move custom draw into the crate: row color, `set_highlighting`, match painting with a cached brush, and system highlight colors in high-contrast themes; verify the list-search "Match highlighting" scenarios manually, including with a Windows contrast theme switched on **Open:** implemented; the highlight was verified on screen in the normal theme (highlight pixels present while searching, none after closing). The run with a Windows contrast theme switched on is still to be done by hand.
- [x] 3.4 Implement `Edit` (replacing `ui::pathbar` and `ui::searchbox`) with `EditEvent`, and `StatusBar`; verify `cargo build` and that `src/ui/pathbar.rs`, `searchbox.rs`, `statusbar.rs` and `list.rs` are deleted
- [x] 3.5 Implement the quiet guard of design D7; verify hidden-window tests that programmatic `select`, `select_only`, `clear_selection` and `Edit::set_text` raise no notification, and that a simulated user selection change does
- [x] 3.6 Switch `App` to `ListSource` and `EditEvent`; remove `search_guard`; call the search-changed function explicitly from `search`; add the status refresh to `step_match`; go through every caller of a mutating list or edit method and verify each list-search scenario and the main-window "Status bar" scenarios manually; commit

## 4. Window, handler and keys

- [x] 4.1 Implement `Window::create`, `run(&handler)` with the handler pointer in the window's user data, `Handler`, `WindowEvent`, timers, title, client size and `WindowToken`; verify a hidden-window test that a posted wake reaches `on_event(Wake)` and that the handler is detached after `run` returns
- [x] 4.2 Move DPI handling, font creation and assignment, and focus restore on activation into the crate; remove `apply_font`, `last_focus` and the `WM_DPICHANGED`/`WM_ACTIVATE`/`WM_SETFOCUS` arms from the application; verify the main-window "Move between monitors" scenario manually and that focus returns to the previously focused control after Alt+Tab away and back (DPI handling verified by the lwfm-win test `dpi_change_swaps_the_font_and_rescales_columns`, because a DPI change cannot be simulated from another process; a real move between monitors was not exercised. Focus restore verified by activating another window and coming back.)
- [ ] 4.3 Handle system setting changes: recreate the font only when the metrics differ, release the old font, raise `SettingsChanged`, and have the application recompute its layout; verify the main-window "System font changed while running" scenario manually **Open:** implemented; the lwfm-win test shows that the font is swapped when the metrics differ and kept on an unrelated settings change, and a settings-change message leaves listing, selection and focus untouched. Changing the real system font (accessibility text size) while lwfm runs is still to be checked by hand.
- [x] 4.4 Add `UiWaker` and replace the application's `Waker` handle-as-integer with it; verify enumeration, config reload and shell exit-code messages still arrive (navigate, save config, run `shell "exit 3"`)
- [x] 4.5 Add `KeyEvent`, `KeyCode`, `NamedKey`, `VirtualKey` and `ModState::current` to the crate and feed `Handler::on_key` from the message loop; map `NamedKey` to `Named` in `keys/normalize.rs`; verify the `normalize` unit tests pass after the switch and the key-input scenarios for AltGr characters and dead keys manually on the QWERTZ layout (AltGr+Q, AltGr+8 and the dead key ^ + e verified with injected input on the German layout)
- [x] 4.6 Delete `src/ui/`; verify `App` implements `Handler` and `grep -rn "unsafe" src` returns nothing; commit

## 5. Close the boundary

- [x] 5.1 Remove the `windows` dependency from the binary and add `#![forbid(unsafe_code)]` to `src/main.rs`; verify `cargo build` succeeds and `grep -rn "windows::" src` returns only `std::os::windows` paths
- [x] 5.2 Review the public API of `lwfm-win` for leaked `windows` types and for escape hatches; verify with `cargo doc -p lwfm-win` that no public signature mentions a `windows::` path; commit

## 6. Application structure

- [x] 6.1 Split `src/app.rs` into `src/app/{mod,list,keys,nav,search,config,commands,status}.rs` without changing logic; verify `cargo test` passes and no file under `src/app/` exceeds 400 lines
- [x] 6.2 Group the fields into `ConfigState`, `KeySeq`, `Listing`, `SearchState`, `RebuildTimer` and `TraceState` (design D11) and document the borrow rule on `Listing` and `SearchState`; add the `debug_assert!` next to the `try_borrow` fallbacks; verify a debug build runs through navigation, sort, toggle-hidden, search and config reload without an assertion
- [x] 6.3 Replace `load`, `loading`, `loaded` and `committed` with `LoadState`; verify the navigation and directory-listing scenarios manually: slow path keeps the old listing until the first batch, failed start argument falls back to home with a message, refresh keeps selection and search
- [x] 6.4 Pass `Startup` by value and remove the `config::defaults()` placeholder from `App::new`; verify with `LWFM_TRACE=1` that the config is loaded once per start; commit

## 7. Errors and idioms

- [x] 7.1 Implement `std::error::Error` for `ParseError`, `CmdError` and `ChordError`; introduce `KeymapError` and `ConfigError` with `Display` texts identical to today's; verify the existing config, schema and keymap tests pass unchanged (the `Display` texts are unchanged; three test helpers convert the typed error with `to_string()`, and `shell::expand` got an `ExpandError` too)
- [x] 7.2 Replace lossy `as` casts with `From`/`TryFrom` or the documented helpers, wildcard `windows` imports with explicit lists, and the `COLUMNS` tuple with a struct; implement `FromStr` for `SortKey` and `Scope`; verify `cargo clippy --all-targets -- -W clippy::pedantic` reports no cast, pointer or wildcard-import warnings
- [x] 7.3 Settle the pedantic allow list in `[workspace.lints]` and turn pedantic into a denied group; verify `cargo clippy --all-targets -- -D warnings` passes and every `unsafe` block in `lwfm-win` has a `// SAFETY:` comment (enforced by the lint); commit (the only allowed pedantic lint is `module_name_repetitions`; three item-level `#[allow]`s carry their reason)

## 8. Verification and project rules

- [ ] 8.1 Measure the performance budgets with a release build and `LWFM_TRACE=1` exactly as in 1.4 and add the results next to the baseline in `design.md`; verify first painted listing is under 100 ms for 1,000 entries, 10,000 entries are painted under 50 ms after enumeration completes, search update is under 50 ms, config load is under 10 ms, and none is worse than the baseline by more than 10 %
- [ ] 8.2 Walk through every scenario of the main-window, directory-listing, navigation, key-input, command-prompt, list-search, lua-config, lua-scripting and external-commands specs in a release build; verify each passes and note any deviation in this file before fixing it
- [ ] 8.3 Check for leaks with the window open for a session of navigation, search and five config reloads; verify in Task Manager or Process Explorer that the handle, GDI object and thread counts return to their values after the first listing
- [ ] 8.4 Update the architecture rules in `openspec/config.yaml`: `windows` and `unsafe` only in `lwfm-win`, no `windows` types in its public API, controls are used through use-case traits, programmatic changes are not echoed; verify `openspec validate win32-crate-restructure --strict` passes; commit
