# Spec Delta

## Purpose

Defines how lwfm turns keyboard input into commands: normalization into layout-aware chords with a canonical text form, AltGr-safe handling, scope-based keymap lookup, and pass-through of unbound keys to native controls.

## ADDED Requirements

### Requirement: Chords
Every key press SHALL be normalized into a chord made of a set of modifiers (ctrl, alt, shift, win) and either a named key or a character. Keys that produce a character without ctrl or alt SHALL become a character chord with shift folded into the character (e.g. Shift+J is `J`, Shift+7 on a German layout is `/`). Named keys (function keys, arrows, Enter, Escape, Tab, Backspace, Delete, Insert, Home, End, PageUp, PageDown, Space) and keys pressed with ctrl or alt SHALL become chords with explicit modifiers. Chords SHALL follow the active keyboard layout (logical keys), not physical key positions.

#### Scenario: Shifted character
- **WHEN** the user presses Shift+J
- **THEN** the chord is `J`

#### Scenario: Ctrl combination
- **WHEN** the user presses Ctrl+Shift+N
- **THEN** the chord is `ctrl+shift+n`

#### Scenario: Logical layout
- **WHEN** the user presses the key labelled Z on a German QWERTZ keyboard
- **THEN** the chord is `z`

### Requirement: AltGr characters
Characters produced with AltGr SHALL be treated as character chords without ctrl or alt modifiers, and SHALL NOT be interpreted as ctrl+alt combinations.

#### Scenario: AltGr in path bar
- **WHEN** the path bar is focused on a German layout and the user presses AltGr+Q
- **THEN** `@` is typed into the path bar

#### Scenario: AltGr character chord
- **WHEN** the list is focused on a German layout and the user presses AltGr+8
- **THEN** the chord is `[`, not `ctrl+alt+8`

### Requirement: Canonical chord text
Chords SHALL have a canonical text form: modifiers in the order `ctrl+alt+shift+win`, lowercase named keys (`f5`, `enter`, `esc`, `left`, `pageup`, `space`, `plus` for `+`), joined with `+`. Parsing a chord text and formatting the result SHALL round-trip, and parsing SHALL accept modifiers and key names in any letter case and modifier order.

#### Scenario: Round trip
- **WHEN** the text `Shift+Ctrl+F5` is parsed and formatted
- **THEN** the result is `ctrl+shift+f5`

#### Scenario: Invalid chord text
- **WHEN** the text `ctrl+foo` is parsed
- **THEN** parsing fails naming the unknown key `foo`

### Requirement: Scoped keymap lookup
Each chord SHALL be resolved against keymap scopes, from the scope of the focused element (`input` for the path bar, `list` for the file list) up to `global`. The first scope that binds the chord SHALL determine the command to run. A bound chord SHALL be consumed and not reach the native control.

#### Scenario: List binding
- **WHEN** the list is focused and the user presses Backspace
- **THEN** the `up` command runs

#### Scenario: Global fallback
- **WHEN** the path bar is focused and the user presses Ctrl+Q
- **THEN** the `quit` command runs

### Requirement: Unbound keys pass through
A chord bound in no active scope SHALL be delivered to the focused native control unchanged, so text editing in the path bar and native list navigation (arrows, Home, End, PageUp, PageDown, type-to-find) keep working.

#### Scenario: Editing in path bar
- **WHEN** the path bar is focused and the user presses Backspace
- **THEN** a character is deleted in the path bar and `up` does not run

#### Scenario: Arrow keys in list
- **WHEN** the list is focused and the user presses Down
- **THEN** the focus moves to the next entry

### Requirement: Default keymap
The application SHALL ship this default keymap:

| scope | chord | command |
|---|---|---|
| global | `ctrl+q` | `quit` |
| global | `ctrl+l`, `alt+d`, `f4` | `focus-path` |
| global | `alt+left` | `back` |
| global | `alt+right` | `forward` |
| global | `alt+up` | `up` |
| global | `f5`, `ctrl+r` | `refresh` |
| global | `ctrl+h` | `toggle-hidden` |
| list | `enter` | `open` |
| list | `backspace` | `up` |
| input | `enter` | run `cd` with the path bar text |
| input | `esc` | cancel path editing |

#### Scenario: Default binding works
- **WHEN** the list is focused and the user presses Alt+Left after navigating from A to B
- **THEN** the listing shows A
