# Exposé

Exposé shows a scaled preview of every visible window on the current workspace,
laid out on a grid, so windows can be picked or dragged onto another workspace.
It is triggered by a keyboard shortcut or a three-finger vertical swipe.

## The core trick: mirrors, not moved windows

Exposé does not move the real windows. Each window gets a **mirror layer** —
a second node in the scene graph that follows the real window's layer via
`add_follower_node` — and it is the mirrors that are laid out on the grid.

The value of this is that a window keeps rendering into its normal place in the
scene while a scaled copy of it appears in the grid. Live video keeps playing in
the preview; the window's own state is untouched, so leaving exposé needs no
restore step.

Mirrors are created in `WorkspaceView::map_window`, which hands them to
`window_selector_view.map_window`. A window being dragged is excluded
(`expose_dragging_window`) so it isn't drawn twice, and a minimized window's
mirror is hidden by `minimize_window` and restored on unminimize.

**The mirror-freeze trap.** `lay-rs` propagates `NEEDS_PAINT` from a *leader
node itself* to its followers — never from the leader's descendants. A client
commit repaints the surface layer deep inside the window's subtree, so the
mirror is never flagged and keeps drawing its last recorded picture. With
`workspaces_layer` hidden during exposé, nothing else damages it either, and
the previews freeze — a playing video looks stuck. `update_window_view`
therefore calls `add_damage` on the window's *base* layer while exposé is up,
which does mark the followers.

This is covered by the `expose_preview_repaints_on_client_commit` headless test
(`tests/headless_basic.rs`), which asserts `subtree_damage` on the mirror node.
Asserting whole-scene damage is not specific enough to catch the regression.

## The wallpaper is painted twice, on purpose

`WindowSelectorView` mirrors the workspace background (`window_selector_background`)
and the wlr-layer-shell background (`layer_shell_bg_expose_mirror`) into its own
subtree, below the previews — even though the background plane underneath is
already showing the same wallpaper. Both are needed:

- **Composite path.** `workspaces_layer` — which owns the real background — is
  hidden while exposé is up, so without the mirrors the whole scene renders
  exposé over nothing.
- **Plane path.** The decoration inside a preview carries
  `BlendMode::BackgroundBlur`, and a background blur blurs *what the destination
  canvas already holds*. Exposé renders into its own buffer (its own KMS plane),
  where the wallpaper of the plane below does not exist. The cross-plane
  external backdrop does not rescue it either: lay-rs seeds that backdrop only
  for layers whose own blend mode is `BackgroundBlur`, and a preview is a mirror
  — `Layer::as_content()` re-renders the leader's subtree with the backdrop
  parameter set to `None`. So the previews blur the empty exposé buffer and
  every titlebar comes out the same flat grey, no matter what is behind it.

Painting the wallpaper into the exposé subtree fixes both, and fixes them with
the *right* pixels: the blur reads the canvas under the mirror's own transform,
so a preview samples the wallpaper where the preview sits, not where the real
window sits (which is what seeding the external backdrop by the leader's global
bounds would have given).

The cost is one extra full-screen wallpaper draw per exposé frame. Only the
on-screen workspace pays it — the other workspaces' selector roots are laid out
side by side beyond the output edge and are clipped away.

The same limitation still applies to a preview dragged onto the workspace strip:
it is reparented into the drag overlay, in the *overlay* plane, whose buffer has
no wallpaper either.

Outside exposé the same class of bug hit the ordinary server-side titlebar —
its blur is a real `BackgroundBlur` layer, but the windows plane was never
given a backdrop, so it blurred an empty buffer too. That one is fixed the
other way, with the external backdrop: `udev::backdrop` hands the middle plane
the background-only stage of the composite and the titlebar opts into
`blur_include_content`, so it blurs the wallpaper *and* the windows painted
below it in the same pass. See [`specs/plane-scanout.md`](../../specs/plane-scanout.md).

## Lifecycle

- **Enter / exit** — `Workspaces::expose_show_all(delta, end_gesture)` is the
  public entry point. It routes to `expose_show_all_workspace`, which
  accumulates gesture state and decides the target, then calls
  `expose_show_all_layout` to build the grid and
  `expose_show_all_update` (mid-gesture) or `expose_show_all_end`
  (on release) to drive it. Both land in `expose_show_all_apply`, which does
  the actual layer work.
- **Updates** — `expose_update_if_needed` recalculates when windows change
  (map, unmap, move, drag, drop), but only while exposé is visible.
- **Visibility** — the exposé layer and the overlay layers stay hidden unless
  an animation is running or `show_all` is set, so they cost nothing when
  closed.
- **Chrome ownership** — while exposé is open *or* transitioning
  (`get_show_all() || is_expose_transitioning()`), exposé alone owns the dock
  position and the `layer_shell_top` / `layer_shell_overlay` opacity, and
  restores them from its close animation's `on_finish`. The workspace-switch
  paths — `workspace_swipe_update`, `workspace_swipe_end` →
  `set_workspace_for_output`, `scroll_to_workspace_index` — drive that same
  chrome from the target workspace's fullscreen state, so they must skip it
  under that condition. Otherwise starting a workspace swipe (or switching by
  key) while exposé is up fades the top bar back onto the screen.

## Layout: natural flow

`expose_show_all_layout` builds a list of windows with their real geometry and
title, skipping minimized and currently-dragged ones.
`WindowSelectorView::update_windows` then calls `natural_layout`
(`src/utils/natural_layout.rs`) to pack them into the target rectangle:

- Aspect ratios are preserved, and scaling is capped at 1.0 so a preview never
  exceeds the window's real size.
- Packing is deterministic — windows are sorted by protocol id before hashing —
  and a layout hash is cached so a no-op recalculation costs nothing.
- Results land in `expose_bin` and are mirrored into `WindowSelectorState.rects`,
  which drives both drawing and hit-testing.

## Hover selection

`WindowSelectorState.current_selection` is the index of the hovered preview. It
drives the accent highlight and the title label drawn by `view_window_selector`.

Keeping it across a re-layout is fiddlier than it sounds, because re-layouts
are **not** rare: a window's geometry comes from its surface-tree bbox, so an
ordinary client commit invalidates the layout hash and rebuilds the grid under
a stationary pointer. `update_windows` rebuilds `rects` from scratch, so it
carries the hovered window over *by id*, keeping it selected as long as the
last recorded cursor position still falls inside that window's (possibly moved)
preview.

For the same reason `expose_update_if_needed` re-shows the selection overlay
(`show_selection_overlays`) after scheduling the re-layout animation:
`expose_show_all_apply` blanks that overlay to zero opacity for the length of
the open animation and only restores it in the animation's `on_finish`, which
would otherwise blink the highlight and label out on every re-layout while
exposé is already open.

Covered by the `expose_selection_survives_client_commit` headless test.

## Animation and positioning

`expose_show_all_apply` interpolates window layers from their on-screen bbox to
the target rects in `expose_bin`, applying translation plus scale. Easing is
Spring-based when `end_gesture` is true. The workspace selector, the dock and
overlay opacity animate in tandem so the whole UI slides into place; the dock
hides while exposé is open unless fullscreen requires otherwise. Popups are
hidden for the whole exposé lifetime and restored in the close animation's
`on_finish`.

## Gesture direction detection

A three-finger swipe can mean either "switch workspace" or "exposé", and the
compositor cannot know which until the finger has moved.

So it commits late. Both horizontal and vertical deltas accumulate without
activating either mode. Once accumulated movement passes **20 px** in either
direction, the axis with the greater magnitude wins: horizontal goes to
`workspace_swipe_update`, vertical to `expose_update`. After that, every
subsequent event feeds the chosen mode directly, with no re-evaluation — so a
diagonal drift mid-gesture cannot flip modes. Velocity samples are collected
along the way for workspace switching, which uses them for momentum-based
snapping on release.

## Drag and drop

- `WindowSelectorView::try_activate_drag` starts a drag after a small
  threshold; the mirror moves to the drag overlay, keeping its anchor and scale
  consistent so it doesn't jump.
- Drop targets come from the workspace selector's previews; intersecting a drop
  layer sets `current_drop_target`. A drop target is keyed by the workspace
  *view* index (a stable id), not by its position in the strip — positions are
  resolved through `workspace_position_by_view_index`, and the hover highlight
  matches on the view index too. The two only ran in lockstep before workspaces
  could be added and removed from the strip.
- On drop with a target: `move_window_to_workspace` is called with the window's
  last known position. It drops the cached grid of the source and destination
  workspaces first (`invalidate_layout`), because the drag already re-laid the
  source grid out without the dragged window when it was picked up — the
  cached hash equals the post-move one, so without the invalidation the drop
  applies nothing and the grid keeps the layout it was dropped on until an
  unrelated client commit moves the hash again. It then re-lays out both grids;
  the drop path only has to put the selection overlay back
  (`show_selection_overlays`), which the drag hid. Moving a window also drops
  it from the source grid's selector map, keeping its mirror alive for the
  destination view (`unmap_window_keep_mirror`).
- On drop with no target: the mirror is restored to its original parent and
  ordering (`restore_layer_order_from_state`) and exposé refreshes to realign.
- The selection overlay is hidden for the whole drag and revealed again by
  `show_selection_overlays`, which **re-renders it before raising the opacity**.
  Its layer keeps the last picture it was rasterized with, and content that
  changes while it is invisible is never re-recorded: the highlight left on the
  preview that was then dragged away came back with the overlay, sitting on
  empty grid, even though the state had dropped the selection long before.
  Reproduced and confirmed on hardware (drag a preview onto another workspace's
  thumbnail and screenshot the frame after the release).
- Drop events log the window id and target workspace.

## Multi-output

Exposé is **global, not per-output**: opening or closing it drives every
output's grid at once.

- `expose_show_all_layout_for(output_name, workspace_index)` computes one
  output's grid against that output's own `workspaces_layer` size and origin.
  `expose_show_all_layout` is a thin wrapper resolving the *focused* output's
  name — used by the gesture/keyboard path, which addresses a `workspace_index`
  rather than an output.
- Only the focused output uses `workspace_index`. Every other output grids and
  animates its own `current_workspace` from its `OutputWorkspaces`, so a
  mid-gesture focused output does not drag secondary outputs' layouts along with
  it. The `is_focused_output` / `animate_this` split encodes this: the focused
  output only animates when it is showing its current workspace; secondary
  outputs always animate.
- **Window geometry must be rebased output-local before comparing to the bin.**
  `space.element_geometry(window)` returns a position in the global Space
  layout, so each output subtracts its own `current_location()` before
  converting to physical pixels. Without this, every tile lands off-screen.
- The workspace-selector strip (`Workspaces::workspace_selector_view`) is a
  single shared instance, not one per output. Both entry points — gesture start
  in `expose_show_all_workspace` and the keyboard path in `expose_show_all` —
  reparent its layer into the *focused* output's `overlay_plane` before showing
  it, so the strip follows the user to whichever screen they are on.

### Two multi-output footguns

**Deadlock.** `Workspaces::focused_output()` takes the model's read lock, so it
must be resolved *before* entering a `with_model` closure. Nesting the two
deadlocks the main thread as soon as a writer contends for the lock. Layout and
animation hoist `focused_output()` / `focused_output_workspaces()` to the top of
the function for exactly this reason.

**Stale focused output.** `focused_output()` resolves to the output most
recently confirmed under the pointer, falling back to primary. It is kept
current by *every* pointer-motion path — udev relative motion, udev absolute
motion, winit, and the virtual-pointer harness path. A path that forgets to
update it leaves exposé and the selector opening on the wrong screen.
Virtual-pointer motion is clamped to the combined output bounds
(`Otto::clamp_coords`) like real input, so a synthesized move cannot drift the
focused output out of resolvable range.

**Hit-testing.** `layers_engine.pointer_move` — which drives hover state and all
dock/overlay-UI interaction, not only exposé — is fed the pointer rebased to the
focused output's own origin, because every output's scene subtree overlaps at
(0, 0). See [`specs/multi-output.md`](../../specs/multi-output.md). Without the
rebasing, hover and clicks land on whichever output subtree happens to be
topmost in the layer tree, rather than the one the pointer is actually over.

## Entry points

| Action | Call |
|--------|------|
| Toggle exposé | `expose_show_all(delta, end_gesture)` |
| Force a relayout while open | `expose_update_if_needed` / `expose_update_if_needed_workspace` |
| Show desktop (push windows away) | `expose_show_desktop(delta, end_gesture)` |

Show desktop reuses the exposé machinery: it hides `workspaces_layer`, shows
`expose_layer`, and animates the same mirror layers off the screen edges. The
render paths must therefore drop the real windows plane for it too — they gate
on `Workspaces::mirrors_active()` (exposé, its transition, show desktop, or its
transition), not on `get_show_all()` alone. Testing only the exposé flags left
the untouched windows compositing on top of the mirrors sliding away, so the
gesture rendered as a no-op.

Its completion hook — the one that hides the mirrors and hands the screen back
to the real windows — rides the first mirror's own position animation. Hanging
it off a no-op property change instead (setting a layer to the opacity it
already has) finished on the spot, so dismissing show desktop restored the
windows in a single frame while the mirrors were still flying back. Anything
that dismisses it (clicking a window, `ExposeShowDesktop`, a three-finger
swipe) goes through `expose_show_desktop(-2.0, true)` and gets that animation.

## Keyboard focus

Opening exposé clears keyboard focus (`Otto::enter_expose_focus`, called
alongside `dismiss_all_popups` / `demote_all_scanout_windows` at every open
site: the action handler and both gesture handlers). Closing restores it —
`close_expose_show_all_and_focus_top` for the click/keyboard path,
`expose_end_with_velocity_and_focus_top` for the gesture, both focusing the
hovered preview or the workspace's top window.

Two things ride on this. Keys pressed while the previews are up no longer land
in whatever window happened to be in front. And it is the **only** signal a
client gets that exposé opened: `dismiss_all_popups` reaches popups, but an app
that draws transient chrome into a *subsurface* — the file browser's quick view
panel — is out of its reach, and takes the panel down on `wl_keyboard.leave`
instead.

## Testing notes

- Wait for exposé to finish initializing — `show_all` true *and* `expose_bin`
  populated — before asserting on layout.
- Assert on semantic data (`WindowSelectorState.rects`, `expose_bin`) rather
  than pixels; fractional scaling shifts raster output.
- The background mirror is covered by
  `expose_subtree_paints_the_wallpaper_below_the_previews`, which asserts the
  layer exists under `window_selector_root` *and* is ordered before the previews
  container. A pixel assertion would not catch the regression: the background
  plane below keeps showing the wallpaper, so only the blur inside the previews
  changes.
- During a drag the dragged window is intentionally absent from the grid.
  Expect a gap until the drop completes or is cancelled.
- `tests/workspace_selector.rs` covers the preview layout end to end against a
  headless compositor: opening a window while exposé is up re-lays out the
  grid, closing it restores the previous layout, and a workspace move updates
  the preview counts on both workspaces. `tests/app_switcher.rs` covers
  click-to-raise, alt-tab and the switcher's ordering. Run them with
  `cargo test --features headless --test workspace_selector` (and
  `--test app_switcher`).
- **Debug lever:** `echo ActionName > $OTTO_ACTION_FILE` (default
  `/tmp/otto-action`, polled once per frame by both backends — see
  [the debug action hook](debug-action-hook.md)) runs a builtin shortcut
  action as if its key had been pressed — useful for driving `ExposeShowAll`,
  `ExposeShowDesktop` or workspace switches from a harness, since
  virtual-keyboard input bypasses the libinput shortcut layer entirely. It
  requests a redraw afterwards so the scheduled `lay-rs` transaction actually
  ticks, and resolves through the same action handler as a real key press —
  warning, rather than panicking, on an action that is backend-specific or
  unresolvable.
