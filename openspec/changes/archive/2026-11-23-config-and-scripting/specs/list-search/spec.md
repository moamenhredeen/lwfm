# Spec Delta

## Purpose

Defines find-as-you-type in the file list: typing opens a small search input, selects the first entry whose name contains the typed text and highlights the matches, without filtering the list, with predictable keys and a measurable responsiveness bound.

## ADDED Requirements

### Requirement: Typing opens the search input
When the list has focus, no key sequence is pending, and the user types a printable character that is not bound in the active scopes, a search input SHALL appear over the bottom-left corner of the list with that character as its text and with keyboard focus. The character SHALL NOT be delivered to the list control. The command `search <text>` SHALL open the input with the given text in the same way.

#### Scenario: Start by typing
- **WHEN** the list has focus and the user types `r`
- **THEN** the search input appears containing `r` and has focus

#### Scenario: Bound character is not searched
- **WHEN** the list has focus and the user types `:`
- **THEN** the command prompt opens (the `:` binding) and no search input appears

### Requirement: Jump to the first match
Whenever the search text changes, the first entry in display order whose name contains the text, compared case-insensitively, SHALL become the focused and only selected entry and SHALL be scrolled into view. The list SHALL NOT be filtered: all entries stay listed in their order.

#### Scenario: First match selected
- **WHEN** the directory lists `Reports` (directory), `notes.md`, `prep.txt`, `report.pdf` in that order and the user types `rep`
- **THEN** `Reports` is focused and selected, and all four entries are still listed

#### Scenario: Case-insensitive substring
- **WHEN** the same directory is listed and the user types `PREP`
- **THEN** `prep.txt` is focused and selected

### Requirement: Next and previous match
While the search input has focus, Down SHALL move the selection to the next entry after the focused one whose name contains the search text, and Up to the previous one, wrapping around at the ends. The commands `search-next` and `search-prev` SHALL do the same.

#### Scenario: Cycle through matches
- **WHEN** the user has typed `rep` (so `Reports` is selected) and presses Down twice
- **THEN** `prep.txt` and then `report.pdf` become the selected entry

#### Scenario: Wrap around
- **WHEN** `report.pdf` is the last match and selected and the user presses Down
- **THEN** `Reports` becomes the selected entry

### Requirement: Match highlighting
While the search input is open, the part of each listed name that matches the search text SHALL be drawn with a highlight background in the Name column. Highlighting SHALL disappear when the input closes.

#### Scenario: Highlight in all matches
- **WHEN** the user has typed `rep`
- **THEN** `Rep` in `Reports`, `rep` in `prep.txt` and `rep` in `report.pdf` are highlighted, and `notes.md` has no highlight

### Requirement: No match
When no name contains the search text, the selection SHALL stay where it was and the status bar SHALL report that there are no matches.

#### Scenario: Nothing matches
- **WHEN** `prep.txt` is selected and the user extends the search text to `zzz`
- **THEN** `prep.txt` stays selected and the status bar reports no matches

### Requirement: Search display
While the search input is open, the status bar SHALL show the search text and the number of matching entries (e.g. `Search: rep  3 matches`).

#### Scenario: Count shown
- **WHEN** 3 entries contain `rep`
- **THEN** the status bar shows `rep` and `3 matches`

### Requirement: Closing the search input
Escape in the search input SHALL close it, keep the current selection and return focus to the list. Enter SHALL close it and open the selected entry. Deleting all of its text, or focus moving elsewhere (e.g. a mouse click in the list), SHALL close it. Text editing inside the input (Backspace, cursor keys within the text, AltGr characters, dead keys) SHALL work as in any text field. The command `search-close` SHALL close the input.

#### Scenario: Escape keeps the selection
- **WHEN** the user types `rep`, presses Down to select `prep.txt` and presses Escape
- **THEN** the input disappears, `prep.txt` stays selected, and the list has focus

#### Scenario: Enter opens
- **WHEN** the user types `rep` (selecting `Reports`) and presses Enter
- **THEN** the input disappears and lwfm navigates into `Reports`

#### Scenario: Backspace to empty
- **WHEN** the search text is `r` and the user presses Backspace
- **THEN** the input disappears and Backspace did not navigate to the parent directory

### Requirement: Search lifetime
Navigating to another directory SHALL close the search input. `refresh`, `toggle-hidden` and `sort` SHALL keep it open and SHALL keep the highlighting up to date.

#### Scenario: Navigation closes search
- **WHEN** the search input is open and the user runs `back`
- **THEN** the input disappears in the new directory

### Requirement: Search performance
Selecting the first match and updating the highlighting after a keystroke SHALL take less than 50 ms for a local directory of 10,000 entries, and the time SHALL be reported when tracing is enabled.

#### Scenario: 10k entries
- **WHEN** a 10,000-entry directory is listed with tracing enabled and the user types a character
- **THEN** the traced search update time is under 50 ms
