# command-registry Specification

## Purpose

Defines the named-command layer of lwfm: a single registry of commands with arguments and a textual command syntax. Every user action goes through it, so keymaps, prompts and future scripting share one API.

## Requirements

### Requirement: Built-in commands
The registry SHALL provide at least these commands: `cd <path>`, `up`, `back`, `forward`, `open`, `refresh`, `toggle-hidden`, `sort <name|size|mtime> [--reverse]`, `focus-path`, `focus-list` and `quit`. All user actions in this change (keys, mouse, path bar) SHALL be carried out by running a registry command.

#### Scenario: Command performs action
- **WHEN** the command string `toggle-hidden` is run
- **THEN** hidden-item visibility toggles exactly as when the bound key is pressed

#### Scenario: Sort command
- **WHEN** the command string `sort size --reverse` is run
- **THEN** the listing is sorted by size descending, directories first

### Requirement: Command string syntax
A command string SHALL consist of a command name followed by whitespace-separated arguments. Arguments containing whitespace SHALL be written in double quotes, with `\"` for a literal quote inside quotes. Options SHALL be written as `--name` or `--name=value`.

#### Scenario: Quoted argument
- **WHEN** the command string `cd "C:\Program Files"` is run
- **THEN** the listing shows `C:\Program Files`

#### Scenario: Unterminated quote
- **WHEN** the command string `cd "C:\Program Files` is parsed
- **THEN** parsing fails with an error naming the unterminated quote, and nothing is executed

### Requirement: Command errors
Running an unknown command, or a known command with missing, extra or invalid arguments, SHALL fail with a descriptive error shown in the status bar and SHALL NOT change application state or terminate the application.

#### Scenario: Unknown command
- **WHEN** the command string `cpoy` is run
- **THEN** the status bar reports unknown command `cpoy` and nothing else changes

#### Scenario: Invalid argument
- **WHEN** the command string `sort color` is run
- **THEN** the status bar reports that `color` is not a valid sort key and the sort order is unchanged

### Requirement: Commands are enumerable
The registry SHALL be able to list every command with its name, argument signature and a one-line description, so later features (help, completion, scripting bindings) can be generated from it.

#### Scenario: List commands
- **WHEN** the registry is asked for its commands
- **THEN** it returns every built-in command with name, arguments and description
