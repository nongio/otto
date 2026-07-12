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
- The workspace selector (expose mode) is per-output: each output shows its own workspace previews.
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

- When expose mode is activated, all outputs enter expose simultaneously.
- Each output shows miniature previews of its own workspaces in its workspace selector.
- Each output shows its own windows in the expose grid.
- Clicking a window preview in expose focuses that window on the output it belongs to.
- Clicking a workspace preview in expose navigates only that output.
- The "+" and "×" buttons in expose function per-output (as described above).
- When expose mode is dismissed, each output returns to its own current workspace.

### Pointer Hit-Testing

- Pointer events in expose mode use output-local coordinates for hit-testing.
- The pointer position is converted to the target output's physical coordinate space before testing against workspace selector previews and window previews.
- Clicking on a workspace preview on output A must never cause navigation on output B.

### Rendering

- Each output renders its own scene subtree independently.
- Output layers are arranged left-to-right in the shared scene: the first output sits at the scene origin, and every output added after it is placed immediately to the right of the combined extent of the outputs already placed. There is no vertical stacking. See multi-output.md for the full layout, per-output plane-buffer coordinate, and cursor-ownership contract.
- Workspace layers, expose layers, and workspace selector layers are all per-output sublayers.

## Constraints & Edge Cases

- **Minimum one workspace:** Each output must always have at least one workspace. Remove is a no-op when only one remains.
- **Stale scroll offsets:** After workspace removal, the scroll position may reference a workspace that no longer exists. The scroll must be clamped to valid bounds before any animation.
- **Workspace counter is global:** Workspace indices (used for view identification and the model) are assigned from a shared counter. This means workspace index values are unique across all outputs but non-contiguous within a single output.
- **Model mirrors primary only:** The shared `WorkspacesModel` (used by observers like the dock and app switcher) reflects only the primary output's workspace list and current index. Secondary outputs do not update the shared model directly.
- **Dock and app switcher are shared:** These are attached to the primary output layer and respond to the shared model. They are not duplicated on secondary outputs.
- **Layer engine pointer overlap:** Outputs are normally laid out non-overlapping (left-to-right), but a virtual output left at its default position, or any output explicitly configured to coincide with another, does overlap another output's region in scene-graph space. Pointer hit-testing through the layer engine with a global root may then hit layers belonging to the wrong output. All pointer interactions in expose mode must use output-scoped hit-testing regardless of layout.

## Rationale

- **Per-output workspaces** allow each monitor to serve a different purpose (e.g. code on one, browser on another) without forcing them to stay in sync.
- **Left-to-right output layout** was chosen as the simplest arrangement covering the common case (monitors side by side); each output still renders into its own framebuffer, so a nonzero scene position only affects where the output's layers sit in shared scene space, not how many framebuffers exist. Because outputs are no longer guaranteed to sit at the same scene position, pointer hit-testing and plane-buffer rendering must each independently account for an output's own placement (see multi-output.md) rather than assuming a shared origin.
- **Shared removal channel** ensures all workspace selector instances (primary and secondary) route removal requests through a single handler, avoiding orphaned receivers on secondary selectors.
- **Model mirrors primary only** because the dock, app switcher, and other observers only need to know about the primary output's state. Extending the model to be per-output would add complexity with no current consumer.

## Open Questions

- Should removing a workspace on a secondary output also remove windows from it, or should windows be migrated to the same workspace index on the primary output?
- Should keyboard shortcuts for "move window to workspace N" be scoped to the focused output, or should they always target the primary?
- Should there be a maximum number of workspaces per output?
- When a virtual output is destroyed (screenshare ends), what happens to its workspaces and any windows on them?
