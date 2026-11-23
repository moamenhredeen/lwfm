# Spec Delta

## MODIFIED Requirements

### Requirement: Open entry
The `open` command SHALL act on the focused entry: for a directory it SHALL navigate into it; for a file it SHALL run the matching configured opener (see external-commands) or, when none matches, launch the file with its default application through the Windows shell. Double-clicking an entry SHALL run `open`. Launching SHALL NOT block the window.

#### Scenario: Open directory
- **WHEN** the focused entry is directory `docs` and the user runs `open`
- **THEN** the listing switches to `docs`

#### Scenario: Open file
- **WHEN** the focused entry is `notes.txt`, no opener matches it, and the user runs `open`
- **THEN** `notes.txt` opens in its associated application and the window stays responsive

#### Scenario: Open file with opener
- **WHEN** the config has an opener for `*.txt` and the focused entry is `notes.txt`
- **THEN** `open` runs that opener instead of the associated application

#### Scenario: File without association
- **WHEN** the user opens a file whose type has no associated application and no opener matches
- **THEN** Windows' "open with" choice is offered, or the status bar reports that the file could not be opened
