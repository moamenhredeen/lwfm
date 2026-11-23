# external-commands Specification

## Purpose

Defines how lwfm hands work to other programs: the `shell` command that runs a command line with the current context substituted, and openers that choose how files are opened.

## Requirements

### Requirement: Shell command
The `shell <command line>` command SHALL run the command line through `cmd.exe /c` with the current directory as working directory, without blocking the window. By default the process SHALL get no console window; with `--console` it SHALL get its own new console window. For a process started without a console, a non-zero exit code SHALL be reported in the status bar.

#### Scenario: Launch a GUI tool
- **WHEN** the user runs `shell "code $d"` in `C:\work`
- **THEN** VS Code opens `C:\work`, no console window appears, and the window stays responsive

#### Scenario: Terminal program
- **WHEN** the user runs `shell --console "nvim $f"`
- **THEN** nvim opens the focused file in a new console window

#### Scenario: Failing command
- **WHEN** the user runs `shell "doesnotexist"`
- **THEN** the status bar reports that the command exited with a non-zero code

### Requirement: Context variables
Before running, the command line SHALL have these variables substituted, each path wrapped in double quotes: `$f` the focused entry's full path, `$fs` the full paths of the selected entries separated by spaces (the focused entry when nothing is selected), `$d` the current directory. `$$` SHALL produce a literal `$`. Using `$f` or `$fs` when no entry is focused SHALL fail with an error and run nothing.

#### Scenario: Selection substitution
- **WHEN** `C:\w\a b.txt` and `C:\w\c.txt` are selected and the user runs `shell "7z a out.7z $fs"`
- **THEN** the executed command line is `7z a out.7z "C:\w\a b.txt" "C:\w\c.txt"`

#### Scenario: Literal dollar
- **WHEN** the user runs `shell "echo $$HOME"`
- **THEN** the executed command line is `echo $HOME`

#### Scenario: Nothing focused
- **WHEN** the directory is empty and the user runs `shell "notepad $f"`
- **THEN** the status bar reports that no entry is focused and nothing is started

### Requirement: Openers
The `openers` setting SHALL map file name patterns (`*` and `?` wildcards, matched case-insensitively against the file name) to an action: a command string or a Lua function. When `open` is run on a file, the matching pattern with the most characters SHALL be used and its action run with the same context variables as `shell`. When no pattern matches, the file SHALL open with its Windows association. Openers SHALL NOT apply to directories.

#### Scenario: Opener used
- **WHEN** the config sets `openers = { ["*.md"] = 'shell --console "nvim $f"' }` and the user opens `README.md`
- **THEN** `README.md` opens in nvim in a console window instead of the associated application

#### Scenario: Most specific pattern wins
- **WHEN** openers exist for `*.txt` and `*.log.txt`, and the user opens `app.log.txt`
- **THEN** the `*.log.txt` opener runs

#### Scenario: No match falls back
- **WHEN** no opener matches `photo.jpg` and the user opens it
- **THEN** it opens with its Windows association
