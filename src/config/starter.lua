-- lwfm configuration. Saved changes apply immediately.
-- Check from a terminal with: lwfm --check-config

local lwfm = require("lwfm")
local cfg = lwfm.preset("explorer") -- Windows Explorer keys; typing in the list searches

-- Settings -----------------------------------------------------------------

cfg.show_hidden = false                        -- ctrl+h toggles at runtime
cfg.sort = { key = "name", reverse = false }   -- name | size | mtime
cfg.colors = { dir = "#1a5fb4", hidden = "#808080" }

-- Command used by ctrl+, (edit-config); $f is this file.
-- For a terminal editor: 'shell --console "nvim $f"'
cfg.editor = 'shell "notepad $f"'

-- Keys ---------------------------------------------------------------------
-- Scopes: global, list, search (the search input), input (path bar).
-- A value is a command string, a list of commands, a Lua function,
-- or { run = ..., desc = "shown in the hint bar" }. "noop" unbinds.
-- In shell commands: $f focused file, $fs selection, $d current folder.

-- cfg.keys.global["ctrl+e"] = 'shell "code $d"'
-- cfg.keys.global["ctrl+t"] = 'shell "wt -d $d"'
-- cfg.keys.global["ctrl+g d"] = { run = "cd ~/Downloads", desc = "Downloads" }
-- cfg.keys.global["ctrl+g p"] = { run = "cd ~/projects", desc = "Projects" }
-- cfg.keys.list["ctrl+i"] = function(fm) fm:notify(fm.file or fm.dir) end

-- Openers: choose the program for "open" by file name pattern -------------

-- cfg.openers["*.md"] = 'shell --console "nvim $f"'
-- cfg.openers["*.log"] = 'shell "notepad $f"'

-- Events -------------------------------------------------------------------

-- lwfm.on("dir_enter", function(fm, dir)
--   if dir:match("Downloads$") then fm:sort("mtime", { reverse = true }) end
-- end)

-- Your own modules go in the lua folder next to this file: require("mymodule")

return cfg
