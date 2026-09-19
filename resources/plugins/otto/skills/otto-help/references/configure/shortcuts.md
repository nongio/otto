# Keyboard shortcuts

Shortcuts are a keyed table, not single values, so they have no identifier and
no `Set`: they are edited in the configuration file and take effect at the next
login. The Settings app's Keyboard pane edits the same table, so a binding made
there is an ordinary entry that can also be read and edited by hand.

```toml
[keyboard_shortcuts]
"Ctrl+Space" = { run = { cmd = "otto-launcher", args = [] } }
"Ctrl+1" = { builtin = "Workspace", index = 0 }
"Ctrl+Tab" = "ApplicationSwitchNext"
"Ctrl+ArrowUp" = "ToggleMaximizeWindow"
"XF86AudioRaiseVolume" = "VolumeUp"
"Logo+t" = "TilingToggle"
```

## Writing a trigger

Modifiers are `Ctrl`, `Alt`, `Shift` and `Logo`, joined with `+`, and they are
case-insensitive. Aliases are accepted: `Control` and `Primary` for `Ctrl`;
`Super`, `Meta`, `Win` and `Command` for `Logo`. Keys are keysym names —
`Escape` (or `Esc`), `ArrowUp`/`Up`, `grave`, `period`, `space`,
`ISO_Left_Tab` for Shift+Tab, `Prior`/`Next` for Page Up/Down, and the
`XF86…` names for hardware keys. `scripts/show-keys.sh` prints the keysym for
whatever is pressed.

**An unparsable trigger or action is skipped with a warning, not an error.**
Grep the log for `skipping shortcut` when a binding does nothing.

## Writing an action

Four forms:

```toml
"Ctrl+Tab" = "ApplicationSwitchNext"                              # a built-in
"Ctrl+1"   = { builtin = "Workspace", index = 0 }                 # one that takes an index
"Ctrl+Space" = { run = { cmd = "otto-launcher", args = [] } }     # run a binary
"Logo+B"   = { open_default = "browser" }                         # the default app for a role
"Logo+T"   = { open_default = { role = "terminal", fallback = "alacritty" } }
```

`run` names a binary on `PATH` and nothing else. `open_default` resolves a
**role** through the desktop's default applications — `browser`,
`file_manager` / `files`, `terminal` / `shell` — with an optional `fallback`
for when nothing is registered, and takes a desktop file id directly as well
(`open_default = "firefox.desktop"`). Use `open_default` for "open a terminal":
there is no `terminal` binary for `run` to find.

## The built-in actions

| Group | Actions |
|---|---|
| Session | `Quit`, `LockSession` |
| Windows | `CloseWindow`, `ToggleMaximizeWindow`, `TileWindowLeft`, `TileWindowRight`, `ToggleDecorations` |
| Applications | `ApplicationSwitchNext`, `ApplicationSwitchPrev`, `ApplicationSwitchNextWindow`, `ApplicationSwitchQuit` |
| Workspaces | `Workspace` (needs `index`, zero-based), `ExposeShowAll`, `ExposeShowDesktop` |
| Displays | `Screen` (needs `index`), `ScaleUp`, `ScaleDown`, `RotateOutput` |
| Hardware | `BrightnessUp`, `BrightnessDown`, `VolumeUp`, `VolumeDown`, `VolumeMute`, `MediaPlayPause`, `MediaNext`, `MediaPrev`, `MediaStop` |
| Tiling | `TilingToggle`, `FocusLeft`/`Right`/`Up`/`Down`, `MoveContainerLeft`/`Right`/`Up`/`Down`, `SplitHorizontal`, `SplitVertical`, `ResizeGrowWidth`, `ResizeShrinkWidth`, `ResizeGrowHeight`, `ResizeShrinkHeight`, `EqualizeContainer`, `FloatingToggle`, `FocusModeToggle` |
| Debugging | `SceneSnapshot`, `SkpSnapshot` |

Nothing tiling-related is bound by default; the block is in
`otto_config.example.toml`, commented out.

## An i3-style set

Otto's tiling is i3's model — a tree of splits, one tiling mode per workspace —
so an i3 keymap ports mostly as it stands. Nothing tiling-related is bound by
default, so this is a fresh block, not an override.

```toml
[keyboard_shortcuts]
# Mode key: "Logo" is Super. See the warning below if the Cmd key is in play.
"Logo+t" = "TilingToggle"                  # tile / untile this workspace

# Focus and move, vi keys. Arrow keys work too: ArrowLeft, ArrowDown, …
"Logo+h" = "FocusLeft"
"Logo+j" = "FocusDown"
"Logo+k" = "FocusUp"
"Logo+l" = "FocusRight"
"Logo+Shift+h" = "MoveContainerLeft"
"Logo+Shift+j" = "MoveContainerDown"
"Logo+Shift+k" = "MoveContainerUp"
"Logo+Shift+l" = "MoveContainerRight"

# How the next window splits this cell. Pressing the same one again disarms it.
"Logo+b" = "SplitHorizontal"
"Logo+v" = "SplitVertical"

# Resize, one `[tiling] resize_step` at a time, taken from the neighbour.
"Logo+r" = "ResizeGrowWidth"
"Logo+Shift+r" = "ResizeShrinkWidth"
"Logo+Ctrl+r" = "ResizeGrowHeight"
"Logo+Ctrl+Shift+r" = "ResizeShrinkHeight"

"Logo+e" = "EqualizeContainer"             # even out the focused container
"Logo+Shift+space" = "FloatingToggle"      # float this tile, or tile this window
"Logo+space" = "FocusModeToggle"           # focus the other layer
"Logo+Shift+q" = "CloseWindow"             # i3's kill

# Workspaces. The index is zero-based, so Logo+1 is index 0.
"Logo+1" = { builtin = "Workspace", index = 0 }
"Logo+2" = { builtin = "Workspace", index = 1 }
"Logo+3" = { builtin = "Workspace", index = 2 }
"Logo+4" = { builtin = "Workspace", index = 3 }

# The launcher, and a terminal through the desktop's default handler.
"Logo+d" = { run = { cmd = "otto-launcher", args = [] } }
"Logo+Return" = { open_default = { role = "terminal", fallback = "xdg-terminal-exec" } }
```

### Four i3 bindings that have no action yet

These exist as commands but not as named actions, so they are bound by running
`otto-msg`. It goes out over D-Bus and back, which is slower than an action and
can fail while the compositor is busy — use it only for these four, and prefer
the named action everywhere else.

```toml
"Logo+f" = { run = { cmd = "otto-msg", args = ["fullscreen"] } }
"Logo+a" = { run = { cmd = "otto-msg", args = ["focus parent"] } }
"Logo+w" = { run = { cmd = "otto-msg", args = ["layout toggle split"] } }
"Logo+Shift+1" = { run = { cmd = "otto-msg", args = ["move container to workspace 1"] } }
```

Note that `otto-msg`'s workspace numbers are one-based, while the `Workspace`
builtin's `index` is zero-based. `Logo+1` and `Logo+Shift+1` therefore disagree
by one on purpose — say so when writing both, or the person will think one of
them is a typo.

### Where i3 habits do not transfer

- **`Logo+…` cannot match at all under `altwin:ctrl_win` or
  `mac_style_modifiers`** — the Cmd key reports as Control. On that layout write
  the whole set as `Ctrl+Alt+h`, `Ctrl+Alt+Shift+h` and so on, which is Cmd+Alt
  on the keyboard. Check `input.xkb_options` before writing a `Logo` block.
- **Tiling is per workspace and off by default.** `TilingToggle` turns the
  workspace in front of you into a tiling one; a floating workspace and a tiled
  one live a swipe apart. On a floating workspace the tiling actions say so
  rather than doing something surprising.
- **Workspaces are created, never destroyed.** `Workspace` with `index = 6`
  gives you seven; Otto does not delete one when its last window closes,
  because its workspaces are named, reorderable and per monitor.
- **`layout tabbed` and `layout stacking` do not exist**, and neither does
  `resize set`. `otto-msg` says so rather than quietly doing nothing.
- **There is no resize *mode*.** i3's `$mod+r` then arrows is a binding mode,
  and binding modes are not parsed. Bind the four resize actions directly, as
  above.
- **No criteria, marks or `for_window` rules.** `[app_id="…"]` is not parsed,
  so per-application rules have no equivalent yet.
- **Moving a window to another output** is understood by `otto-msg` but not
  built yet.
- **Gaps are a setting, not a binding** — `[tiling] inner_gap` / `outer_gap`,
  live over the bus. `otto-msg gaps inner <n>` changes the workspace in front
  of you and remembers it; add `all` for the session default. See
  [tiling.md](tiling.md).

## Keys that are always on

`Logo+Q` and `Ctrl+Alt+Backspace` quit Otto immediately. `Ctrl+Alt+Escape`
locks the session, whatever holds the keyboard. `Ctrl+Alt+F1`…`F12` switch
virtual terminal and work even while locked. None of them needs an entry, and
none can be unbound.

## Worth knowing

- **With `altwin:ctrl_win` (or `mac_style_modifiers`), a `Logo+…` binding can
  never match** — the Cmd key reports as Control. Write those bindings as
  `Ctrl+Alt+h` and so on, which is Cmd+Alt on the keyboard. See
  [keyboard.md](keyboard.md).
- **Otto's shortcuts win over the focused application**, unless the application
  holds a shortcut inhibitor. A binding that "does not work" in one app and
  works everywhere else is usually that.
- **Adding a binding needs a restart.** The file is read once, at startup.
- Tiling actions want a tiling workspace; on a floating one they say so rather
  than doing something surprising.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/keyboard-shortcuts.md
- https://github.com/nongio/otto/blob/main/docs/user/scripting.md — the same
  commands from a script, through `otto-msg`
