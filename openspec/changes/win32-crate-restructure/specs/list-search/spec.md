# Spec Delta

## MODIFIED Requirements

### Requirement: Match highlighting
While the search input is open, the part of each listed name that matches the search text SHALL be drawn with a highlight background in the Name column. Highlighting SHALL disappear when the input closes. The highlighted text SHALL stay readable against its highlight background: when a high-contrast theme is active, the highlight SHALL use the system's highlight background and highlight text colors instead of the fixed highlight color.

#### Scenario: Highlight in all matches
- **WHEN** the user has typed `rep`
- **THEN** `Rep` in `Reports`, `rep` in `prep.txt` and `rep` in `report.pdf` are highlighted, and `notes.md` has no highlight

#### Scenario: High-contrast theme
- **WHEN** a high-contrast theme is active and the user has typed `rep`
- **THEN** the matched parts are drawn with the system highlight background and highlight text colors
