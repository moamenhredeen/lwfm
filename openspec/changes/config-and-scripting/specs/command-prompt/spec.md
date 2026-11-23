# Spec Delta

## Purpose

Defines the `:` command prompt, which lets the user run any registry command by typing its command string into the path bar, without needing a key binding.

## ADDED Requirements

### Requirement: Open the prompt
The `prompt` command SHALL focus the path bar with the text `:` and the caret after it. In the explorer preset, typing `:` in the list SHALL run `prompt`.

#### Scenario: Colon opens prompt
- **WHEN** the list has focus and the user types `:`
- **THEN** the path bar has focus and contains `:` with the caret at the end

### Requirement: Run a command from the path bar
When Enter is pressed in the path bar and its text starts with `:`, the rest of the text SHALL be run as a command string instead of being treated as a path. On success the path bar SHALL show the current directory again and the list SHALL get focus; on failure the error SHALL be shown in the status bar and the typed text SHALL stay for correction. This SHALL also apply when the user typed the leading `:` manually.

#### Scenario: Run a command
- **WHEN** the user types `:sort size --reverse` and presses Enter
- **THEN** the listing is sorted by size descending, the path bar shows the current directory and the list has focus

#### Scenario: Error keeps text
- **WHEN** the user types `:cpoy` and presses Enter
- **THEN** the status bar reports unknown command `cpoy` and the path bar still contains `:cpoy`

### Requirement: Cancel the prompt
Escape in the path bar SHALL discard the prompt text, show the current directory and focus the list, as for path editing.

#### Scenario: Cancel
- **WHEN** the user types `:sort` and presses Escape
- **THEN** no command runs, the path bar shows the current directory and the list has focus
