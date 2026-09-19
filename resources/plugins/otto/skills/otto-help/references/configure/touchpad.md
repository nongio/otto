# Trackpad and pointer

Every setting on this page reconfigures the connected libinput devices at once.
Nothing here needs a restart. `input.scroll_speed` is read per scroll event, so
it takes effect on the next scroll.

## Exact commands

```sh
# Tap the pad to click
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.tap_enabled b true

# Tap, hold, then drag
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.tap_drag_enabled b true

# Keep dragging through a brief lift of the finger
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.tap_drag_lock_enabled b true

# How a physical click is read: "clickfinger" or "buttonareas"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.touchpad_click_method s "buttonareas"

# Ignore the pad while typing
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.touchpad_dwt_enabled b true

# Scroll direction: true = content follows the fingers
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.touchpad_natural_scroll_enabled b false

# Swap the buttons for a left-handed user
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.touchpad_left_handed b true

# Both buttons together = middle click
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.touchpad_middle_emulation_enabled b true

# Scroll multiplier, 0.1 to 2.0
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.scroll_speed d 1.2

# Pointer speed, -1.0 (slowest) to 1.0 (fastest)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.pointer_accel_speed d 0.3

# Acceleration curve: "adaptive" or "flat"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv input.pointer_accel_profile s "flat"
```

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `input.tap_enabled` | bool | live | `true` | `true`, `false` | `b true` | Treat a tap on the touchpad as a click. |
| `input.tap_drag_enabled` | bool | live | `true` | `true`, `false` | `b true` | Start a drag from a tap followed by a held touch. |
| `input.tap_drag_lock_enabled` | bool | live | `false` | `true`, `false` | `b true` | Keep a tap-drag going through a brief lift of the finger. |
| `input.touchpad_click_method` | enum | live | `clickfinger` | `clickfinger`, `buttonareas` | `s "buttonareas"` | Whether a click means finger count or button areas. |
| `input.touchpad_dwt_enabled` | bool | live | `true` | `true`, `false` | `b false` | Ignore the touchpad while the keyboard is in use. |
| `input.touchpad_natural_scroll_enabled` | bool | live | `true` | `true`, `false` | `b false` | Content follows the fingers. |
| `input.touchpad_left_handed` | bool | live | `false` | `true`, `false` | `b true` | Swap the primary and secondary buttons. |
| `input.touchpad_middle_emulation_enabled` | bool | live | `false` | `true`, `false` | `b true` | Pressing both buttons together is a middle click. |
| `input.scroll_speed` | double | live | `1.0` | 0.1 – 2.0, step 0.05 | `d 1.2` | Software multiplier applied to scroll events. |
| `input.pointer_accel_speed` | double | live | `-1.0` – `1.0` | -1.0 – 1.0, step 0.1 | `d 0.3` | Pointer acceleration, from -1 (slowest) to 1 (fastest). |
| `input.pointer_accel_profile` | enum | live | `adaptive` | `flat`, `adaptive` | `s "flat"` | Flat is raw speed; adaptive follows libinput's curve. |

## What each one means

**`tap_enabled`** — tapping the surface counts as a click without pressing the
pad down. One finger is a left click, two fingers a right click, three a middle
click. Turn it off for someone who rests their fingers on the pad and gets
clicks they did not mean.

**`tap_drag_enabled`** — tap, then immediately touch again and move: the second
touch drags as though the button were held. Only does anything with
`tap_enabled` on.

**`tap_drag_lock_enabled`** — during such a drag, lifting the finger briefly
does not end it; the drag continues when the finger comes back. Useful for
dragging further than the pad is wide. Off by default because a drag that does
not end when you lift can be surprising.

**`touchpad_click_method`** — how libinput turns a physical press into a button:

- `clickfinger`: the number of fingers on the pad decides. One finger left, two
  right, three middle, anywhere on the surface. This is Otto's default and what
  most laptops with a single-piece pad want.
- `buttonareas`: where you press decides. The bottom of the pad is split into a
  left button and a right button, and the rest of the surface is a left click.
  This is what a pad with printed button areas expects.

**`touchpad_dwt_enabled`** — "disable while typing". libinput ignores the pad
for a short window after each keystroke, so a palm brushing the surface
mid-sentence does not move the cursor. Turn it off if you use the pad and the
keyboard genuinely at the same time.

**`touchpad_natural_scroll_enabled`** — which way two-finger scrolling goes. On
means the content follows the fingers, as on a phone. Off means the fingers
move the scrollbar. Otto defaults to on.

**`touchpad_left_handed`** — swaps the primary and secondary buttons. Applies to
touchpads; a mouse with its own left-handed support is not covered by this
setting.

**`touchpad_middle_emulation_enabled`** — pressing the left and right buttons
together produces a middle click, for hardware with no middle button.

**`scroll_speed`** — a multiplier Otto applies to scroll events after libinput
has produced them, not a libinput option. `1.0` leaves them alone, `2.0`
doubles, `0.5` halves. Use it when a mouse wheel's notches travel too far.
Finger scrolling is already reported in the finger's own travel, so there is
little reason to go past `2.0`.

**`pointer_accel_speed`** — libinput's speed setting, from `-1.0` (slowest) to
`1.0` (fastest), with `0.0` as the device's normal speed. It applies to **every**
pointing device, mice included, not just touchpads.

**`pointer_accel_profile`** — the shape of the acceleration curve:

- `adaptive`: the pointer moves further when you move faster. The usual desktop
  feel, and the default.
- `flat`: no acceleration at all, a constant ratio between hand and pointer.
  What games and precise drawing usually want.

## What Otto does and does not send to libinput

Otto applies exactly the eight touchpad options and the two acceleration
options above (`src/udev/input_config.rs`). Everything else libinput can
configure is left at the device's default. **Do not offer a setting from this
second list — there is no identifier for it and no way to set it.**

| libinput feature | In Otto? | Note |
|---|---|---|
| Tap to click, tap-and-drag, drag lock | **Yes** | `input.tap_*` |
| Tap finger mapping (which count is which button) | No | Fixed at libinput's default: 1 left, 2 right, 3 middle |
| Click method | **Yes** | `input.touchpad_click_method` |
| Disable while typing | **Yes** | `input.touchpad_dwt_enabled` |
| Natural scrolling | **Yes** | `input.touchpad_natural_scroll_enabled` |
| Left-handed | **Yes** | `input.touchpad_left_handed` |
| Middle-button emulation | **Yes** | `input.touchpad_middle_emulation_enabled` |
| Pointer acceleration speed and profile | **Yes** | `input.pointer_accel_*`, every pointer device |
| Custom acceleration curve (libinput's `custom` profile) | No | Only `flat` and `adaptive` |
| Scroll method (two-finger, edge, button) | No | Two-finger on a touchpad, wheel on a mouse |
| Scroll button and button lock | No | For trackpoint-style button scrolling |
| Disable-while-trackpointing | No | |
| Send-events mode (disable a device, or disable it while a mouse is plugged in) | No | A device cannot be turned off from the config |
| Rotation angle | No | For a trackball |
| Calibration matrix | No | For a touchscreen or tablet |
| Tablet pressure curves, area mapping | No | Comes from the application |

## Worth knowing

- **Touchpad settings are for touchpads only.** Otto decides what is a touchpad
  by asking libinput for the tap finger count: a device that reports zero is a
  mouse and keeps its own button behaviour. So `touchpad_left_handed` will not
  swap a mouse's buttons.
- **Acceleration is the exception** — `pointer_accel_speed` and
  `pointer_accel_profile` go to every pointing device.
- **Hardware differs, and a setting can be accepted but do nothing.** Otto asks
  libinput whether each feature is available before setting it, and libinput
  refuses what the hardware cannot do. A `Set` that answers `applied` means the
  value was stored and sent, not that the pad obeyed it.
- **A device plugged in later gets the current values**, not the ones Otto
  started with.
- **Gestures are not configurable.** Otto's touchpad gestures — swipe between
  workspaces, pinch for exposé — are built in and have no settings.
- Scroll *acceleration* (speed varying with how fast the wheel spins) is not
  implemented.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/input.md
- https://github.com/nongio/otto/blob/main/docs/user/gestures.md
- libinput's own documentation: https://wayland.freedesktop.org/libinput/doc/latest/
