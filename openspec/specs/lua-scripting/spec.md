# lua-scripting Specification

## Purpose

Defines the Lua scripting surface of lwfm: the `fm` object that Lua functions receive, how registry commands are exposed to Lua, event hooks, user modules, and the guarantees that a misbehaving script cannot crash or freeze the window.

## Requirements

### Requirement: The fm object
Every Lua function that lwfm calls (key bindings, openers, events) SHALL receive an `fm` object as its first argument with read-only fields: `fm.dir` (current directory as a string), `fm.file` (full path of the focused entry, or `nil`), and `fm.selection` (array of full paths of the selected entries, empty when nothing is selected). The fields SHALL reflect the state at the moment the function is called.

#### Scenario: Read state
- **WHEN** the current directory is `C:\work`, `a.txt` and `b.txt` are selected, `b.txt` is focused, and a bound Lua function runs `fm:notify(fm.file .. " " .. #fm.selection)`
- **THEN** the status bar shows `C:\work\b.txt 2`

### Requirement: Commands as methods
Every registry command SHALL be callable as a method on `fm`, with `-` in its name replaced by `_`. Positional arguments SHALL be passed as Lua strings, and options as a final table (`fm:sort("size", { reverse = true })`). `fm:cmd(str)` SHALL run a command string. A command error SHALL raise a Lua error carrying the command's error message. `fm:notify(msg)` SHALL show a message in the status bar.

#### Scenario: Method call
- **WHEN** a bound Lua function runs `fm:sort("size", { reverse = true })`
- **THEN** the listing is sorted by size descending, exactly as with the command string `sort size --reverse`

#### Scenario: Hyphenated command
- **WHEN** a bound Lua function runs `fm:toggle_hidden()`
- **THEN** hidden-item visibility toggles

#### Scenario: Command error inside Lua
- **WHEN** a bound Lua function runs `fm:sort("color")` without handling the error
- **THEN** the function stops and the status bar shows that `color` is not a valid sort key

### Requirement: Events
`lwfm.on(event, fn)` SHALL register a handler; several handlers per event SHALL run in registration order. The `startup` event SHALL fire once, after the first directory has been listed completely. The `dir_enter` event SHALL fire with `(fm, dir)` after each navigation to a directory has been listed completely, including the start directory; it SHALL NOT fire for `refresh`. Registering an unknown event name SHALL be a config error.

#### Scenario: dir_enter handler
- **WHEN** the config registers `lwfm.on("dir_enter", function(fm, dir) if dir:match("Downloads$") then fm:sort("mtime", { reverse = true }) end end)` and the user navigates to `C:\Users\me\Downloads`
- **THEN** the Downloads listing is sorted newest first

#### Scenario: Unknown event
- **WHEN** the config calls `lwfm.on("on_copy", fn)`
- **THEN** the config is rejected with an error naming the unknown event `on_copy` and listing `startup`, `dir_enter`

### Requirement: User modules
`require` SHALL find Lua modules in the `lua` directory next to the active config file (`<config dir>\lua\<name>.lua` and `<config dir>\lua\<name>\init.lua`). The standard Lua libraries except `debug` SHALL be available.

#### Scenario: Require a module
- **WHEN** `config.lua` contains `local bm = require("bookmarks")` and `lua\bookmarks.lua` exists next to it
- **THEN** the module is loaded and its return value is available to the config

### Requirement: Script time limit
A Lua function called by lwfm SHALL be aborted when it runs longer than 50 ms, and evaluating the config SHALL be aborted when it runs longer than 1 s. An aborted script SHALL be reported in the status bar as taking too long, and the window SHALL remain responsive.

#### Scenario: Endless loop in a binding
- **WHEN** a key is bound to `function() while true do end end` and the user presses it
- **THEN** within about 50 ms the function is aborted, the status bar reports that the script took too long, and the window keeps responding

### Requirement: Script errors are contained
A Lua error raised by a binding, opener or event handler SHALL NOT terminate lwfm or change the active configuration; the status bar SHALL show the error message including the Lua source position when available.

#### Scenario: Runtime error in binding
- **WHEN** a key is bound to `function(fm) error("boom") end` and the user presses it
- **THEN** the status bar shows an error containing `boom` and lwfm keeps running with the same config
