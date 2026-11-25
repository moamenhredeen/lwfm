# Proposal

## Why

Win32 calls are spread over eleven source files, and `src/app.rs` (1,505 lines) mixes about 35 `unsafe` blocks, pointer casts from `LPARAM` and list-view draw protocol with application logic held in 30 `Cell`/`RefCell` fields. `cargo clippy -W clippy::pedantic` reports 73 `unsafe` blocks without a safety comment and roughly 100 pointer or integer cast warnings. A code review also found resource handling defects (see What Changes). The next roadmap items (file operations, dual pane, later dark mode) all add Win32 surface; doing them on the current structure would multiply the problem, so the restructure comes first.

## What Changes

- Turn the repository into a Cargo workspace with a new member crate `lwfm-win` that is the only place allowed to depend on the `windows` crate or contain `unsafe`. Its public API exposes no `windows` types.
- The `lwfm` binary crate drops its `windows` dependency and declares `#![forbid(unsafe_code)]`.
- Controls are wrapped by use case, not by message: the application implements a `ListSource` trait (cell text, row color, match range, activation, column click, selection change) and the crate owns the list-view notification and custom-draw protocol, including the match-highlight painting. Window-level facts (resize, DPI change, timer, wake, settings change) arrive as plain typed events through a `Handler` trait.
- Changes made through the crate's own methods are no longer echoed back as notifications; the `search_guard` flag and the per-item status refresh during programmatic selection go away.
- File enumeration, directory watching, process launching, shell open, natural name comparison, time conversion, console attach and trace helpers move behind plain functions in `lwfm-win`, with RAII ownership of every handle, font, brush and COM apartment.
- `App` is split into several files and its state grouped into a few cohesive structs; the `load`/`loading`/`loaded`/`committed` flags become one load-state enum.
- String errors (`Result<_, String>`, `Result<(), u32>`) become small typed errors implementing `std::error::Error`.
- Lints are enforced from `Cargo.toml` and the toolchain is pinned with `rust-toolchain.toml`.
- Defects fixed on the way:
  - `CoUninitialize` is called even when `CoInitializeEx` failed (`App::shell_open`).
  - The working directory, the watched config directory and `ComSpec` are converted lossily (`to_string_lossy`), which breaks names that are not valid Unicode.
  - A brush is created and destroyed for every painted match; the highlight colors are fixed and unreadable in high-contrast themes.
  - The UI font is never refreshed when the system font changes, and the last font is never released.
  - The config watcher cannot be stopped while it waits for a change (latent: thread and directory handle stay alive until the next file event).
  - A failed thread spawn for enumeration panics instead of reporting an error.
  - `App::new` builds a second, discarded default configuration (a full Lua state) on every start.

## Non-goals

- Replacing the `Target` trait with a command enum: later change `command-enum`.
- A third crate for the pure logic (commands, keys, navigation, model): later change `core-crate-split`.
- Dark mode for common controls: later change `dark-mode`. This change only makes the crate the single place where it will be implemented.
- A second list pane: `dual-pane-layout`. This change only makes routing by control id possible.
- New file operations: `file-operations`.
- An automated end-to-end UI test harness: later change `ui-test-harness`. This change adds only a few hidden-window tests for the crate.
- Adopting winsafe, iced or any other UI library.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `main-window`: adds a requirement that the window follows system font changes while running.
- `list-search`: the match highlighting requirement gains a readability rule for high-contrast themes.

All other capabilities keep their requirements; their implementation moves but their behavior must not change.

## Impact

- Code: every file under `src/` is touched. New `crates/lwfm-win/`. `src/ui/` is removed (its content moves into the crate). `src/app.rs` becomes `src/app/`.
- Build: root `Cargo.toml` becomes a workspace root that is also the `lwfm` package; `windows` moves to `crates/lwfm-win/Cargo.toml`. New files `rust-toolchain.toml` and a `[lints]` table. No new third-party dependency.
- Project rules: `openspec/config.yaml` architecture rules gain the crate boundary rule and lose the statement that app code calls `SendMessageW` directly.
- Tests: enumeration and long-path tests move with their code; new hidden-window tests in `lwfm-win`.
- Performance: startup, listing and search budgets must be re-measured because cell text and painting now cross a crate boundary and startup no longer builds a second Lua state.
- Users: no change in commands, keys or configuration.
