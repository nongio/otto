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
- The workspace selector (expose mode) always shows the workspaces of the currently focused output (see multi-output.md), and follows focus from output to output.
- Clicking a workspace preview in the selector navigates only that output.
- The "+" button in the workspace selector adds a workspace only to that output.
- The "×" remove button in the workspace selector removes a workspace only from that output.

## Non-Goals

- Drag-and-drop of workspaces between outputs.
- Synchronised workspace counts across outputs (outputs may have different numbers of workspaces).
- Per-output dock or app switcher (these remain shared/global, attached to the primary output).

## Behavior

### Output Types

- **Primary output:** The first physical output mapped. Owns the shared dock, app switcher, and overlay layers.
- **Secondary physical outputs:** Additional monitors. Each gets its own workspace set.
- **Virtual outputs:** Outputs created for PipeWire screensharing. Identified by a virtual-output marker. Treated identically to secondary physical outputs for all workspace operations.

### Workspace Lifecycle

**Adding a workspace:**

- When the user clicks "+" on an output's workspace selector, a new workspace is created on that output only.
- The new workspace appears at the end of the output's workspace list.
- Other outputs are unaffected.
- An entry animation plays (the preview grows from zero width and slides in).

**Removing a workspace:**

- When the user clicks "×" on a workspace preview, that workspace is removed from that output only.
- If the output has only one workspace, the remove action is ignored (minimum one workspace per output).
- A removal animation plays (the preview shrinks to zero width) before the workspace is actually removed.
- Windows on the removed workspace are moved to the current workspace of that output.
- If the removed workspace was the last in the list and was active, the current workspace index is clamped to the new last workspace.
- Other outputs are unaffected.

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

### Expose Mode (Show All Workspaces)

- When expose mode is activated (open or close), every output lays out and animates its own tile grid at the same time — a single global "show all" state drives every screen together, there is no per-output open/close.
- Each output grids and animates its own **current** workspace: the focused output keeps its usual workspace-indexed behavior (e.g. mid-swipe scroll interacts with layout as before), while every other output always shows its own current workspace regardless of what the focused output is doing.
- The workspace selector strip is a single shared instance, not one per output (see Constraints). Opening expose reparents it into the focused output's overlay, so it visibly appears on the screen the user is actually on, showing only that output's workspace previews.
- Each output shows its own windows in the expose grid, laid out and positioned in that output's own local geometry.
- Clicking a window preview in expose focuses that window on the output it belongs to.
- Clicking a workspace preview in expose navigates the focused output (the only output the shared selector can display at a given moment).
- The "+" and "×" buttons in expose apply to the focused output, since the shared selector only ever displays that output's workspaces.
- When expose mode is dismissed, each output returns to its own current workspace.

### Pointer Hit-Testing

- Pointer-driven scene hit-testing (not only in expose mode — this covers dock and other overlay-UI hover/interaction too) always uses output-local coordinates, rebased against the focused output specifically. See multi-output.md's Focused Output and Output-Local Rendering behavior for the general rule; this section covers the workspace/expose-specific consequences.
- The pointer position is converted to the focused output's physical coordinate space before testing against workspace selector previews and window previews.
- Clicking on a workspace preview must never cause navigation on an output other than the focused one — since the selector is a single shared instance that only ever shows the focused output's workspaces, this holds by construction rather than needing a separate per-click output check.

### Rendering

- Each output renders its own scene subtree independently; every output's subtree lives at scene coordinate (0,0) and overlaps every other output's, since each CRTC/output render pass only ever walks its own subtree. Scene coordinates are therefore output-local by construction — they carry no information about where the output sits relative to others.
- Outputs are arranged left-to-right only in the separate, global layout used by window management and input (the smithay `Space`): the first output sits at the global origin, and every output added after it is placed immediately to the right of the combined extent of the outputs already placed (or at its configured position, if one is set and doesn't overlap). There is no vertical stacking. See multi-output.md for the full layout, damage-generation, per-output lookup, and cursor-ownership contract.
- Workspace layers, expose layers, and workspace selector layers are all per-output sublayers, parented under that output's (0,0)-positioned container layer.

## Constraints & Edge Cases

- **Minimum one workspace:** Each output must always have at least one workspace. Remove is a no-op when only one remains.
- **Stale scroll offsets:** After workspace removal, the scroll position may reference a workspace that no longer exists. The scroll must be clamped to valid bounds before any animation.
- **Workspace counter is global:** Workspace indices (used for view identification and the model) are assigned from a shared counter. This means workspace index values are unique across all outputs but non-contiguous within a single output.
- **Model mirrors the focused output, falling back to primary:** The shared `WorkspacesModel` (used by observers like the dock, app switcher, expose layout/animation, and the workspace selector) reflects the workspace list and current index of the focused output (see multi-output.md), or the primary output whenever no output has been focused yet. Non-focused outputs do not update the shared model directly; only the model's source output changes.
- **Dock and app switcher are shared and primary-only regardless of focus:** These are attached to the primary output layer and respond to the shared model, but — unlike expose and the workspace selector — they do not follow focus to a secondary output; they are not duplicated on secondary outputs and stay visible only on the primary output's screen.
- **Layer engine pointer overlap is now the normal case, not an edge case:** every output's scene subtree lives at (0,0) and overlaps every other output's by design (see multi-output.md), so pointer hit-testing through the layer engine with a global root would hit layers belonging to whichever output's subtree happens to be on top, regardless of where the pointer actually is on screen. All pointer interactions (expose mode and otherwise) must resolve the target output first (from the pointer's global/logical position against each output's global geometry) and then hit-test only that output's subtree — never hit-test the shared scene root directly for input routing.

## Rationale

- **Per-output workspaces** allow each monitor to serve a different purpose (e.g. code on one, browser on another) without forcing them to stay in sync.
- **Left-to-right output layout** was chosen as the simplest arrangement covering the common case (monitors side by side), for the *global* (Space/input) layout only; each output still renders into its own framebuffer from its own (0,0)-positioned, overlapping scene subtree, so global position never affects scene-graph coordinates (see multi-output.md). Because every output's subtree sits at the same scene position by design, pointer hit-testing must resolve which output owns an event using the global layout (never the scene graph) before hit-testing that output's subtree specifically.
- **Shared removal channel** ensures all workspace selector instances (primary and secondary) route removal requests through a single handler, avoiding orphaned receivers on secondary selectors.
- **Model mirrors the focused output (falling back to primary)** rather than being extended to a genuinely per-output model, because most observers (dock, app switcher) only ever need primary's state, while a smaller set of observers (expose layout, the workspace selector) need to reflect whichever output the user is actually on. Layout/animation lookups against the model resolve the focused output's name *outside* the model's own read-lock closure before using it — resolving focus requires taking that same lock, and nesting the two deadlocks the main thread once a writer contends for it.
- **The workspace-selector strip is a shared singleton that reparents to the focused output** rather than one instance per output, avoiding duplicated selector state and hit-testing machinery; since only one selector exists, following focus by re-parenting its layer into whichever output's overlay is focused is enough to make it appear on the correct screen without needing per-output show/hide bookkeeping.
- **Expose opens and closes on every output at once, each animating its own current workspace,** rather than only the focused output entering expose, so that the mental model of expose ("see everything") holds across a multi-monitor desktop instead of leaving secondary screens showing stale, un-exposed content while the focused screen is mid-transition.

## Open Questions

- Should removing a workspace on a secondary output also remove windows from it, or should windows be migrated to the same workspace index on the primary output?
- Should keyboard shortcuts for "move window to workspace N" be scoped to the focused output, or should they always target the primary?
- Should there be a maximum number of workspaces per output?
- When a virtual output is destroyed (screenshare ends), what happens to its workspaces and any windows on them?
