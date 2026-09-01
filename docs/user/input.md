# Input

Keyboard layout and repeat, touchpad behaviour, pointer acceleration and
scrolling. Everything here lives under `[input]` except the two keyboard-repeat
keys, which are top-level.

## Keyboard layout

```toml
[input]
xkb_layout = "us"
xkb_variant = "dvorak"
xkb_options = ["caps:escape"]
```

These are standard XKB settings — the same names `setxkbmap` uses.

### Multiple layouts

```toml
[input]
xkb_layout = "us,ru"
xkb_options = ["grp:win_space_toggle", "caps:escape"]
```

`grp:win_space_toggle` binds `Logo+Space` to cycle layouts. Other common
switchers: `grp:alt_shift_toggle`, `grp:caps_toggle`, `grp:win_space_toggle`.

### Useful options

| Option | Effect |
|--------|--------|
| `caps:escape` | Caps Lock becomes Escape |
| `caps:swapescape` | Swap Caps Lock and Escape |
| `ctrl:swapcaps` | Swap Ctrl and Caps Lock |
| `ctrl:nocaps` | Caps Lock becomes another Ctrl |
| `altwin:ctrl_win` | Super becomes Ctrl |
| `compose:ralt` | Right Alt is the Compose key, for accented characters |
| `terminate:ctrl_alt_bksp` | Ctrl+Alt+Backspace terminates — already built in |

Discover what is available:

```sh
xkbcli list                    # every layout, variant and option
man xkeyboard-config           # the full reference
./scripts/show-keys.sh         # print the keysym for whatever you press
```

Layout changes apply at startup. Edit and restart the session to change them.

### Mac-style modifiers

`altwin:ctrl_win` maps the Cmd keys onto Ctrl, so `Cmd+C`, `Cmd+V` and `Cmd+X`
reach applications as the `Ctrl+C`/`Ctrl+V`/`Ctrl+X` they expect. The catch is
that Cmd and the real Ctrl key then produce the same event, and a binding like
`Ctrl+W` fires from both — closing the window when you meant `^W` to delete a
word in a terminal.

With this option set, Otto reads the physical keycode behind the modifier and
matches shortcuts on **Cmd alone**. The real Ctrl key is left to the focused
application:

| You press | Otto | Application receives |
|-----------|------|----------------------|
| `Cmd+W` | matches a `Ctrl+W` binding | — (Otto consumed it) |
| `Ctrl+W` | no match | `^W` — deletes a word in a terminal |
| `Cmd+C` | no match unless you bound one | `Ctrl+C` — copies |

Bindings are still written as `Ctrl+...` in the config; they simply follow the
Cmd key. Nothing changes for layouts without this option, where Ctrl behaves
normally.

This follows the `altwin:ctrl_win` option, since that is the only layout it
makes sense for. To force it either way, set it explicitly:

```toml
[input]
mac_style_modifiers = true
```

Note that the built-in `Ctrl+Alt+Backspace` follows the same rule and becomes
`Cmd+Alt+Backspace`. VT switching (`Ctrl+Alt+F1`) is read from raw keycodes and
keeps working from either key.

## Key repeat

```toml
keyboard_repeat_delay = 300    # ms before repeat starts
keyboard_repeat_rate = 30      # repeats per second
```

These are top-level keys, not inside `[input]`. Otto sends them to clients over
`wl_keyboard`, so applications repeat at the rate you set.

## Touchpad

```toml
[input]
tap_enabled = true
tap_drag_enabled = true
tap_drag_lock_enabled = false
touchpad_click_method = "clickfinger"
touchpad_dwt_enabled = true
touchpad_natural_scroll_enabled = true
touchpad_left_handed = false
touchpad_middle_emulation_enabled = false
```

| Option | Default | Effect |
|--------|---------|--------|
| `tap_enabled` | `true` | Tap to click: 1 finger = left, 2 = right, 3 = middle |
| `tap_drag_enabled` | `true` | Tap, hold, then drag |
| `tap_drag_lock_enabled` | `false` | Keep dragging after lifting your finger briefly |
| `touchpad_click_method` | `"clickfinger"` | How a physical click maps to a button — see below |
| `touchpad_dwt_enabled` | `true` | Disable the touchpad while typing |
| `touchpad_natural_scroll_enabled` | `true` | Reversed ("natural") two-finger scrolling |
| `touchpad_left_handed` | `false` | Swap left and right buttons |
| `touchpad_middle_emulation_enabled` | `false` | Left + right pressed together = middle click |

### Click method

| Value | Behaviour |
|-------|-----------|
| `"clickfinger"` | The number of fingers on the pad decides: 1 = left, 2 = right, 3 = middle |
| `"buttonareas"` | Traditional: where you click decides — bottom-right corner = right click |

`clickfinger` is the default, and is what GNOME and KDE use too. `buttonareas` is
what most Windows laptops do.

These settings apply to **touchpads only**, not to mice. Some hardware does not
support every option; libinput ignores what it cannot do.

## Pointer

```toml
[input]
pointer_accel_speed = 0.0        # -1.0 (slowest) to 1.0 (fastest)
pointer_accel_profile = "adaptive"
scroll_speed = 1.0
```

`pointer_accel_speed` and `pointer_accel_profile` apply to **all** pointing
devices — mice and touchpads alike.

| Profile | Behaviour |
|---------|-----------|
| `"adaptive"` | Speed depends on how fast you move — the usual desktop feel |
| `"flat"` | No acceleration; 1:1 movement, which gamers usually want |

`scroll_speed` is a software multiplier applied to scroll events. `1.0` leaves
them alone, `2.0` doubles them, `0.5` halves them. Use it when a mouse wheel's
notches move too far. The Settings app offers `0.1` to `2.0`; libinput reports
finger scrolling in the same units as pointer motion, so `1.0` is already the
finger's own travel and there is little reason to go past double it.

Scroll *acceleration* (speed varying with how fast you spin the wheel) is not
implemented.

## Touchscreen

Touch input works for ordinary interaction — tap, drag, and window move and
resize requests from applications. There are no touchscreen gestures; the
[gestures](gestures.md) documented for Otto are touchpad-only.

## Tablets

Otto implements the tablet protocol (`wp-tablet-v2`), so drawing tablets with
pressure and tilt work in applications that support them. There are no
tablet-specific settings — mapping and pressure curves come from the
application.

## Input methods and virtual devices

Otto implements the protocols an input-method framework needs, so IBus, fcitx5
and similar work for CJK and other complex input: `text-input`, `input-method`
and `virtual-keyboard`.

`zwlr-virtual-pointer-v1` is also implemented, which is what tools like
`wtype`, `ydotool` and the [RDP bridge](remote-desktop.md) use to synthesize
input.

## Shortcut inhibition

Applications can ask Otto to suspend its keyboard shortcuts while they are
focused, via `keyboard-shortcuts-inhibit`. Remote-desktop viewers, virtual
machines and terminal multiplexers use this so keys like `Ctrl+Tab` reach the
guest rather than being eaten by Otto.

The always-on keys (`Logo+Q`, `Ctrl+Alt+Backspace`, VT switching,
`Ctrl+Alt+Escape`, the power button) are never inhibited. See
[Keyboard Shortcuts](keyboard-shortcuts.md#always-on-keys).

## Troubleshooting

**Touchpad settings are ignored.** They apply to touchpads only. Confirm
libinput classifies your device as one:

```sh
libinput list-devices
```

Check the `Tap-to-click` and `Click methods` lines — a device that reports
`n/a` for a capability cannot do it.

**The layout does not change.** Layout is applied at startup. Restart the
session. A typo in `xkb_layout` leaves you on the previous layout with a warning
in the log.

**Keys produce the wrong characters.** Check `xkb_variant` — many layouts
(`dvorak`, `colemak`, `intl`) are variants of a base layout, not layouts in
their own right.

**A shortcut does nothing in one app but works elsewhere.** That app has
probably taken a shortcuts inhibitor. That is intentional.
