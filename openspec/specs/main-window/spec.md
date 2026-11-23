# main-window Specification

## Purpose

Defines the lwfm application window: its layout (path bar, file list, status bar), how it starts, and the responsiveness and startup-time guarantees the rest of the application relies on.

## Requirements

### Requirement: Window layout
The application SHALL show a single top-level window containing, from top to bottom, a path bar showing the current directory, a file list filling the remaining space, and a status bar. The window title SHALL contain the current directory path and the application name.

#### Scenario: Initial layout
- **WHEN** the application starts
- **THEN** the window shows a path bar, a file list and a status bar, and the title contains the current directory and "lwfm"

#### Scenario: Resize
- **WHEN** the user resizes the window
- **THEN** the path bar and status bar keep their height and span the full width, and the file list fills the remaining area

### Requirement: Start directory
The application SHALL accept an optional directory path as its first command-line argument and open it on start. Without an argument, or when the given path is not an accessible directory, it SHALL open the user's home directory and, in the latter case, report the problem in the status bar.

#### Scenario: Start with argument
- **WHEN** the user runs `lwfm C:\Windows`
- **THEN** the window opens listing `C:\Windows`

#### Scenario: Start without argument
- **WHEN** the user runs `lwfm` with no arguments
- **THEN** the window opens listing the user's home directory

#### Scenario: Start with invalid argument
- **WHEN** the user runs `lwfm C:\does-not-exist`
- **THEN** the window opens listing the user's home directory and the status bar reports that `C:\does-not-exist` could not be opened

### Requirement: Responsive UI
The window SHALL keep processing input, repainting and resizing while a directory is being enumerated or a file is being opened, regardless of how slow the underlying storage is.

#### Scenario: Slow or unreachable location
- **WHEN** the user navigates to a network path that takes many seconds to respond
- **THEN** the window can still be moved, resized and repainted, and the user can navigate elsewhere before the slow path responds

### Requirement: Status bar
The status bar SHALL show the number of items in the current listing and the number of selected items. Messages and errors from commands and navigation SHALL be shown in the status bar instead of modal dialogs, and remain visible until replaced by the next message or the next successful navigation.

#### Scenario: Item counts
- **WHEN** a directory with 12 visible items is listed and the user selects 3 of them
- **THEN** the status bar shows 12 items and 3 selected

#### Scenario: Error message
- **WHEN** a command fails
- **THEN** the status bar shows the error and no modal dialog is opened

### Requirement: High DPI rendering
The window SHALL be per-monitor DPI aware: text and layout SHALL render at the native resolution of the monitor the window is on, and re-scale when the window moves to a monitor with a different scale factor.

#### Scenario: Move between monitors
- **WHEN** the window is moved from a 100% scale monitor to a 150% scale monitor
- **THEN** fonts and layout re-scale to 150% without blurriness

### Requirement: Startup performance
On a local SSD, the time from process start to the first painted listing of the start directory SHALL be under 100 ms for a directory of up to 1,000 entries. The application SHALL be able to report this measurement when tracing is enabled.

#### Scenario: Cold start budget
- **WHEN** the application is started with tracing enabled on a directory of 1,000 local entries
- **THEN** the reported time from process start to first painted listing is under 100 ms

### Requirement: Quit
The application SHALL exit when the window is closed or the `quit` command is run, without leaving background threads that keep the process alive.

#### Scenario: Quit during enumeration
- **WHEN** the user quits while a slow directory is still being enumerated
- **THEN** the process exits promptly
