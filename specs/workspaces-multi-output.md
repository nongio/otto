# Workspaces & Multi-Output

**Status:** draft  
**Related specs:** multi-output, plane-scanout

## Summary

Otto supports multiple workspaces across multiple outputs (physical monitors and virtual outputs used for screensharing). Each output maintains its own independent set of workspaces that can be added, removed, and navigated independently.

## Goals

- Each output has its own list of workspaces with independent navigation.
- Adding a workspace on one output does not add a workspace on other outputs.
- Removing a workspace on one output does not remove a workspace on other outputs.
- Swiping/scrolling on one output only scrolls that output's workspaces.
- Virtual outputs (PipeWire screenshare) behave identically to physical outputs for workspace management.
- Every output renders its own workspace selector (expose mode) showing that output's own workspaces; the selector is not a single instance that follows focus.
- Clicking a workspace preview in the selector navigates only that output.
- The "+" button in the workspace selector adds a workspace only to that output.
- The "×" remove button in the workspace selector removes a workspace only from that output.

## Non-Goals

- Drag-and-drop of workspaces between outputs.
- Synchronised workspace counts across outputs (outputs may have different numbers of workspaces).
- Per-output dock (it remains shared/global, attached to the primary output). The app switcher is also a single shared panel, but it migrates to the output it should appear on rather than being duplicated.

## Behavior

### Output Types

- **Primary output:** The first physical output mapped. Owns the shared dock and overlay layers, and hosts the app switcher whenever it is not following the pointer elsewhere.
- **Secondary physical outputs:** Additional monitors. Each gets its own workspace set.
- **Virtual outputs:** Outputs created for PipeWire screensharing. Identified by a virtual-output marker. Treated identically to secondary physical outputs for all workspace operations.

### Workspace Lifecycle

**Adding a workspace:**

- When the user clicks "+" on an output's workspace selector, a new workspace is created on that output only.
- The new workspace appears at the end of the output's workspace list.
- Other outputs are unaffected.
- An entry animation plays (the preview grows from zero width and slides in).

**Removing a workspace:**

- The "×" button is revealed when the pointer hovers a workspace preview, and stays visible while the pointer moves from the preview onto the button itself. It hides again once the pointer leaves both.
- No "×" is rendered for the current workspace, nor for a fullscreen workspace that still has windows — neither can be removed.
- When the user clicks "×" on a workspace preview, that workspace is removed from that output only.
- If the output has only one workspace, the remove action is ignored (minimum one workspace per output).
- A removal animation plays before the workspace is actually removed: the item's width collapses to zero on a spring and the preview is cropped against it — the preview keeps its size and opacity and is wiped away rather than faded or scaled. The remaining previews slide across to close the gap because the strip re-lays out against the shrinking width every frame.
- The crop is centred and keeps the strip's spacing: the preview's box is inset half a gap on each side of the item and takes its width from the item, and the preview (and its label) sit centred in that box, so equal amounts are cropped from the left and the right and a full gap remains on both sides of the shrinking sliver. The preview is fully cropped away once the item is narrower than one gap, and the last of the item's width closes the remaining space.
- Clipping is armed only for the collapse — at rest the remove button and its shadow overhang the preview box and must not be cut off — and the button is hidden outright when the collapse starts, for the same reason. The workspace is dropped from compositor state when the collapse finishes, so the row never jumps.
- The collapsing item stops being a pointer target as soon as the animation starts: it is shrinking under the cursor, so clicks on it must not switch to it, rename it, or start a second removal.
- Item widths are owned by one place (the selector's post-render hook), not by the render function, so a re-render triggered mid-animation — a window opening, a workspace switch, a drop-hover change, the rename caret blinking — cannot cut an enter or leave animation short, and a workspace whose removal is refused cannot be left collapsed to zero width.
- The removal is addressed by the workspace's stable index and resolved to a list position at the moment it is sent, not at the moment it is clicked: an add or remove elsewhere during the animation would otherwise shift the position and delete the wrong workspace.
- Windows on the removed workspace are moved to the current workspace of that output.
- If the removed workspace was the last in the list and was active, the current workspace index is clamped to the new last workspace.
- Other outputs are unaffected.

**Moving a window between workspaces:**

- The window is unmapped from every space that held it and mapped into the target workspace on its own output; its scene layer follows into that workspace's view.
- Both workspaces' exposé grids are re-laid out, the one it left and the one it landed on.
- The workspace model is rebuilt, so everything derived from it follows the move immediately: the selector previews' window counts, the app switcher's ordering, and the dock's app list. None of these may wait for an unrelated window to open or close before catching up.

**Fullscreen guard:** A workspace that is in fullscreen mode and still contains windows cannot be removed.

### Navigation

**Workspace switching (per-output):**

- When the user selects a workspace preview in the selector, only that output navigates to the selected workspace.
- The output's workspace layer scrolls to the target workspace using the output's own physical width and scale for offset calculation.
- Other outputs remain on their current workspace.

**Keyboard workspace switching:**

- Global keyboard shortcuts (e.g. Ctrl+Left/Right) switch the workspace on the focused output only.
- The focused output is determined by pointer location.

**Three-finger swipe gesture:**

- A horizontal swipe gesture scrolls only the output the pointer is on.
- Scroll offset is computed using that output's workspace count, physical width, and scale.
- Rubber-band resistance applies at the edges (before first workspace and after last workspace).
- On gesture end, the output snaps to the nearest workspace based on position and velocity.
- Other outputs are unaffected by the swipe.

**Scroll clamping:**

- When a global scroll (e.g. after a workspace removal or expose exit) is applied, each output is scrolled to its own `current_workspace` index, clamped to that output's workspace count.
- An output with fewer workspaces than another is never scrolled past its last workspace.

### Fullscreen (Per-Output)

- Fullscreening a window stays on the output the window lives on: a free (or newly created) workspace on that output only receives the window, and only that output scrolls to it. Other outputs neither animate nor change workspace.
- The fullscreen window is mapped into its own output's space and scene view only, so it renders — and direct-scans out — only on that output. The render-side fullscreen-scanout check is per-CRTC: each output only promotes a fullscreen workspace of its own.
- During the enter/leave transition the window layer is parked in its own output's overlay plane (not the primary-only dnd layer), so the animation plays on the right screen.
- Dock hide/show and layer-shell top/overlay fades on fullscreen enter/leave apply only when the fullscreened output is the primary — that is where the chrome lives.
- A fullscreen window on one output does not affect frame-callback throttling of windows on other outputs: the throttle classifier evaluates fullscreen and top-of-stack per output.
- Unfullscreen restores the window to its saved rect on the same output, switches only that output back, and removes the dedicated workspace from that output only (via the removal channel's named-output form).
- Maximize similarly accounts for per-output chrome: the usable area subtracts the dock height only on the primary output; maximized windows on other outputs use the full height.
- Maximize and tiling target the output whose space the window is actually mapped in, including an interactive virtual (remote/RDP) output. Resolving the target from geometry overlap alone — with virtual outputs excluded from that lookup — made a window maximized on a virtual output resolve to no output and fall back to the primary physical screen, so it jumped off the remote session. Non-interactive virtual outputs remain excluded everywhere: nothing is ever placed on them.

### App Switcher (Output Placement)

- The app switcher is a single panel, never duplicated: it is re-parented into the switcher plane of the one output that should show it.
- With `appswitcher.follow_cursor` (default `true`) it appears on the output under the pointer — the focused output, the same one the workspace selector and window placement use. With the option off it always appears on the primary output.
- The host output is resolved once, when the panel is about to be shown; a switcher already on screen never jumps to another output mid-cycle, however far the pointer travels while the modifier is held.
- The panel is laid out from its host output's own physical width and fractional scale, not from the shared model's (primary) screen dimensions, so it is sized correctly on a screen of a different resolution or scale. Every layout metric — panel size, icon slot size, padding, gap, corner radius, label font size — derives from those two numbers.
- A change to the host geometry re-renders the panel immediately, whether it came from the panel moving to another output or from that output's mode or scale changing while the panel sits on it. Recording new host metrics is what triggers the re-render, so the panel is never left laid out for the screen it used to be on.
- Only the host output pushes a switcher plane, counts the panel as a scanout occluder, and treats it as blocking fullscreen direct scanout; every other output is unaffected while the switcher is up.
- The panel still lists windows from every output (it mirrors the global z-index app list); selecting one focuses it on the output that owns it, which may be a different output from the one showing the switcher.
- Apps are listed front to back: the app owning the topmost window first, so the first alt-tab step lands on the app that was in use before the current one. Apps whose windows are all on a non-current workspace sort behind the apps on the workspace in front of the user. An app appears once, at the position of its frontmost window.
- Committing a switch (releasing the modifier) focuses the selected app, which raises its window and re-sorts the list behind it, so stepping again returns to the app just left.
- If the host output is unplugged while hosting the panel, the panel returns to the primary output rather than being left detached from the scene.

### Expose Mode (Show All Workspaces)

- When expose mode is activated (open or close), every output lays out and animates its own tile grid at the same time — a single global "show all" state drives every screen together, there is no per-output open/close.
- Each output grids and animates its own **current** workspace: the focused output keeps its usual workspace-indexed behavior (e.g. mid-swipe scroll interacts with layout as before), while every other output always shows its own current workspace regardless of what the focused output is doing.
- Every output renders its own workspace selector strip, parented into that output's own overlay, showing only that output's workspace previews. There is no single shared selector that reparents between outputs.
- Each output's selector previews are built from that output's own workspace views at that output's own physical mode size and fractional scale, so each preview shows live, per-output-sized content.
- Opening expose redraws its wallpaper mirrors, on the gesture and on the hotkey alike. The full-screen expose backdrop is a mirror of the workspace's wallpaper and of the wlr-layer-shell background a wallpaper client paints into, and a mirror shows its leader as of the last time the *mirror* was repainted — not as of now. Nothing repaints these while expose is closed: the leader marks its followers, but unhiding a subtree does not walk into it, so the flag never becomes a repaint. Without the refresh on open, a wallpaper changed while the overview was closed never reaches it and every later expose shows the wallpaper that was set when it was last open.
- Each output shows its own windows in the expose grid, laid out and positioned in that output's own local geometry.
- Each window's expose mirror layer is created once (a follower of its base layer) and must survive cross-output migration: moving a window to another output detaches it from the old view's bookkeeping without deleting the mirror scene node (`unmap_window_keep_mirror`), and the target view re-parents the same mirror. Deleting the node (plain `unmap_window`, reserved for window destruction) would leave the window's fixed mirror handle pointing at a freed node — its expose preview would render as an empty rectangle forever after.
- Entering expose (hotkey or gesture) synchronously demotes every direct-scanout-promoted window and re-imports its buffer: while promoted, a window's scene content is blanked and commits skip the scene import, so its expose mirror would otherwise draw an empty rectangle. On the render side, scanout departure detection compares each output's previous promoted set against its own new set — never the global union, which would make every output treat other outputs' promoted windows as departures each frame.
- Clicking a window preview in expose focuses that window and scrolls ONLY the output that owns it, to the workspace containing it (`raise_element`, `focus_app`, and `focus_app_with_window` resolve the owning output by searching every output's spaces, then switch via the per-output workspace path). Other outputs stay where they are.
- Clicking a workspace preview in a selector navigates that output. Pointer input is routed to the selector on the output under the cursor and hit-tested in that output's local coordinates (the pointer's global position minus that output's global origin, since output subtrees render at scene (0,0)). Workspace switching is applied through the focused output.
- The "+" and "×" buttons in a selector apply to the output whose selector was clicked, that output only.
- When expose mode is dismissed, each output returns to its own current workspace.

### Pointer Hit-Testing

- Pointer-driven scene hit-testing (not only in expose mode — this covers dock and other overlay-UI hover/interaction too) always uses output-local coordinates. See multi-output.md's Focused Output and Output-Local Rendering behavior for the general rule; this section covers the workspace/expose-specific consequences.
- Workspace-selector hit-testing resolves the output under the pointer first, then converts the pointer position into that output's own physical coordinate space (subtracting that output's global origin) before testing against that output's selector previews. Window previews are hit-tested in the same per-output local space.
- Because each output owns its own selector showing only its own workspaces, clicking a workspace preview navigates the output that owns the clicked selector and can never navigate a different output.

### Rendering

- Each output renders its own scene subtree independently; every output's subtree lives at scene coordinate (0,0) and overlaps every other output's, since each CRTC/output render pass only ever walks its own subtree. Scene coordinates are therefore output-local by construction — they carry no information about where the output sits relative to others.
- Outputs are arranged left-to-right only in the separate, global layout used by window management and input (the smithay `Space`): the first output sits at the global origin, and every output added after it is placed immediately to the right of the combined extent of the outputs already placed (or at its configured position, if one is set and doesn't overlap). There is no vertical stacking. See multi-output.md for the full layout, damage-generation, per-output lookup, and cursor-ownership contract.
- Workspace layers, expose layers, and workspace selector layers are all per-output sublayers, parented under that output's (0,0)-positioned container layer.

## Constraints & Edge Cases

- **Minimum one workspace:** Each output must always have at least one workspace. Remove is a no-op when only one remains.
- **One workspace operation is still lockstep (known limitation):** while the selector's add ("+") and remove ("×") act on a single output, window overflow — when a new window needs a free workspace and none exists — still adds the newly created workspace to all outputs at once. Fullscreen is fully per-output: entering fullscreen creates (or reuses) a free workspace on the window's own output only, switches only that output to it, and leaving fullscreen removes that workspace from that output only, via the removal channel's named-output form. Window moves between workspaces (fullscreen enter/leave, expose drag-drop, workspace removal re-homing) are scoped to the window's owning output: the window is mapped into that output's space and scene view only, never into other outputs' spaces.
- **Stale scroll offsets:** After workspace removal, the scroll position may reference a workspace that no longer exists. The scroll must be clamped to valid bounds before any animation.
- **Workspace counter is global:** Workspace indices (used for view identification and the model) are assigned from a shared counter. This means workspace index values are unique across all outputs but non-contiguous within a single output.
- **Model mirrors the focused output, falling back to primary:** The shared `WorkspacesModel` (used by observers like the dock, app switcher, and expose layout/animation) reflects the workspace list and current index of the focused output (see multi-output.md), or the primary output whenever no output has been focused yet. Non-focused outputs do not update the shared model directly; only the model's source output changes. The per-output workspace selectors do not read the shared model — each is fed directly from its own output's workspace views.
- **The dock is shared and primary-only regardless of focus:** it is attached to the primary output layer and responds to the shared model, but — unlike expose, the workspace selector and the app switcher — it does not follow focus to a secondary output; it is not duplicated on secondary outputs and stays visible only on the primary output's screen.
- **Layer engine pointer overlap is now the normal case, not an edge case:** every output's scene subtree lives at (0,0) and overlaps every other output's by design (see multi-output.md), so pointer hit-testing through the layer engine with a global root would hit layers belonging to whichever output's subtree happens to be on top, regardless of where the pointer actually is on screen. All pointer interactions (expose mode and otherwise) must resolve the target output first (from the pointer's global/logical position against each output's global geometry) and then hit-test only that output's subtree — never hit-test the shared scene root directly for input routing.

## Rationale

- **Per-output workspaces** allow each monitor to serve a different purpose (e.g. code on one, browser on another) without forcing them to stay in sync.
- **Left-to-right output layout** was chosen as the simplest arrangement covering the common case (monitors side by side), for the *global* (Space/input) layout only; each output still renders into its own framebuffer from its own (0,0)-positioned, overlapping scene subtree, so global position never affects scene-graph coordinates (see multi-output.md). Because every output's subtree sits at the same scene position by design, pointer hit-testing must resolve which output owns an event using the global layout (never the scene graph) before hit-testing that output's subtree specifically.
- **Single removal channel carrying an output target** ensures every output's selector routes removal requests through one handler while still keeping per-output scope: a request names the output it should act on, and a request with no output name means "remove in lockstep on all outputs" (now unused by regular flows; the fullscreen-close path names the window's output). This keeps a single receiver instead of one per selector while preserving the per-output-vs-lockstep distinction.
- **Model mirrors the focused output (falling back to primary)** rather than being extended to a genuinely per-output model, because most observers (dock, app switcher) only ever need primary's state, while a smaller set of observers (expose layout, the workspace selector) need to reflect whichever output the user is actually on. Layout/animation lookups against the model resolve the focused output's name *outside* the model's own read-lock closure before using it — resolving focus requires taking that same lock, and nesting the two deadlocks the main thread once a writer contends for it.
- **The workspace-selector strip is now per-output** rather than a single instance that reparents to the focused output. An earlier design used one shared selector reparented into the focused output's overlay; that could only ever show one output's workspaces at a time, so a multi-monitor expose left secondary screens without a usable selector. Giving each output its own selector — parented to that output's overlay, sized to that output's mode/scale, and fed from that output's own workspace views — makes every screen show and edit its own workspaces during expose. Routing pointer input to the selector on the output under the cursor (hit-tested in that output's local coordinates) is what keeps clicks acting on the right output.
- **Expose opens and closes on every output at once, each animating its own current workspace,** rather than only the focused output entering expose, so that the mental model of expose ("see everything") holds across a multi-monitor desktop instead of leaving secondary screens showing stale, un-exposed content while the focused screen is mid-transition.

## Open Questions

- Should removing a workspace on a secondary output also remove windows from it, or should windows be migrated to the same workspace index on the primary output?
- Should keyboard shortcuts for "move window to workspace N" be scoped to the focused output, or should they always target the primary?
- Should there be a maximum number of workspaces per output?
- When a virtual output is destroyed (screenshare ends), what happens to its workspaces and any windows on them?
