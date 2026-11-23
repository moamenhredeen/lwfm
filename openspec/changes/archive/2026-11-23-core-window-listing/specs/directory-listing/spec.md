# Spec Delta

## Purpose

Defines how lwfm reads a directory and presents its entries: asynchronous enumeration with cancellation, columns, sort order, hidden-item visibility and error handling, at a speed suited to very large directories.

## ADDED Requirements

### Requirement: Asynchronous enumeration
Directory contents SHALL be enumerated off the UI thread. Entries SHALL appear progressively in batches while enumeration runs, and the status bar SHALL indicate that loading is in progress until enumeration completes.

#### Scenario: Large directory loads progressively
- **WHEN** the user opens a directory with 200,000 entries
- **THEN** entries start appearing before enumeration completes, the status bar shows loading progress, and the window stays responsive

### Requirement: Stale results are discarded
When the user navigates away while a directory is still being enumerated, the pending enumeration SHALL be abandoned and none of its results SHALL appear in the new listing.

#### Scenario: Navigate during enumeration
- **WHEN** the user opens directory A, and navigates to directory B before A finishes loading
- **THEN** the list shows only entries of B, never entries of A

### Requirement: Columns
The list SHALL show the columns Name, Size and Modified. Size SHALL be shown in human-readable binary units (e.g. `12 KB`, `3.4 MB`) for files and left empty for directories. Modified SHALL be shown as local time in the format `YYYY-MM-DD HH:MM`. Directories SHALL be visually distinguishable from files.

#### Scenario: File and directory rows
- **WHEN** a directory containing a subdirectory `docs` and a 12,288-byte file `a.txt` modified at 2026-11-23 14:05 local time is listed
- **THEN** `docs` shows no size and is marked as a directory, and `a.txt` shows `12 KB` and `2026-11-23 14:05`

### Requirement: Sorting
The list SHALL be sorted with directories before files. Within each group, entries SHALL be sorted by the active sort key (name, size or modified), ascending or descending. Name sorting SHALL be case-insensitive and treat digit runs as numbers. The default sort SHALL be name ascending. Clicking a column header SHALL sort by that column, and clicking it again SHALL reverse the order. Sorting SHALL keep the current selection on the same entries.

#### Scenario: Natural name order
- **WHEN** a directory contains `file10.txt`, `File2.txt` and `file1.txt`
- **THEN** with name ascending the order is `file1.txt`, `File2.txt`, `file10.txt`

#### Scenario: Directories first
- **WHEN** a directory contains a file `a.txt` and a subdirectory `z`
- **THEN** `z` is listed before `a.txt` for every sort key and direction

#### Scenario: Toggle sort direction via header
- **WHEN** the user clicks the Size header twice
- **THEN** files are sorted by size descending

#### Scenario: Selection survives re-sort
- **WHEN** the user has `b.txt` selected and changes the sort key
- **THEN** `b.txt` is still selected after sorting

### Requirement: Hidden items
Entries with the hidden or system attribute SHALL be hidden by default. Toggling hidden-item visibility SHALL show or hide them without re-reading the directory. When shown, hidden items SHALL be visually dimmed.

#### Scenario: Default hides hidden items
- **WHEN** a directory containing a hidden file `secret.txt` is listed with default settings
- **THEN** `secret.txt` is not shown

#### Scenario: Toggle hidden items
- **WHEN** the user runs `toggle-hidden`
- **THEN** `secret.txt` appears dimmed, and running `toggle-hidden` again hides it

### Requirement: Refresh
The `refresh` command SHALL re-read the current directory, keeping the selection on entries that still exist.

#### Scenario: Refresh picks up new file
- **WHEN** a file is created in the current directory by another program and the user runs `refresh`
- **THEN** the new file appears and the previously selected entry is still selected

### Requirement: Long paths
Directories whose full path exceeds 260 characters SHALL be listed and navigable like any other directory.

#### Scenario: Deep directory
- **WHEN** the user navigates to a directory whose path is 400 characters long
- **THEN** its entries are listed

### Requirement: Enumeration errors
If a directory cannot be read (access denied, not found, device not ready, network unreachable), the application SHALL keep showing the previous listing and report the error with the path in the status bar.

#### Scenario: Access denied
- **WHEN** the user opens a directory they lack permission to read
- **THEN** the previous listing stays visible and the status bar reports access denied for that path

### Requirement: Listing performance
For a local directory of 10,000 entries, the time from enumeration completing to the sorted listing being painted SHALL be under 50 ms.

#### Scenario: 10k entries budget
- **WHEN** a local directory with 10,000 entries is opened with tracing enabled
- **THEN** the reported time between enumeration complete and listing painted is under 50 ms
