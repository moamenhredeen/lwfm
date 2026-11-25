# Spec Delta

## ADDED Requirements

### Requirement: System font changes
The window SHALL follow changes of the system message font while it is running: when the system reports that this font changed, the path bar, file list, status bar and search input SHALL switch to the new font and the layout SHALL be recomputed, without restarting the application and without losing the listing, the selection or the keyboard focus.

#### Scenario: System font changed while running
- **WHEN** lwfm is showing a listing and the system message font changes (for example through the accessibility text size setting)
- **THEN** the path bar, file list and status bar are drawn with the new font, and the current directory, selection and focus are unchanged
