-- The explorer preset: Windows Explorer keys plus find-as-you-type.
-- Returned fresh by lwfm.preset("explorer"); config.lua edits its copy.
return {
  show_hidden = false,
  sort = { key = "name", reverse = false },
  colors = { dir = "#1a5fb4", hidden = "#808080" },
  editor = 'shell "notepad $f"',
  openers = {},
  keys = {
    global = {
      ["ctrl+q"] = "quit",
      ["ctrl+l"] = "focus-path",
      ["alt+d"] = "focus-path",
      ["f4"] = "focus-path",
      ["alt+left"] = "back",
      ["alt+right"] = "forward",
      ["alt+up"] = "up",
      ["f5"] = "refresh",
      ["ctrl+r"] = "refresh",
      ["ctrl+h"] = "toggle-hidden",
      ["ctrl+,"] = "edit-config",
    },
    list = {
      ["enter"] = "open",
      ["backspace"] = "up",
      [":"] = "prompt",
    },
    search = {
      ["esc"] = "search-close",
      ["enter"] = { "search-close", "open" },
      ["down"] = "search-next",
      ["up"] = "search-prev",
    },
    input = {
      ["enter"] = "path-submit",
      ["esc"] = "path-cancel",
    },
  },
}
