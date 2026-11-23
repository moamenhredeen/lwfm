# Spec Delta

## MODIFIED Requirements

### Requirement: Scoped keymap lookup
Each key sequence SHALL be resolved against keymap scopes, from the scope of the focused element up to `global`: for the path bar `input` then `global`; for the search input `search` then `global`; for the file list `list` then `global`. The first scope that binds the sequence SHALL determine the action to run. A bound chord, and every chord of a pending sequence, SHALL be consumed and not reach the native control. The bindings SHALL come from the active configuration.

#### Scenario: List binding
- **WHEN** the list is focused and the user presses Backspace
- **THEN** the `up` command runs

#### Scenario: Search scope first
- **WHEN** the search input is focused and the user presses Down
- **THEN** `search-next` runs and the list control does not receive the key

#### Scenario: Global fallback
- **WHEN** the path bar is focused and the user presses Ctrl+Q
- **THEN** the `quit` command runs

### Requirement: Unbound keys pass through
A chord bound in no active scope SHALL be delivered to the focused native control unchanged, so text editing in the path bar and list navigation (arrows, Home, End, PageUp, PageDown) keep working. Exception: in the list, an unbound printable character SHALL open the search input (see list-search) and SHALL NOT be delivered to the list control, so the list's native prefix type-to-find is not used.

#### Scenario: Editing in path bar
- **WHEN** the path bar is focused and the user presses Backspace
- **THEN** a character is deleted in the path bar and `up` does not run

#### Scenario: Arrow keys in list
- **WHEN** the list is focused and the user presses Down
- **THEN** the focus moves to the next entry

#### Scenario: Letters search in list
- **WHEN** the list is focused and the user types `d`
- **THEN** the search input opens with `d` instead of a native jump to the next entry starting with `d`

### Requirement: Default keymap
The explorer preset, used when no config file exists and returned by `lwfm.preset("explorer")`, SHALL contain this keymap:

| scope | chord | command |
|---|---|---|
| global | `ctrl+q` | `quit` |
| global | `ctrl+l`, `alt+d`, `f4` | `focus-path` |
| global | `alt+left` | `back` |
| global | `alt+right` | `forward` |
| global | `alt+up` | `up` |
| global | `f5`, `ctrl+r` | `refresh` |
| global | `ctrl+h` | `toggle-hidden` |
| global | `ctrl+,` | `edit-config` |
| list | `enter` | `open` |
| list | `backspace` | `up` |
| list | `:` | `prompt` |
| search | `esc` | `search-close` |
| search | `enter` | `search-close`, then `open` |
| search | `down` | `search-next` |
| search | `up` | `search-prev` |
| input | `enter` | `path-submit` (runs `cd` with the path bar text, or the command after a leading `:`) |
| input | `esc` | `path-cancel` |

#### Scenario: Default binding works
- **WHEN** the list is focused and the user presses Alt+Left after navigating from A to B
- **THEN** the listing shows A

#### Scenario: Edit config key
- **WHEN** the user presses Ctrl+, with no config file present
- **THEN** a starter config is created and opened in the editor

## ADDED Requirements

### Requirement: Configurable keymap
The config's `keys` table SHALL hold one table per scope (`global`, `list`, `search`, `input`) mapping a key text to an action. The key text SHALL be one chord or several chords separated by single spaces. The action SHALL be one of: a command string; an array of command strings run in order until one fails; a Lua function; or a table `{ run = <string, array or function>, desc = <string> }`. The command string `noop` SHALL remove a binding. User bindings SHALL replace the preset's binding for the same scope and key text. An invalid chord, an invalid command string, an unknown scope, or two key texts in the same scope that normalize to the same chords SHALL reject the config with an error naming the offending key text.

#### Scenario: Override a preset binding
- **WHEN** the config sets `cfg.keys.global["ctrl+h"] = "refresh"`
- **THEN** Ctrl+H refreshes and no longer toggles hidden items

#### Scenario: Unbind
- **WHEN** the config sets `cfg.keys.list["backspace"] = "noop"`
- **THEN** Backspace in the list does nothing

#### Scenario: Lua function binding
- **WHEN** the config sets `cfg.keys.list["ctrl+e"] = function(fm) fm:notify(fm.dir) end` and the user presses Ctrl+E
- **THEN** the status bar shows the current directory

#### Scenario: Duplicate after normalization
- **WHEN** the config binds both `"ctrl+x"` and `"Ctrl+X"` in the `list` scope
- **THEN** the config is rejected with an error naming the duplicate key text

### Requirement: Key sequences
A binding whose key text has several chords SHALL run when those chords are pressed in order. After a chord that is a prefix of one or more bindings, lwfm SHALL wait for the next chord without a timeout and show the possible next chords with their `desc` (or command text) in the status bar. Escape SHALL cancel a pending sequence; a chord that continues no binding SHALL cancel the sequence and be discarded. Within the user's `keys` table, a key text that is a prefix of another key text in the same scope SHALL reject the config. A user binding SHALL remove preset bindings in the same scope that it is a prefix of, or that are a prefix of it.

#### Scenario: Two-chord binding
- **WHEN** the config binds `cfg.keys.global["ctrl+g d"] = { run = "cd ~/Downloads", desc = "Downloads" }` and the user presses Ctrl+G then D
- **THEN** lwfm navigates to the Downloads folder

#### Scenario: Hint bar
- **WHEN** bindings `ctrl+g d` (Downloads) and `ctrl+g p` (Projects) exist and the user presses Ctrl+G
- **THEN** the status bar lists `d Downloads` and `p Projects` and nothing runs yet

#### Scenario: Cancel sequence
- **WHEN** the user presses Ctrl+G and then Escape
- **THEN** no command runs, the hint disappears, and Escape is not delivered to any control

#### Scenario: Prefix conflict in user config
- **WHEN** the config binds both `"ctrl+g"` and `"ctrl+g d"` in the `global` scope
- **THEN** the config is rejected with an error naming both key texts
