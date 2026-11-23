# Tasks

## 1. Project setup

- [x] 1.1 Add `windows` (with only the needed Win32 features) as a dependency and `embed-manifest` as a build dependency; verify `cargo build` succeeds
- [x] 1.2 Add `build.rs` embedding the manifest (Common Controls v6, PerMonitorV2 DPI, longPathAware); verify with `mt.exe -inputresource:target\debug\lwfm.exe -out:con` or by inspecting the resource
- [x] 1.3 Configure the release profile (opt-level 3, thin LTO, codegen-units 1, panic abort, strip) and `windows_subsystem = "windows"` for release; verify `cargo build --release` produces a binary that starts without a console window
- [x] 1.4 Create the module skeleton from design D2 (`app`, `ui/*`, `fs/*`, `nav`, `cmd/*`, `keys/*`); verify `cargo build` and `cargo test` pass with empty modules

## 2. Pure cores (no UI)

- [x] 2.1 Implement `keys::chord`: Chord type, canonical formatting and case-insensitive parsing (modifier order ctrl+alt+shift+win, named keys incl. `plus`); verify unit tests for round trip `Shift+Ctrl+F5` -> `ctrl+shift+f5` and error on `ctrl+foo`
- [x] 2.2 Implement `cmd::parse` tokenizer (whitespace split, double quotes, `\"` escapes, `--flag`, `--flag=value`); verify unit tests for quoted path, unterminated quote error and flags
- [x] 2.3 Implement `fs::path`: normalize, `~` expansion from `%USERPROFILE%`, parent (none at drive/share root), `\\?\` / `\\?\UNC\` prefixing for long paths; verify unit tests for each case incl. `C:\` and `\\server\share\` roots
- [x] 2.4 Implement `fs::entry` display formatting (binary-unit sizes, empty for dirs, local `YYYY-MM-DD HH:MM`); verify unit tests for `12288` -> `12 KB` and a fixed timestamp
- [x] 2.5 Implement `fs::model`: entries + visible index, hidden/system filtering, directories first, sort by name (`CompareStringEx` natural, case-insensitive) / size / mtime asc/desc, select-by-name restore helpers; verify unit tests for natural order `file1, File2, file10`, dirs-first in all sorts, and hidden toggle without re-enumeration
- [x] 2.6 Implement `nav` history (back/forward stacks with remembered selection, forward cleared on new navigation); verify unit tests for back/forward, forward clearing and selection restore

## 3. Command registry

- [x] 3.1 Implement `cmd::registry` static command table (name, arg specs, description, handler) and argument validation with `CmdError`; verify unit tests for unknown command `cpoy`, `sort color` invalid argument, extra/missing args
- [x] 3.2 Register built-ins `cd`, `up`, `back`, `forward`, `open`, `refresh`, `toggle-hidden`, `sort`, `focus-path`, `focus-list`, `quit` with handlers stubbed against `App`; verify a unit test that lists all commands with name, args and description

## 4. Enumeration worker

- [x] 4.1 Implement `fs::enumerate` with `CreateFileW` + `GetFileInformationByHandleEx(FileFullDirectoryInfo)` in 64 KB buffers, skipping `.`/`..`; verify an integration test enumerating a temp dir with known files, dirs and a hidden file
- [x] 4.2 Add `FindFirstFileExW(FindExInfoBasic, LARGE_FETCH)` fallback when the bulk call is unsupported; verify by a test forcing the fallback path on the same temp dir
- [x] 4.3 Add batching (1,024 entries or 30 ms), generation check between batches, channel send and `PostMessageW(WM_APP_FS)` wake-up only when the channel becomes non-empty; verify a test that a stale generation stops sending and that a 5,000-entry temp dir arrives in multiple batches
- [x] 4.4 Map OS errors (access denied, not found, not ready, network) to readable messages with the path; verify unit test on the mapping and a manual check opening a denied directory (e.g. `C:\System Volume Information`)

## 5. Window and controls

- [x] 5.1 Create the main window with path bar (Edit), virtual ListView (`LVS_REPORT | LVS_OWNERDATA`, full-row select, double buffer) and status bar; lay them out on `WM_SIZE`; verify manually that resizing keeps bars full-width and the list fills the rest
- [x] 5.2 Implement `LVN_GETDISPINFO` for Name/Size/Modified from `model.visible`, directory trailing `\`, and `NM_CUSTOMDRAW` for directory color and dimmed hidden items; verify manually on a dir with hidden items and `toggle-hidden`
- [x] 5.3 Implement `LVN_ODFINDITEM` type-to-find over the visible list; verify manually that typing a name prefix in the list jumps to it
- [x] 5.4 Wire header clicks to `sort` (second click reverses) and double-click to `open`; verify manually by clicking Size twice and double-clicking a folder
- [x] 5.5 Status bar shows item count, selected count, loading progress and the latest message/error; verify manually by selecting items and triggering an error
- [ ] 5.6 Handle `WM_DPICHANGED` (suggested rect, recreate message font for the DPI, rescale columns); verify manually by moving the window between monitors with different scale factors, or by changing the scale factor
- [x] 5.7 Update the window title to `<current dir> - lwfm` on each committed navigation; verify manually

## 6. Navigation and app wiring

- [x] 6.1 Implement `App::navigate`: bump generation, spawn worker, commit on first batch or Done (dir, title, path bar, history), keep previous listing and report on error; verify manually that `cd C:\nope` leaves the listing unchanged with an error
- [x] 6.2 Handle `WM_APP_FS`: drain channel, drop stale generations, merge batches into the model, keep selection by name, update the list item count; verify manually that quickly navigating A -> B never shows A's entries in B (use a large dir for A)
- [x] 6.3 Implement `up` (select the directory just left), `back`/`forward` with selection restore, and `refresh` keeping selection; verify the navigation scenarios from `specs/navigation` manually
- [x] 6.4 Implement `open`: directories navigate; files launch via `ShellExecuteExW` on a short-lived STA thread with errors posted back; verify manually that opening a `.txt` works and the window stays responsive
- [x] 6.5 Implement the start directory from argv[1] with home fallback and status-bar error for an invalid argument; verify manually with `lwfm C:\Windows`, `lwfm`, and `lwfm C:\does-not-exist`
- [x] 6.6 Implement path bar behaviour: `focus-path` selects all text, Enter runs `cd` and returns focus to the list on success (keeps text on failure), Escape restores text and focuses the list; verify the path bar scenarios manually

## 7. Key input

- [x] 7.1 Implement `keys::normalize` as a pure function of (vk, modifier state, altgr flag, optional char), covering named keys, ctrl/alt via VK, punctuation via `MapVirtualKeyExW` (wrapped behind a trait for tests); verify unit tests for `ctrl+shift+n`, `J` and the AltGr case producing a character chord
- [x] 7.2 Implement `keys::keymap` scopes (`input`, `list`, `global`), lookup from the focused scope to global, and the default keymap table from `specs/key-input`, parsed at startup into invocations; verify a unit test that the default keymap parses and resolves `backspace` in `list` to `up` and `ctrl+q` in `input` to `quit`
- [x] 7.3 Hook the message loop per design D7 (keydown for named/ctrl/alt chords, `WM_CHAR` for character chords, unbound keys dispatched unchanged); verify manually: Backspace edits in the path bar but goes up in the list, arrows move in the list, AltGr+Q types `@` in the path bar on a German layout, dead keys (`^` + `e`) still produce `ê`, Alt+F4 still closes

## 8. Performance and quit

- [x] 8.1 Implement `LWFM_TRACE=1` tracing (process start, enumeration done, first paint after commit) via `OutputDebugStringW` and stderr; verify the timings are printed when set and nothing is printed when unset
- [x] 8.2 Measure cold start on a 1,000-entry local dir and listing time on a 10,000-entry local dir with a release build; verify under 100 ms and 50 ms respectively, and if shell32 loading dominates startup, switch `ShellExecuteExW` to lazy `LoadLibraryW` (design risk)
- [x] 8.3 Ensure quit (`ctrl+q`, window close) exits the process even with an enumeration blocked on an unreachable share; verify manually with a non-existent UNC path like `\\10.255.255.1\x`
- [x] 8.4 Run `cargo clippy -- -D warnings` and `cargo test`; verify both pass
