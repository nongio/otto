# Multi-Output Rendering & Scheduling

**Status:** draft  
**Related specs:** workspaces-multi-output.md, plane-scanout.md, pointer-input-focus.md

## Summary

Otto can drive more than one output (physical monitor or virtual/screenshare output) at once. This spec covers the infrastructure shared by all outputs regardless of what they display: how outputs are laid out relative to each other in the shared scene, how per-output KMS plane buffers stay correctly positioned despite that shared layout, how the render loop schedules and wakes each output independently, and which output(s) show the hardware cursor. Workspace lifecycle/navigation is covered by `workspaces-multi-output.md`; the KMS plane-decomposition mechanism itself is covered by `plane-scanout.md`.

## Goals

- Multiple outputs can be active at once, each showing its own content, without one output's position affecting what another output visually displays.
- A newly connected output is placed into the shared layout automatically, with no visible content shift on outputs already active.
- Input events (pointer motion/click, keyboard) reliably wake rendering on whichever output needs it, even when other outputs are idle or busy.
- The hardware cursor is visible on the output(s) the pointer geometrically occupies, and disappears promptly from an output the pointer has left.

## Non-Goals

- Vertical or grid output arrangement — only a single horizontal row is supported.
- User-configurable placement for physical (non-virtual) outputs (see Constraints).
- A first-class output-mirroring feature (showing the same content on two outputs by design).
- Per-output dock, topbar, or app switcher — these remain primary-output-only (see `workspaces-multi-output.md`).

## Behavior

### Output Layout

- The first output mapped is placed at the origin of the shared scene.
- Each subsequent output is placed immediately to the right of the combined width of all previously-mapped outputs (left-to-right, single row, converted to physical pixels via that output's scale). There is no vertical offset — every output's row position is fixed.
- Virtual (screenshare) outputs are placed according to their configured position; if unconfigured, a virtual output defaults to the same origin as the first output, which overlaps it (see Constraints).
- Re-laying-out (e.g. after a mode change, hotplug, or resume) recomputes every output's position from scratch and re-applies it, so positions stay consistent across changes.

### Output-Local Rendering

- Content rendered for a given output — including KMS plane buffers (background, windows, expose, overlay UI, dock, switcher) and the cross-plane backdrop composite used for blur — is always anchored to that output's own top-left corner, never shifted by the output's position in the shared scene.
- An output placed to the right of another one displays its content starting from its own edge; it never shows a horizontally-shifted or partially-blacked-out view of what should be at its origin.
- The only positional information that varies per output within its own rendered content is that output's own workspace-scroll offset (e.g. mid-swipe), never its placement relative to other outputs.

### Per-Output Render Scheduling

- Each output tracks its own idle/active state independently; one output being idle (nothing pending, no timer scheduled) never blocks another output from rendering, and vice versa.
- An input event (or any event that requests a render) wakes exactly the outputs that are currently idle; an output that already has a render pending absorbs the same event through its own in-flight schedule rather than being redundantly kicked.
- Every output that receives activity extends its own active window by a short, fixed tail so a burst of input across multiple outputs doesn't cause any of them to prematurely appear idle.
- Input directed at one output must reliably wake that output's rendering even while other outputs are fully idle, and must not require every output to be idle simultaneously first.

### Cursor Ownership

- The hardware cursor is rendered on an output only when the pointer's current position, expressed in that output's own local coordinate space, falls within that output's geometry.
- Cursor ownership is evaluated independently per output, purely by geometric containment — there is no separate global "which output owns the cursor" arbiter. In the normal case (outputs laid out non-overlapping, per Output Layout above) this means exactly one output contains the pointer at a time.
- **Mirroring corollary:** if two outputs' geometries are ever made to occupy the same region of global space (e.g. a virtual output left at its default, unconfigured position, which coincides with the first output), the pointer satisfies both outputs' containment test simultaneously and the cursor is drawn on both. This is an emergent side effect of geometric containment, not a designed mirroring feature.
- When the pointer moves from inside an output's geometry to outside it, that output renders exactly one further frame with the cursor omitted (a "farewell" frame) so its hardware cursor plane is cleared instead of continuing to display the cursor at its last position. No further cursor-related frames are forced after that single farewell frame while the pointer remains outside.
- An output whose geometry the pointer re-enters resumes drawing the cursor on the next frame, with no farewell frame needed on entry (only on exit).

## Constraints & Edge Cases

- **Config position is not honored for physical outputs:** configuration exposes a position field for physical displays, but physical output placement always uses the automatic left-to-right layout described above — a configured position for a physical display has no effect on where it appears. Only virtual (screenshare) outputs honor a configured position.
- **Virtual outputs default to overlapping the first output:** a virtual output with no configured position is placed at the same origin as the first mapped output, which overlaps it in global space. This is the practical trigger for the cursor mirroring corollary above.
- **No vertical stacking:** all outputs share the same row; there is no way to arrange outputs above/below one another today.
- **No overlap detection:** the automatic layout never checks for or resolves overlaps; overlap can only occur via virtual-output configuration (or default) as noted above.
- **Chrome stays primary-only:** dock, topbar, app switcher, and other shared overlay UI are attached only to the primary output's scene subtree; secondary outputs have empty placeholder layers for the same plane roles but no chrome content (see `workspaces-multi-output.md`).
- **Idle is per-output, not global:** an output that has gone idle does not gate whether another output can be kicked back into rendering by input; each output's idle state is tracked and consumed independently.

## Rationale

- **Left-to-right auto-layout** was chosen as the simplest arrangement that covers the common case (monitors placed side by side) without requiring configuration plumbing for physical displays. It was previously assumed (incorrectly) that every output's scene subtree sat at the shared scene origin; plane buffers and cursor placement were computed against the pointer's raw global coordinates and the root scene position, so any output placed to the right of the first rendered its content shifted (and in the KMS-plane case, mostly black) and drew the cursor at the wrong on-screen location. Both the plane render path and the cursor render path now explicitly subtract the target output's own placement before rendering, restoring output-local correctness regardless of layout.
- **Per-output idle tracking with per-output kicking** replaced an earlier "kick only when every output is idle" rule. That rule reset every output's idle countdown on any input, but only issued an explicit render when *all* outputs were simultaneously idle; once one output idled out with no render scheduled, the "all idle" condition could never become true again on later input, silently wedging rendering across all outputs. Kicking exactly the outputs that are currently idle (while resetting every output's countdown so busy outputs still get their trailing-activity window extended) fixes this without reintroducing double-renders on outputs that already have a render pending.
- **Cursor as pure per-output containment, plus one farewell frame** avoids needing a dedicated cross-output cursor-owner concept: geometric containment against each output's own coordinate space is sufficient for the non-overlapping layout that's actually produced today, and naturally extends to (and documents) the overlap/mirroring corner case rather than needing to special-case it. The farewell frame exists because a hardware cursor plane keeps scanning out its last buffer until told otherwise — without one extra cursor-omitted frame on exit, an output the pointer just left would keep showing a stale cursor frozen at the boundary.

## Open Questions

- Should physical-output configuration position actually be wired up (honoring `[[displays]]` position from config), or should manual output arrangement be dropped from the config schema until it's implemented?
- Should output mirroring become a first-class, explicitly configured feature, given that the geometry-containment cursor logic already tolerates overlapping outputs?
- Is vertical/grid output arrangement worth supporting, or is horizontal-only sufficient for expected hardware setups?
- Should overlap between two physical (non-virtual) outputs be detected and rejected/warned about, given the current layout algorithm has no such check?
