# Keyboard

The layout, the options that remap keys, and how fast a held key repeats.
All of it applies live: a new keymap replaces the seat's and goes straight to
whichever window has the keyboard.

## Exact commands

```sh
# Layout, and a variant. One layout, or several separated by commas
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.xkb_layout s "us"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.xkb_variant s "dvorak"

# Options. The number says how many follow, so add it up
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.xkb_options as 1 "caps:escape"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.xkb_options as 2 "caps:escape" "compose:ralt"

# Clear the options again
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.xkb_options as 0

# Key repeat: milliseconds before it starts, then repeats per second
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv keyboard_repeat_delay i 250
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv keyboard_repeat_rate i 40
```

**Setting `input.xkb_options` replaces the whole list.** To add one, read the
current value first and send every option you want to keep.

**Apply** is `live` (now) or `restart` (at the next login).

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `input.xkb_layout` | string | live | `""` (empty) | free text | `s "us"` | XKB layout name. Empty uses the system default. |
| `input.xkb_variant` | string | live | `""` (empty) | free text | `s "dvorak"` | XKB variant name. Empty uses the system default. |
| `input.xkb_options` | string-list | live | `[]` | list of strings | `as 1 "caps:escape"` | XKB option strings. |
| `keyboard_repeat_delay` | int | live | `300` | 100 – 2000, step 25 | `i 250` | Milliseconds a key is held before it starts repeating. |
| `keyboard_repeat_rate` | int | live | `30` | 1 – 100, step 1 | `i 40` | Repeats per second while a key is held. |

Note that the two repeat settings are **top-level**, not in `[input]`. Otto
sends them to clients over `wl_keyboard`, so applications repeat at the rate set
here.

## In the file

```toml
[input]
xkb_layout = "us,ru"
xkb_variant = "dvorak"
xkb_options = ["caps:escape", "grp:win_space_toggle"]
mac_style_modifiers = true
```

Several layouts go in one comma-separated string, with an option to switch
between them.

## Options worth knowing

| Option | Effect |
|---|---|
| `caps:escape` | Caps Lock becomes Escape |
| `caps:swapescape` | Swap Caps Lock and Escape |
| `ctrl:swapcaps` | Swap Ctrl and Caps Lock |
| `ctrl:nocaps` | Caps Lock becomes another Ctrl |
| `altwin:ctrl_win` | Super becomes Ctrl — the Cmd-key layout |
| `compose:ralt` | Right Alt is Compose, for accented characters |
| `grp:win_space_toggle` | Super+Space switches layout |

`xkbcli list` prints every layout, variant and option; `man xkeyboard-config` is
the reference.

## Mac-style modifiers

`altwin:ctrl_win` maps the Cmd keys onto Ctrl so `Cmd+C` reaches applications as
the `Ctrl+C` they expect. The catch: Cmd and the real Ctrl then produce the same
event, and a `Ctrl+W` binding would fire from both — closing the window when
`^W` was meant to delete a word.

With that option set, Otto matches its own shortcuts on **Cmd alone** and leaves
the real Ctrl key to the focused application. Bindings are still written
`Ctrl+…` in the config; they simply follow the Cmd key. `mac_style_modifiers`
forces that behaviour either way, and has no identifier — it is file-only.

`Ctrl+Alt+Backspace` follows the same rule and becomes `Cmd+Alt+Backspace`. VT
switching is read from raw keycodes and works from either key.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/input.md
- [shortcuts.md](shortcuts.md) — what the keys are bound to
