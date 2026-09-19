# Touchpad Gestures

Otto's gestures are driven by libinput and work on any multi-touch touchpad.
They are not configurable yet — the mappings below are fixed.

## Three-finger swipe

A three-finger swipe does one of two things, decided by direction:

| Direction | Effect |
|-----------|--------|
| Horizontal (left / right) | Switch workspaces |
| Vertical (up / down) | Open / close [Exposé](expose-and-switcher.md) |

Otto does not commit to either the moment you touch down. It accumulates the
movement and waits until you have travelled about 5 pixels in one direction,
then picks whichever axis has moved further. This means a slightly diagonal
swipe still does what you meant, and small stray movements never trigger
anything.

Once the direction is decided, it holds for the rest of the gesture — you cannot
turn a workspace swipe into an exposé by changing direction mid-way.

### Workspace switching

The workspaces track your fingers continuously: the whole desktop slides
horizontally as you move, so you can see the next workspace coming and back out
by reversing.

When you lift your fingers, Otto snaps to the nearest workspace using the
**velocity** of the last part of the gesture, not just the final position. A
quick flick carries you to the next workspace even if you barely moved; a slow
drag that stops short springs back.

Swiping past the first or last workspace has no wraparound.

### Exposé

Swiping **up** opens the exposé grid; the windows scale down and spread out in
step with your fingers, so you can peek at the grid and abandon it by swiping
back down. Lifting your fingers snaps open or closed based on how far you got,
with a spring animation carrying the momentum.

Swiping **down** from the open state closes it again.

While exposé is up, the [workspace selector](workspaces.md) strip appears above
the grid, and the dock hides.

## Four-finger pinch

| Gesture | Effect |
|---------|--------|
| Pinch **out** (spread) | Show desktop — windows slide away to the edges |
| Pinch **in** (close) | Bring the windows back |

Like the swipes, this tracks your fingers rather than firing at a threshold, so
you can pinch part-way to peek at the desktop and release to snap back.

The four-finger pinch is ignored while a three-finger swipe is in progress, and
while exposé is open — the two modes do not stack.

## Two-finger scroll

Two-finger scrolling is ordinary scroll input forwarded to whatever is under
the pointer. Natural (reversed) scrolling is on by default and configurable, as
are speed and acceleration — see [Input](input.md).

## Tap and click

Tap-to-click is on by default:

| Fingers | Button |
|---------|--------|
| 1 | Left |
| 2 | Right |
| 3 | Middle |

Physical clicks use the `clickfinger` method by default — the same mapping,
using how many fingers rest on the pad rather than where you click. Switch to
`buttonareas` (bottom-right corner = right click) in the config if you prefer.

Tap-and-drag is enabled; drag lock is not. Otto also disables the touchpad while
you type by default. All of this is configurable — see [Input](input.md).

## Keyboard equivalents

Every gesture has a keyboard action, so gestures are never the only route:

| Gesture | Action |
|---------|--------|
| Three-finger swipe left/right | `Workspace` with an index |
| Three-finger swipe up | `ExposeShowAll` |
| Four-finger pinch out | `ExposeShowDesktop` |

See [Keyboard Shortcuts](keyboard-shortcuts.md).

## Touchscreen

Touch input is supported for ordinary interaction — tap, drag, and window
move/resize requests from clients. The gestures above are touchpad-only.

## Troubleshooting

**Gestures do nothing.** Confirm libinput sees your touchpad as a multi-touch
device: `libinput list-devices` should report gesture support. Gestures do not
work on the `--winit` backend, where the host compositor consumes them.

**Swipes feel over- or under-sensitive.** The 5-pixel direction threshold and
the momentum model are not configurable yet. `scroll_speed` and
`pointer_accel_speed` in `[input]` affect scrolling and pointer motion, not
gestures.

**A swipe switched workspaces when I meant exposé.** Start the swipe with a
more deliberate vertical movement — the axis with the larger accumulated delta
at the 5-pixel mark wins.
