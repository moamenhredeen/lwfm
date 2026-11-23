# Spec Delta

## MODIFIED Requirements

### Requirement: Built-in commands
The registry SHALL provide at least these commands: `cd <path>`, `up`, `back`, `forward`, `open`, `refresh`, `toggle-hidden`, `sort <name|size|mtime> [--reverse]`, `focus-path`, `focus-list`, `path-submit`, `path-cancel`, `quit`, `shell <command line> [--console]`, `search <text>`, `search-next`, `search-prev`, `search-close`, `prompt`, `edit-config` and `reload-config`. All user actions (keys, mouse, path bar, command prompt, Lua) SHALL be carried out by running a registry command.

#### Scenario: Command performs action
- **WHEN** the command string `toggle-hidden` is run
- **THEN** hidden-item visibility toggles exactly as when the bound key is pressed

#### Scenario: Sort command
- **WHEN** the command string `sort size --reverse` is run
- **THEN** the listing is sorted by size descending, directories first

#### Scenario: Search command
- **WHEN** the command string `search rep` is run
- **THEN** the search input opens containing `rep` and the first entry whose name contains `rep` is selected, as when typing `rep`
