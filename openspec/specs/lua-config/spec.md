# lua-config Specification

## Purpose

Defines how lwfm is configured through a single Lua file, `config.lua`: where it is found, how it is evaluated and validated, which settings it controls, and how errors and edits are handled without ever blocking startup.

## Requirements

### Requirement: Config discovery
The application SHALL use the first existing file of: the path given with `--config <path>`, the path in the environment variable `LWFM_CONFIG`, `%USERPROFILE%\.config\lwfm\config.lua`, `%APPDATA%\lwfm\config.lua`. When `--config` or `LWFM_CONFIG` names a file that does not exist, the application SHALL report that and fall back to the built-in defaults. When no file exists, the built-in defaults (the explorer preset) SHALL apply without any message.

#### Scenario: Explicit path wins
- **WHEN** both `%APPDATA%\lwfm\config.lua` and `D:\my.lua` exist and lwfm is started with `--config D:\my.lua`
- **THEN** `D:\my.lua` is used

#### Scenario: Dotfile location before AppData
- **WHEN** both `%USERPROFILE%\.config\lwfm\config.lua` and `%APPDATA%\lwfm\config.lua` exist and neither `--config` nor `LWFM_CONFIG` is set
- **THEN** `%USERPROFILE%\.config\lwfm\config.lua` is used

#### Scenario: No config file
- **WHEN** no config file exists
- **THEN** lwfm starts with the explorer keymap and default settings, and the status bar shows no config message

#### Scenario: Missing explicit file
- **WHEN** lwfm is started with `--config D:\missing.lua` and that file does not exist
- **THEN** lwfm starts with built-in defaults and the status bar reports that `D:\missing.lua` was not found

### Requirement: Config script
`config.lua` SHALL be a Lua 5.4 script that returns a table. `require("lwfm")` SHALL provide the lwfm module, and `lwfm.preset("explorer")` SHALL return a fresh, modifiable table holding the explorer keymap and default settings. A script that returns something other than a table SHALL be an error.

#### Scenario: Minimal config
- **WHEN** `config.lua` contains `local cfg = require("lwfm").preset("explorer"); cfg.show_hidden = true; return cfg`
- **THEN** lwfm starts showing hidden items with the explorer keymap

#### Scenario: Wrong return value
- **WHEN** `config.lua` ends with `return 42`
- **THEN** the config is rejected with an error stating that the script must return a table

#### Scenario: Unknown preset
- **WHEN** `config.lua` calls `lwfm.preset("vim")`
- **THEN** the config is rejected with an error naming the unknown preset `vim` and listing `explorer`

### Requirement: Settings
The returned table SHALL support these settings, each optional:

| key | type | default | effect |
|---|---|---|---|
| `show_hidden` | boolean | `false` | initial hidden-item visibility |
| `sort` | `{ key = "name"\|"size"\|"mtime", reverse = boolean }` | `{ key = "name", reverse = false }` | initial sort order |
| `colors.dir` | `"#rrggbb"` | `"#1a5fb4"` | directory text color |
| `colors.hidden` | `"#rrggbb"` | `"#808080"` | hidden-item text color |
| `editor` | command string | `'shell "notepad $f"'` | command used by `edit-config`, with `$f` set to the config file |
| `keys` | table of scopes | explorer preset | keymap (see key-input) |
| `openers` | table of pattern -> action | empty | file openers (see external-commands) |

Settings SHALL be applied at startup and again on every successful reload; values changed at runtime (e.g. with `toggle-hidden`) SHALL be replaced by the configured values on reload.

#### Scenario: Sort setting
- **WHEN** the config sets `sort = { key = "mtime", reverse = true }`
- **THEN** listings are sorted newest first, directories first

#### Scenario: Color setting
- **WHEN** the config sets `colors = { dir = "#ff0000" }`
- **THEN** directories are drawn in red and hidden items keep the default gray

### Requirement: Config validation
Settings with a wrong type or value SHALL reject the config with an error naming the setting path and the expected value (e.g. `sort.key: expected one of name, size, mtime`). Unknown top-level or nested setting keys SHALL NOT reject the config but SHALL produce a warning naming each unknown key.

#### Scenario: Invalid value
- **WHEN** the config sets `colors = { dir = "blue" }`
- **THEN** the config is rejected with an error naming `colors.dir` and the `#rrggbb` format

#### Scenario: Typo warning
- **WHEN** the config sets `show_hiden = true`
- **THEN** the config is applied and the status bar warns about the unknown key `show_hiden`

### Requirement: Config errors never block startup
When the config cannot be loaded (Lua syntax or runtime error, validation error), the application SHALL start with the built-in defaults and show the error in the status bar, including the file name and line number when Lua provides them.

#### Scenario: Syntax error at startup
- **WHEN** `config.lua` has a syntax error on line 7
- **THEN** lwfm starts with built-in defaults and the status bar shows an error containing `config.lua:7`

### Requirement: Hot reload
Saving the config file, or any file in the `lua` directory next to it, SHALL reload the configuration within one second without restarting lwfm. A reload SHALL evaluate the config in a fresh Lua state; when it succeeds, the new keymap, settings, openers and event handlers SHALL replace the old ones, and the status bar SHALL report `Config reloaded`. When it fails, the previous configuration SHALL stay active and the status bar SHALL show the error. Editors that save through a temporary file and rename SHALL be supported.

#### Scenario: Edit picked up
- **WHEN** lwfm is running and the user adds a binding to `config.lua` and saves
- **THEN** within one second the new binding works and the status bar shows `Config reloaded`

#### Scenario: Broken edit keeps old config
- **WHEN** the user saves `config.lua` with a syntax error
- **THEN** the previous bindings keep working and the status bar shows the error with its line number

### Requirement: Config commands
The `reload-config` command SHALL reload the configuration as on a file change. The `edit-config` command SHALL open the active config file with the `editor` command; when no config file exists, it SHALL first create a commented starter config at `%APPDATA%\lwfm\config.lua` (creating the directory), start watching it, and then open it. Neither command SHALL block the window.

#### Scenario: Edit config without a file
- **WHEN** no config file exists and the user runs `edit-config`
- **THEN** `%APPDATA%\lwfm\config.lua` is created with a commented starter config that returns the explorer preset, and it opens in Notepad

### Requirement: Config check from the command line
Running `lwfm --check-config` (optionally with `--config <path>`) SHALL load and validate the config without opening a window, print `config OK: <path>` (or `no config file, using defaults`) and any warnings to the console it was started from, and exit with code 0; on an error it SHALL print the error and exit with code 1.

#### Scenario: Valid config
- **WHEN** the user runs `lwfm --check-config` in a terminal with a valid config
- **THEN** the terminal shows `config OK:` followed by the config path and the exit code is 0

#### Scenario: Invalid config
- **WHEN** the user runs `lwfm --check-config` with a config that has a syntax error
- **THEN** the terminal shows the error with file and line and the exit code is 1

### Requirement: Config load performance
Evaluating the starter config SHALL take less than 10 ms, so that cold start with the starter config still paints the first listing of a 1,000-entry local directory in under 100 ms. The time spent loading the config SHALL be reported when tracing is enabled.

#### Scenario: Startup budget with config
- **WHEN** lwfm is started with tracing enabled, the starter config and a 1,000-entry local directory
- **THEN** the traced config load time is under 10 ms and the time to first painted listing is under 100 ms
