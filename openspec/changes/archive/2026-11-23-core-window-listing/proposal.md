# Proposal

## Why

Windows File Explorer is slow to start and to open folders. The cost comes from the layers above the file system (shell namespace, shell extensions, cloud providers, Home/Quick Access feeds), not from enumerating files. lwfm aims to be a fast, minimal, keyboard-friendly file manager built in Rust on raw Win32. This first change builds the foundation every later change depends on: a window that lists a directory quickly, lets the user navigate it, and routes keys through a command layer.

## What Changes

- New Win32 application window (Rust, `windows` crate, no UI framework) with a path bar, a file list and a status bar.
- File list implemented as a virtual ListView (`LVS_OWNERDATA`), so only visible rows are materialized and very large directories stay responsive.
- Directory enumeration on a worker thread using bulk file-system APIs. Results are posted to the UI thread, and enumeration is cancelled when the user navigates away. The UI thread never performs disk IO.
- Navigation: enter a directory, go to the parent, back/forward history, type a path into the path bar, and open files with their default application (`ShellExecuteExW`).
- Basic list behaviour: columns name/size/modified, sort by column, directories first, toggle hidden files.
- Command registry: named commands (e.g. `cd`, `up`, `back`, `open`, `toggle-hidden`, `quit`) with arguments, invoked through one dispatcher. Later the keymap, the `:` prompt, Lua and IPC all go through this registry.
- Key input layer: keys are intercepted in the message loop before native controls see them, and normalized into chords (modifiers plus a named key or character). It is AltGr-safe for non-US layouts such as German, and works on logical keys.
- A hardcoded default keymap in Rust, shaped like the future Lua keymap (scope -> chord -> command string), so change 2 only swaps its source.
- Initial performance targets: cold start to first painted listing under 100 ms, and a 10k-entry local directory displayed under 50 ms after enumeration completes.

### Non-goals (deferred to later changes)

- Lua configuration, scripting, presets, key sequences, hint bar, hot reload -> `config-and-scripting`.
- Copy/move/delete/rename/mkdir -> `file-operations`.
- Dual pane layout -> `dual-pane-layout`.
- Per-file icons, thumbnails, previews, shell context menu, drag and drop, dark mode, tabs.

## Capabilities

### New Capabilities
- `main-window`: application window, layout of path bar / file list / status bar, lifecycle and responsiveness guarantees (UI thread never blocks on IO).
- `directory-listing`: asynchronous enumeration, cancellation, virtual list display, columns, sorting, hidden-file visibility, error display for inaccessible directories.
- `navigation`: entering directories, parent, back/forward history, path bar input, opening files with the default application.
- `command-registry`: named commands with arguments, parsing of command strings, single dispatch path, error reporting for unknown commands.
- `key-input`: interception and normalization of keyboard input into chords, AltGr handling, resolving chords against the active scope's keymap, pass-through to text inputs for unbound keys.

### Modified Capabilities
<!-- none: no existing specs -->

## Impact

- New code under `src/` replacing the hello-world `main.rs`.
- New dependency: `windows` crate (Win32 UI, file system, shell `ShellExecuteExW`). Needs an application manifest for Common Controls v6.
- Windows-only binary.
- Establishes the command registry and key chord types that `config-and-scripting`, `file-operations` and `dual-pane-layout` build on.
