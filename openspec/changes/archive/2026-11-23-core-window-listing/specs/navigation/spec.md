# Spec Delta

## Purpose

Defines how the user moves between directories and opens files in lwfm: entering directories, going to the parent, back/forward history, typing a path, and launching files with their default application.

## ADDED Requirements

### Requirement: Open entry
The `open` command SHALL act on the focused entry: for a directory it SHALL navigate into it; for a file it SHALL launch the file with its default application through the Windows shell. Double-clicking an entry SHALL run `open`. Launching SHALL NOT block the window.

#### Scenario: Open directory
- **WHEN** the focused entry is directory `docs` and the user runs `open`
- **THEN** the listing switches to `docs`

#### Scenario: Open file
- **WHEN** the focused entry is `notes.txt` and the user runs `open`
- **THEN** `notes.txt` opens in its associated application and the window stays responsive

#### Scenario: File without association
- **WHEN** the user opens a file whose type has no associated application
- **THEN** Windows' "open with" choice is offered, or the status bar reports that the file could not be opened

### Requirement: Navigate to path
The `cd <path>` command SHALL navigate to the given directory. Relative paths SHALL resolve against the current directory, and a leading `~` SHALL expand to the user's home directory. Navigation SHALL only take effect once the target has been read successfully; on failure the current directory stays and the error is reported.

#### Scenario: Absolute path
- **WHEN** the user runs `cd C:\Windows`
- **THEN** the listing shows `C:\Windows`

#### Scenario: Home expansion
- **WHEN** the user runs `cd ~/Downloads`
- **THEN** the listing shows the Downloads folder in the user's home directory

#### Scenario: Missing directory
- **WHEN** the user runs `cd C:\nope`
- **THEN** the current directory and listing are unchanged and the status bar reports that `C:\nope` was not found

### Requirement: Parent directory
The `up` command SHALL navigate to the parent of the current directory and select the entry of the directory just left. At a drive root or share root, `up` SHALL do nothing.

#### Scenario: Up selects previous directory
- **WHEN** the current directory is `C:\Users\me\docs` and the user runs `up`
- **THEN** the listing shows `C:\Users\me` with `docs` selected

#### Scenario: Up at drive root
- **WHEN** the current directory is `C:\` and the user runs `up`
- **THEN** nothing changes

### Requirement: History
Each successful navigation SHALL be recorded in a history. `back` SHALL return to the previous directory and `forward` SHALL re-visit a directory left via `back`. A new navigation after `back` SHALL discard the forward entries. Returning to a directory through history SHALL restore the selection that was active when it was left, if that entry still exists.

#### Scenario: Back and forward
- **WHEN** the user visits A, then B, then runs `back`, then `forward`
- **THEN** the listing shows A after `back` and B after `forward`

#### Scenario: New navigation clears forward
- **WHEN** the user visits A, then B, runs `back`, then navigates to C
- **THEN** `forward` does nothing

#### Scenario: Selection restored
- **WHEN** the user selects `x.txt` in A, navigates to B, then runs `back`
- **THEN** A is listed with `x.txt` selected

### Requirement: Path bar
The path bar SHALL always show the current directory. The `focus-path` command SHALL focus the path bar and select its text. Pressing Enter in the path bar SHALL run `cd` with its text and return focus to the list on success; on failure the typed text SHALL remain for correction. Pressing Escape SHALL restore the current directory text and return focus to the list.

#### Scenario: Type a path
- **WHEN** the user runs `focus-path`, types `C:\Windows` and presses Enter
- **THEN** the listing shows `C:\Windows` and the list has focus

#### Scenario: Typo kept
- **WHEN** the user types `C:\Windos` in the path bar and presses Enter
- **THEN** the status bar reports the error and the path bar still contains `C:\Windos`

#### Scenario: Cancel
- **WHEN** the user types in the path bar and presses Escape
- **THEN** the path bar shows the current directory again and the list has focus
