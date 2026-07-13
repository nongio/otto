# Expose mode

## What it is
Expose shows scaled previews of every visible window on the current workspace so they can be clicked or dragged between workspaces. It is triggered by `Workspaces::expose_show_all` (keyboard toggle or gesture) which drives both the layout calculation and the transition animation.

## Lifecycle
- Enter/exit: `expose_show_all_workspace` computes gesture state, then calls `expose_show_all_layout` to build/update the layout bin and `expose_show_all_animate` to drive the animation and visibility.
- Updates: `expose_update_if_needed` recalculates when windows change (map/unmap/move/drag/drop) but only when expose is visible.
- Visibility: The expose layer and overlay layers are kept hidden unless animation or `show_all` is active to avoid unnecessary drawing.

## Multi-output
- Expose is global, not per-output: `show_all` opening/closing drives every output's grid at once. `expose_show_all_layout_for(output_name, workspace_index)` computes one output's grid against that output's own `workspaces_layer` size and origin; `expose_show_all_layout` is a thin wrapper that resolves the *focused* output's name and calls it (used by the gesture/keyboard path which addresses a `workspace_index`, not an output).
- Only the focused output uses `workspace_index` (the model's current/gesture-target workspace); every other output always grids and animates its own `current_workspace` from its `OutputWorkspaces`, so a mid-gesture focused output doesn't drag secondary outputs' layout along with it. `expose_show_all_animate`'s `is_focused_output` / `animate_this` split encodes this: the focused output only animates when it's showing its current workspace (as before), secondary outputs always animate.
- Window geometry is rebased output-local before comparing to the bin: `space.element_geometry(window)` returns a position in the global (Space) layout, so each output subtracts its own `current_location()` before converting to physical pixels — otherwise every tile lands off-screen (this was the bug `fix(expose): run expose on the focused output with local coords` fixed for the focused output; `feat(expose): simultaneous per-output expose` extended it to every output).
- The workspace-selector strip (`Workspaces::workspace_selector_view`) is a single shared instance, not one per output. Both expose entry points (gesture start in `expose_show_all_workspace`, and the keyboard path in `expose_show_all`) reparent its layer into the *focused* output's `overlay_plane` before showing it, so the strip visibly follows the user to whichever screen they're on.
- Lookups that need the focused output (`Workspaces::focused_output()`, which takes the model's read lock) must be resolved *before* entering a `with_model` closure — nesting the two deadlocks the main thread once a writer contends for the lock. Layout/animation hoist `focused_output()`/`focused_output_workspaces()` calls to the top of the function for this reason.
- `Workspaces::focused_output()` resolves to the output most recently confirmed under the pointer (falling back to primary). It's kept current by every pointer-motion path — udev relative motion, udev absolute motion, winit, and the virtual-pointer harness path — not just one backend; a path that forgets to update it leaves expose/the selector opening on a stale output. Virtual-pointer motion is clamped to the combined output bounds (`Otto::clamp_coords`) like real input, so a synthesized move can't drift the focused output out of resolvable range.
- lay-rs scene hit-testing (`layers_engine.pointer_move`, driving hover state and dock/overlay-UI interaction, not only expose) is fed the pointer rebased to the focused output's own origin, because every output's scene subtree overlaps at (0,0) — see `specs/multi-output.md`. Without the rebasing, hover and clicks land on whichever output subtree happens to be topmost in the layer tree rather than the output the pointer is actually over.
- Debug lever: `echo ActionName > /tmp/otto-action` (polled once per frame in the udev backend) executes a builtin shortcut action as if its key were pressed — useful for driving expose (`ExposeShowAll`, `ExposeShowDesktop`) or workspace switches from a test harness, since virtual-keyboard input bypasses the libinput shortcut layer entirely. It requests a redraw afterward so the scheduled lay-rs transaction actually ticks, and resolves through the same common action handler used for real key presses, which warns (rather than panics) on an action that's backend-specific or unresolvable.

## Gesture direction detection
Three-finger swipe gestures use accumulated delta values to determine intent: when the gesture begins, both horizontal and vertical deltas are tracked without activating either workspace switching or expose mode. Once the accumulated movement exceeds a 20-pixel threshold in either direction, the compositor commits to that mode based on which axis has greater magnitude—horizontal motion activates workspace switching (`workspace_swipe_update`) while vertical motion triggers expose mode (`expose_update`). This delayed commitment prevents accidental mode activation from minor diagonal movements and ensures the gesture feels responsive once direction is clear. After direction is determined, all subsequent update events feed directly into the active mode (workspace or expose) without re-evaluation, and velocity samples are collected for workspace switching to enable smooth momentum-based snapping on gesture end.

## Window mirroring
- Each window is mirrored by a layer created in `WorkspaceView::map_window` (`window_selector_view.map_window` adds it to the expose container). The mirror follows the real window layer via `add_follower_node`, so content stays in sync.
- Mirrors are excluded from expose while a drag is in progress (`expose_dragging_window`) to avoid double-rendering the dragged item.
- When a window is minimized, it is excluded from the expose, its mirror is hidden (`minimize_window`), and restored on unminimize.

## Layout: natural flow
- `expose_show_all_layout` builds an input list of windows (skipping minimized and currently dragged windows) with their real geometry and title.
- `WindowSelectorView::update_windows` calls `natural_layout` (in `utils::natural_layout`) to pack the windows into the target rectangle (`LayoutRect`) using a flowing grid algorithm:
  - Windows keep aspect ratios; scaling is limited to 1.0 so previews never exceed real size.
  - Packing is deterministic: windows are sorted by protocol id before hashing, and a layout hash is cached to skip no-op recalculations.
  - Results are stored in `expose_bin` and mirrored into `WindowSelectorState.rects`, which drives both drawing and hit-testing.

## Animation and positioning
- `expose_show_all_animate` interpolates window layers from their on-screen bbox to the target rects in `expose_bin`, applying translation + scale; easing is Spring-based when `end_gesture` is true.
- The workspace selector, dock, and overlay opacity/positions are animated in tandem to slide the UI into place. When expose is open, the dock is hidden unless fullscreen requires otherwise.

## Drag and drop in expose
- Drag activation happens in `WindowSelectorView::try_activate_drag` after a small threshold; mirrors are moved to the drag overlay while keeping anchor/scale consistent.
- Drop targets come from the workspace selector previews; intersection with a drop layer sets `current_drop_target`.
- On drop:
  - If a target workspace is selected, `move_window_to_workspace` is called with the window’s last known position; expose is refreshed to rebuild layout.
  - If no target, the dragged mirror is restored to its original parent and ordering (`restore_layer_order_from_state`), and expose is refreshed to realign.
- Logging: drop events log the window id and target workspace to help debugging.

## Common entry points
- Toggle expose: `expose_show_all(delta, end_gesture)`
- Force a relayout while in expose: `expose_update_if_needed` / `expose_update_if_needed_workspace`
- Show desktop (push windows away): `expose_show_desktop`

## Tips for agents
- Wait for expose to finish initializing (`show_all` true and `expose_bin` populated) before asserting layout.
- Use semantic data (rects from `WindowSelectorState` or `expose_bin`) rather than pixel checks; fractional scaling can shift raster output.
- During drags, the dragged window is intentionally absent from the grid; expect a temporary gap until drop completes or is cancelled.
