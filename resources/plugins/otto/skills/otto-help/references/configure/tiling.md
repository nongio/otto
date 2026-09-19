# Tiling

A tiling workspace lays its windows out edge to edge in a tree of splits
instead of letting them float. The mode belongs to a workspace on an output, so
a tiled workspace and a floating one live a swipe apart.

## Exact commands

```sh
# Gaps between tiles, and around them, in logical pixels
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv tiling.inner_gap i 4
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv tiling.outer_gap i 0

# Drop the gaps when a workspace holds one window
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv tiling.smart_gaps b true

# Chrome on a tiled window: "normal", "minimal" or "none"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv tiling.decoration s "none"

# Animation: seconds, and overshoot. 0.0 snaps
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv tiling.layout_duration d 0.2
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv tiling.layout_bounce d 0.0
```

**Apply** is `live` (now) or `restart` (at the next login).

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `tiling.decoration` | enum | live | `minimal` | `normal`, `minimal`, `none` | `s "none"` | How much chrome a window keeps while it is tiled: the same titlebar it wears while it floats, a bar one text line high with its title and a close button, or no bar at all, with the focused tile marked by a hairline border. |
| `tiling.inner_gap` | int | live | `8` | 0 – 64, step 1 | `i 4` | Logical pixels left between two neighbouring tiles. |
| `tiling.outer_gap` | int | live | `8` | 0 – 64, step 1 | `i 0` | Logical pixels left between the tiles and the edge of the screen. |
| `tiling.smart_gaps` | bool | live | `true` | `true`, `false` | `b false` | A workspace holding a single tile leaves no gaps at all, so one window does not look inset for no reason. |
| `tiling.resize_step` | double | live | `0.05` | 0.01 – 0.5, step 0.01 | `d 0.1` | How much of a container one keyboard resize step moves, as a fraction of its width or height. |
| `tiling.layout_duration` | double | live | `0.3` | 0.0 – 2.0, step 0.05 | `d 0.2` | Seconds a layout change takes: a window joining or leaving the tree, a move, a swap, an equalise. Zero snaps. |
| `tiling.layout_bounce` | double | live | `0.0` | 0.0 – 1.0, step 0.05 | `d 0.15` | How far a layout change overshoots before it settles. Zero settles without overshoot. |
| `tiling.mode_duration` | double | live | `0.4` | 0.0 – 2.0, step 0.05 | `d 0.0` | Seconds the workspace takes to rearrange when tiling is switched on or off and every window flies to its cell. Zero snaps. |
| `tiling.mode_bounce` | double | live | `0.1` | 0.0 – 1.0, step 0.05 | `d 0.0` | How far that rearrangement overshoots before it settles. |

## Worth knowing

- **Nothing is bound by default.** Tiling needs `TilingToggle` and the focus,
  move, split and resize actions bound in `[keyboard_shortcuts]`.
  [shortcuts.md](shortcuts.md) has a complete i3-style block to start from,
  the four i3 bindings that need `otto-msg` because they have no named action
  yet, and the i3 habits that do not transfer.
- **Decoration** is how much chrome a tiled window keeps: `normal` is the same
  titlebar it wears while floating, `minimal` is a one-line bar with the title
  and a close button, `none` is no bar at all with the focused tile marked by a
  hairline border in the accent colour. A floating window in a tiling workspace
  keeps its full titlebar under all three.
- **Gaps are per workspace unless told otherwise.** `otto-msg gaps inner 0`
  closes them on the workspace in front of you and remembers it; `gaps inner 8
  all` sets the session default and forgets the per-workspace tweaks. The
  records live under `[workspaces.entries."<output>:<n>"]`.
- **Durations of 0 snap.** Windows land in their cells in one frame.
- `otto-msg` drives the tree from a script — `focus`, `move`, `split`,
  `layout`, `resize`, `floating`, `tiling`.

## Read next

- https://nongio.github.io/otto/tiling/
- https://nongio.github.io/otto/scripting/
