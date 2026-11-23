# Design

## Context

Greenfield: the repository holds a Cargo package `lwfm` (edition 2024, no dependencies) with a hello-world `main.rs`. See proposal.md for motivation and scope and the specs under `specs/` for the required behavior.

Constraints that shape the approach:
- Windows-only, raw Win32. No UI framework, so the startup path stays short and fully under our control.
- The UI thread must never touch the disk (main-window: Responsive UI).
- Later changes build on this one: `config-and-scripting` replaces the hardcoded keymap with a Lua-defined one and exposes the command registry to Lua, `file-operations` adds commands, and `dual-pane-layout` introduces a second list. The registry and chord types must therefore be stable and independent of the window code.

## Goals / Non-Goals

**Goals:**
- One choke point for keyboard input (the message loop) and one for actions (the command registry).
- A data model that keeps re-sorting and hidden-toggling cheap (no re-enumeration).
- Pure, unit-testable cores: chord normalization/parsing, command parsing, sorting, history.
- Measurable startup and listing timings.

**Non-Goals:**
- Live file-system watching. Listings refresh on navigation or `refresh` only; `ReadDirectoryChangesW` comes in a later change.
- Key sequences, counts, hint bar, configurable keymaps (all in `config-and-scripting`).
- A drive list / "This PC" view. `up` stops at roots; other drives are reached via the path bar.
- Icons and shell integration beyond `ShellExecuteExW`.

## Decisions

### D1. `windows` crate, not `windows-sys` or a wrapper library
Use Microsoft's `windows` crate with only the needed feature flags.
- Later changes need COM (`IContextMenu`, `IDropTarget`, clipboard `CF_HDROP`), which `windows` handles well. `windows-sys` would need hand-written COM vtables.
- `native-windows-gui` is mostly unmaintained. `winsafe` is fine, but it adds an abstraction we would fight for custom drawing and message-loop interception.
- Keep feature flags minimal to limit compile time.

### D2. Module layout
```
src/
  main.rs          entry: args, DPI/COM init, create window, run loop
  app.rs           App state (current dir, model, history, keymap, registry), dispatch
  ui/
    window.rs      top-level window proc, layout, WM_DPICHANGED
    list.rs        virtual ListView: columns, LVN_GETDISPINFO, custom draw, header clicks
    pathbar.rs     Edit control, enter/escape handling hooks
    statusbar.rs   status text
  fs/
    entry.rs       Entry struct, attributes, display formatting (size, time)
    enumerate.rs   worker: open dir, bulk read, batching, cancellation
    model.rs       entries + visible index + sort
    path.rs        normalization, ~ expansion, \\?\ prefixing, parent
  nav.rs           history (back/forward stacks with remembered selection)
  cmd/
    parse.rs       command string tokenizer/parser
    registry.rs    Command table: name, signature, description, handler
  keys/
    chord.rs       Chord type, canonical format/parse
    normalize.rs   (vk, scancode, modifier state, char) -> Chord
    keymap.rs      scopes, lookup, built-in default keymap
```
`cmd/`, `keys/chord.rs`, `fs/model.rs`, `fs/path.rs` and `nav.rs` contain no Win32 UI calls, so they can be unit tested.

### D3. Enumeration API: `GetFileInformationByHandleEx(FileFullDirectoryInfo)`
Open the directory with `CreateFileW(FILE_LIST_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS)` and read 64 KB buffers with `GetFileInformationByHandleEx(FileFullDirectoryRestartInfo / FileFullDirectoryInfo)`. Each call returns many records with name, attributes, size and timestamps.
- Alternatives: `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)` is simpler and nearly as fast, but it copies one `WIN32_FIND_DATAW` (with a fixed 260-char buffer) per entry. `NtQueryDirectoryFile` is undocumented-ish and adds no benefit here.
- Fallback: if the bulk call fails with "not supported" (some network redirectors or FAT drivers), use `FindFirstFileExW` with the same batching.
- Directory enumeration does not hydrate cloud placeholder files (OneDrive), so no special handling is needed.

### D4. One thread per enumeration, a generation counter, and channel + wake message
```
 UI thread                                    worker thread (per request)
 ---------                                    ---------------------------
 navigate(path):
   gen += 1  (AtomicU64 shared)
   spawn worker(path, gen) ------------------>  open dir (may block for seconds)
                                                loop: read batch
                                                  if shared_gen != gen: exit
                                                  send Batch{gen, entries} on channel
   <---------- PostMessageW(WM_APP_FS) --------   (only when channel goes non-empty)
 on WM_APP_FS:
   drain channel; drop msgs with gen != current
   first batch of new gen: commit navigation (dir, title, history)
   merge batch, re-sort, refresh list
                                                send Done{gen} or Error{gen, err}
```
- **Thread per request, not a single worker:** opening an unreachable share can block in `CreateFileW` for 20+ s. With one worker, every later navigation would queue behind it. Abandoned threads exit by themselves when their blocking call returns and they see a stale generation.
- **Navigation commits on the first batch or on Done:** this gives the spec rule "only take effect once read successfully". On error, the previous listing stays.
- **Batches** are sent every 1,024 entries or 30 ms, whichever comes first. Small directories therefore arrive in a single batch, giving a single sort and paint.
- **Data goes over a channel, and `lParam` carries no pointers.** The message only wakes the UI thread. Nothing leaks if the window is gone.
- Quit: the main thread exits the process after the loop ends. Detached workers do not keep it alive (`std::process::exit` after cleanup).

### D5. Data model: entries plus a visible index
- `Entry { name: Box<[u16]>, attrs: u32, size: u64, mtime: i64 }`. Names stay UTF-16, because they come from Win32 and go back to Win32 for display and sorting. That means no conversion per paint.
- `Model { entries: Vec<Entry>, visible: Vec<u32> }`. `visible` holds indices filtered by hidden-visibility and sorted. Toggling hidden or re-sorting rebuilds only `visible`. The ListView item count is `visible.len()`.
- Selection identity: before re-sort or refresh, save the selected and focused names, then restore them by name afterwards (sorting and history specs).
- **Name comparison:** `CompareStringEx(LOCALE_NAME_USER_DEFAULT, NORM_IGNORECASE | SORT_DIGITSASNUMBERS)`. This gives natural, case-insensitive, locale-aware order like Explorer, from kernel32. Alternative `StrCmpLogicalW` needs shlwapi loaded at startup. A hand-written natural sort would get locale rules wrong.
- Size and date strings are formatted on demand in `LVN_GETDISPINFO` into a reusable buffer. They are never stored per entry.

### D6. Virtual ListView (`LVS_REPORT | LVS_OWNERDATA`)
- Extended styles `LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER` (no flicker on batch updates).
- `NM_CUSTOMDRAW` dims hidden items and colors directories. Directory marking is a trailing `\` in the name column plus a color; icons are a non-goal.
- Header clicks (`LVN_COLUMNCLICK`) run the `sort` command, so mouse actions also go through the registry.
- `NM_DBLCLK` runs `open`.
- Native type-to-find (`LVN_ODFINDITEM`) is implemented over `visible`, so unbound character keys keep working Explorer-style (key-input: Unbound keys pass through).

### D7. Key interception in the message loop
```
 GetMessageW(msg)
   WM_KEYDOWN / WM_SYSKEYDOWN:
     mods = GetKeyState(ctrl, alt, shift, win)
     altgr = RAlt down && Ctrl down            (layouts with AltGr inject LCtrl)
     if named key (F1..F24, arrows, enter, ...) or ((ctrl||alt) && !altgr):
        chord = normalize(vk, mods)           (letters via VK, punctuation via MapVirtualKeyExW)
        if keymap.lookup(scope(focus), chord) -> run command; continue (skip Translate)
     TranslateMessage(msg)                    -> may queue WM_CHAR
   WM_CHAR (from previous Translate):
     if char is printable: chord = Char(c)    (shift already folded, AltGr already applied)
        if keymap.lookup(...) -> run command; continue
   DispatchMessageW(msg)                      -> native control gets unbound input
```
- Character chords use `WM_CHAR` instead of `ToUnicodeEx`. `ToUnicodeEx` corrupts dead-key state, so `^` + `e` would stop producing `ê`.
- Enter and Escape in the path bar arrive as named keys and are bound in the `input` scope, so the Edit control needs no subclassing.
- `WM_SYSKEYDOWN` for unbound Alt chords is still dispatched, so Alt+F4 and Alt+Space (system menu) keep working.
- Scope is derived from `GetFocus()`: path bar is `input`, list is `list`, anything else is `global`.

### D8. Command registry: static table, parsed once
- `Command { name, args: &[ArgSpec], desc, run: fn(&mut App, &Args) -> Result<(), CmdError> }` in a static slice. Listing commands for the spec is iteration over that slice.
- The keymap stores parsed `Invocation { cmd: &'static Command, args }`, not raw strings. The built-in keymap is parsed at startup. A unit test asserts it parses, so a typo is a test failure, not a runtime surprise. This is the same path `config-and-scripting` will use for Lua-provided strings.
- The tokenizer handles quotes, `\"` escapes, `--flag` and `--flag=value`. Unknown flags are errors.
- `CmdError` renders into one status-bar line.

### D9. Opening files off the UI thread
`ShellExecuteExW` can block (network files, slow handlers, first COM activation). Run it on a short-lived thread with `CoInitializeEx(COINIT_APARTMENTTHREADED)` and `SEE_MASK_NOASYNC | SEE_MASK_FLAG_DDEWAIT`. Errors are posted back to the status bar. Windows' own "open with" UI handles files without an association.

### D10. Paths
- Internally, absolute normalized paths as UTF-16 (`GetFullPathNameW` for relative input, and `~` expanded from `%USERPROFILE%` to avoid loading shell32 at startup).
- When calling file APIs with paths of 260 characters or more, prefix `\\?\` (or `\\?\UNC\` for shares). The prefix is never shown to the user. This works without the system-wide `LongPathsEnabled` setting.
- Parent of `C:\` or `\\server\share\` is none, so `up` is a no-op.

### D11. Manifest, DPI and build profile
- Embed an application manifest at build time with the `embed-manifest` build dependency (no `rc.exe`): Common Controls v6, PerMonitorV2 DPI awareness, `longPathAware`.
- Handle `WM_DPICHANGED`: apply the suggested rect, recreate the font from `SystemParametersForDpi` (message font), and rescale column widths.
- Release profile: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`. With `panic = "abort"`, an unwind can never cross an `extern "system"` window procedure.
- `#![windows_subsystem = "windows"]` in release, so no console window appears.

### D12. Tracing for performance budgets
When `LWFM_TRACE=1` is set, record `QueryPerformanceCounter` timestamps (process start via `GetProcessTimes`, enumeration done, first paint after commit) and write them with `OutputDebugStringW` and to stderr. This is the measurement method for both performance requirements. Tracing must cost nothing when disabled (one check of a static bool).

## Risks / Trade-offs

- [AltGr detection heuristic: RAlt plus Ctrl is treated as AltGr. On layouts without AltGr, a deliberate Ctrl+RightAlt+key chord becomes a character chord] -> Left Ctrl + Left Alt combinations still work. Revisit with `VkKeyScanEx`/layout inspection if users report it.
- [Ctrl+H is bound globally, so in the path bar it no longer acts as Backspace (the classic Edit control behaviour)] -> Accepted. Backspace itself still passes through.
- [Abandoned enumeration threads accumulate if the user repeatedly opens unreachable shares] -> Each thread is small and exits when its blocking call returns. Log the thread count under tracing, and add a cap only if it becomes a real problem.
- [Loading shell32 at startup for `ShellExecuteExW` imports could cost several ms of the 100 ms budget] -> Measure with D12. If significant, resolve `ShellExecuteExW` lazily via `LoadLibraryW` and `GetProcAddress` on first `open`.
- [Re-sorting on every batch is O(n log n) per batch for huge directories (200k entries -> about 200 batches)] -> Sort each batch on its own and merge it into the sorted `visible` list, or throttle repaints to every 100 ms while loading. Pick whichever the tracing data favours.
- [The `FileFullDirectoryInfo` path is unsupported on some file systems or redirectors] -> `FindFirstFileExW` fallback (D3).
- [No file-system watching, so listings go stale] -> `refresh` (F5) for now; watching is planned.

## Migration Plan

Not applicable (first release, nothing to migrate).

## Open Questions

- Exact batch size and timing (1,024 entries / 30 ms) will be tuned against tracing data. Specs and tasks do not depend on the values.
- Whether `alt+d` and `f4` as extra `focus-path` bindings (Explorer habits) survive into the presets of `config-and-scripting`.
