# Multi-Output Rendering & Scheduling

**Status:** draft  
**Related specs:** workspaces-multi-output.md, plane-scanout.md, pointer-input-focus.md

## Summary

Otto can drive more than one output (physical monitor or virtual/screenshare output) at once. This spec covers the infrastructure shared by all outputs regardless of what they display: how outputs are laid out relative to each other in the global (window-management/input) layout versus the render scene, how damage and per-output KMS plane buffers stay correct across outputs, how window/space lookups and drags behave with per-output workspaces, how the render loop schedules and wakes each output independently, and which output(s) show the hardware cursor. Workspace lifecycle/navigation is covered by `workspaces-multi-output.md`; the KMS plane-decomposition mechanism itself is covered by `plane-scanout.md`.

## Goals

- Multiple outputs can be active at once, each showing its own content, without one output's position affecting what another output visually displays.
- A newly connected output is placed into the shared layout automatically, with no visible content shift on outputs already active.
- Input events (pointer motion/click, keyboard) reliably wake rendering on whichever output needs it, even when other outputs are idle or busy.
- The hardware cursor is visible on the output(s) the pointer geometrically occupies, and disappears promptly from an output the pointer has left.

## Non-Goals

- Vertical or grid output arrangement — only a single horizontal row is supported.
- A first-class output-mirroring feature (showing the same content on two outputs by design) — see Constraints for the one unintentional overlap case that remains.
- Per-output dock, topbar, or app switcher — these remain primary-output-only (see `workspaces-multi-output.md`).
- Drag mirrors: while dragging a window across the boundary between two outputs, only one output shows the window at a time (see Cross-Output Drag below). Showing a live preview of the dragged window on both outputs simultaneously while it straddles the boundary is planned but not implemented.

## Behavior

### Output Layout (global / input space)

- The first output mapped is placed at the origin of the shared global layout (the coordinate space used by window management, input, and drag/drop — not the render scene, see Output-Local Rendering below).
- Each subsequent physical output is auto-placed immediately to the right of the combined width of all previously-mapped outputs (left-to-right, single row, logical coordinates), unless a configured position is set for it.
- A configured position is honored for physical outputs when it does not overlap any already-mapped output's geometry; an overlapping configured position is rejected and that output falls back to auto left-to-right placement instead (outputs are never allowed to overlap in the global layout).
- Virtual (screenshare) outputs are placed according to their configured position, following the same overlap rule as physical outputs; if unconfigured, a virtual output defaults to the same origin as the first output, which overlaps it (see Constraints).
- Re-laying-out (e.g. after a mode change, hotplug, or resume) recomputes every output's global position from scratch and re-applies it, so positions stay consistent across changes.

### Output-Local Rendering (scene space)

- Every output's render scene subtree — the per-output container layer holding its background, windows, expose, overlay UI, dock, and switcher layers — is positioned at scene coordinate (0,0) and sized to that output's own physical extent. Output subtrees intentionally overlap one another in scene space.
- Each CRTC/output render pass walks only its own output's subtree (via that output's dedicated plane-element node references), so scene coordinates are output-local by construction: there is no shared/global scene position to subtract or correct for, and no output's content can appear shifted or partially blacked out relative to another's.
- The scene root and the fallback (non-plane) composite scene element are each sized to fit the single largest mapped output, not the union of all outputs — since subtrees overlap rather than tile, the scene never needs to be wider or taller than the biggest output.
- A vestigial per-plane-element "scene origin" correction still exists in the renderer but is always applied as (0,0) today, since every output layer's scene position is fixed at the origin; it has no effect while output subtrees overlap and exists only as forward-compatible plumbing.
- Global side-by-side layout (see Output Layout above) governs the smithay `Space` (window locations), pointer/input geometry, and drag/drop — it is unrelated to, and not reflected in, scene-graph coordinates.
- The only positional information that varies per output within its own rendered scene content is that output's own workspace-scroll offset (e.g. mid-swipe), never a placement relative to other outputs.
- Consumers that hit-test against the scene graph — not just per-CRTC plane rendering — must rebase the pointer's global position to the relevant output's own origin before testing, since the shared scene root itself carries no per-output offset; testing with an un-rebased global position hits whichever output's overlapping subtree happens to be topmost, not the output the pointer is actually over. Scene-graph pointer hit-testing (hover, dock and other overlay-UI interaction) rebases against the *focused* output specifically (see Focused Output below) on every pointer-motion event, not only while a particular UI mode is open.
- Virtual (PipeWire) outputs composite their frame from the same per-plane subtree decomposition as the KMS path — one isolated render per plane subtree (background, windows, expose, overlay, switcher, dock), stacked in z-order into the PipeWire buffer — NOT from a single render of the output's whole layer tree. Plane subtrees ignore ancestor visibility (the `workspaces_layer` that parents background/windows/expose is hidden while expose or an expose gesture is up), so a whole-tree render skips all workspace content and produces a black frame during expose and during any vertical-swipe gesture, including a downward overshoot from the closed state. Like the physical push order, the windows subtree is dropped from the stack while expose is active; each subtree re-applies the dynamic part of its root's scene position (workspace scroll) exactly as the plane elements do.

### Focused Output

- Otto tracks a single "focused output" — the output the pointer was most recently confirmed to be over — as a distinct notion from per-frame cursor-ownership containment (see Cursor Ownership below). It resolves to the primary output whenever no focus has been recorded yet, and it is what any shared, single-instance observer of "the current output" (the flattened workspace model, expose, the workspace-selector strip) reflects.
- The focused output is (re-)resolved on every pointer-motion event, on every input path alike — physical relative motion, physical absolute motion, and virtual-pointer (test-harness) motion — by testing the pointer's current global position for output containment; re-recording the same output is a no-op.
- Virtual-pointer (test-harness) motion is clamped to the combined output bounds exactly like real input, so a synthesized move cannot accumulate an unbounded off-screen position that would leave the focused output unresolvable or stuck.
- A virtual pointer created with an output binding (`create_virtual_pointer_with_output`) maps `motion_absolute` coordinates into that output's global geometry; without a binding, absolute motion maps to the first output. This is what lets an external driver (e.g. the `otto-rdp` bridge) aim absolute input at a specific output — including an interactive virtual output — rather than whichever output enumerates first.
- Virtual-pointer motion drives the compositor UI layer exactly like real input: the position is forwarded to the scene engine in physical pixels (using the pointed-at output's scale) and the dock hot zone is checked, so hover states, dock labels, and press-time UI hit-testing follow synthesized moves instead of sticking to the physical mouse's last position.

### Per-Output Render Scheduling

- Each output tracks its own idle/active state independently; one output being idle (nothing pending, no timer scheduled) never blocks another output from rendering, and vice versa.
- An input event (or any event that requests a render) wakes exactly the outputs that are currently idle; an output that already has a render pending absorbs the same event through its own in-flight schedule rather than being redundantly kicked.
- Every output that receives activity extends its own active window by a short, fixed tail so a burst of input across multiple outputs doesn't cause any of them to prematurely appear idle.
- Input directed at one output must reliably wake that output's rendering even while other outputs are fully idle, and must not require every output to be idle simultaneously first.

### Damage Delivery Across Outputs

- Scene damage is a single shared signal (one scene graph feeds every output's overlapping subtree), so it is consumed by whichever output's render tick observes it first; a global tick counter increments every time that shared signal reports damage, and every output remembers which tick it last rendered.
- An output whose last-rendered tick is behind the current counter must render even if its own tick observes no damage — the damage already happened and was consumed by another output on an earlier tick. This is what keeps a secondary output's windows from freezing on whatever frame happened to run first.
- Damage bookkeeping in the shared scene is only cleared once every output has caught up to the current tick; an output that is idle and behind is explicitly scheduled to render (rather than waiting for its own next natural wakeup) so it cannot stay perpetually behind.
- Every output always renders its very first frame regardless of the shared damage signal, since that signal may already have been consumed by another output before the new output gets its turn — skipping would leave a newly connected or newly woken output permanently black.

### Per-Output Window & Space Lookups

- A window belongs to exactly one output's workspace set at a time. Every lookup that answers "where is this window" or "what's under this point" — hit-testing, a window's location or geometry, frame-callback delivery, and "which output(s) show this window" — searches every output's workspaces rather than only the primary's, so these queries behave correctly for windows on secondary outputs, not just the primary.
- Frame-callback bookkeeping (surface enter/leave tracking) runs for every output's workspaces every refresh, not only the primary's — a client on a secondary output keeps receiving frame callbacks and does not freeze.

### Cross-Output Drag

- Dragging a window is tracked by the window's center point. While the center remains within the output whose workspace currently owns the window, the drag behaves as a same-output move.
- The moment the window's center crosses into another output's region, the window migrates: it is removed from the source output's workspace/space and added to the target output's workspace/space (the same output resolution a drop would use — current owner, then the input-focused output, then the primary output, as fallbacks if the point doesn't land inside any output).
- A migrated (or otherwise moved) window's on-screen layer position is always written in the target output's own local scene coordinates, consistent with Output-Local Rendering above — never as a raw global/logical coordinate.
- Only one output shows the dragged window at a time; there is no live preview on the source output after migration (see Non-Goals: drag mirrors).

### Direct-Scanout Promotion

- Direct-scanout candidate selection and the promoted-window cap are evaluated independently per output, using only that output's own workspace; a window can only be promoted onto the CRTC of the output whose space actually contains it. Full detail is in `plane-scanout.md`.

### Cursor Ownership

- The hardware cursor is rendered on an output only when the pointer's current position, expressed in that output's own local coordinate space, falls within that output's geometry.
- Cursor ownership is evaluated independently per output, purely by geometric containment — there is no separate global "which output owns the cursor" arbiter. In the normal case (outputs laid out non-overlapping, per Output Layout above) this means exactly one output contains the pointer at a time.
- **Mirroring corollary:** if two outputs' geometries are ever made to occupy the same region of global space (e.g. a virtual output left at its default, unconfigured position, which coincides with the first output), the pointer satisfies both outputs' containment test simultaneously and the cursor is drawn on both. This is an emergent side effect of geometric containment, not a designed mirroring feature.
- When the pointer moves from inside an output's geometry to outside it, that output renders exactly one further frame with the cursor omitted (a "farewell" frame) so its hardware cursor plane is cleared instead of continuing to display the cursor at its last position. No further cursor-related frames are forced after that single farewell frame while the pointer remains outside.
- An output whose geometry the pointer re-enters resumes drawing the cursor on the next frame, with no farewell frame needed on entry (only on exit).

## Constraints & Edge Cases

- **Config position is honored for physical outputs, with overlap rejection:** a configured position for a physical display is used as-is as long as it doesn't overlap any already-mapped output's geometry; if it would overlap, the configured position is ignored (with a warning) and that output falls back to automatic left-to-right placement instead. Outputs can never be made to overlap through configuration alone.
- **Virtual outputs default to overlapping the first output:** a virtual output with no configured position is placed at the same origin as the first mapped output, which overlaps it in global space. This is the one remaining, unintentional path to output overlap (the overlap-rejection rule above only applies when a position is actually configured) and is the practical trigger for the cursor mirroring corollary above.
- **No vertical stacking:** all outputs share the same row; there is no way to arrange outputs above/below one another today.
- **Chrome stays primary-only:** dock, topbar, app switcher, and other shared overlay UI are attached only to the primary output's scene subtree; secondary outputs have empty placeholder layers for the same plane roles but no chrome content (see `workspaces-multi-output.md`). This also means primary-only chrome never occludes direct-scanout candidates on a secondary output (see Direct-Scanout Promotion above). The dock/app-switcher strip planes are likewise pushed to the CRTC only on the primary output — a secondary output never submits an (empty) plane for either role, which also saves display fetch bandwidth (see `plane-scanout.md`).
- **Idle is per-output, not global:** an output that has gone idle does not gate whether another output can be kicked back into rendering by input; each output's idle state is tracked and consumed independently.
- **Global layout and scene layout are two different coordinate systems:** an output's position in the side-by-side global layout (Space/input) has no relationship to its position in the render scene (always (0,0), overlapping every other output's subtree). Code that needs an output's on-screen scene content must never reuse its global position for that purpose, and vice versa.

## Rationale

- **Left-to-right auto-layout** was chosen as the simplest arrangement that covers the common case (monitors placed side by side) without requiring configuration plumbing for physical displays; a configured position is layered on top of it (with overlap rejection) for the cases where the user does want explicit placement.
- **Overlapping per-output scene subtrees** replaced an earlier design where every output's scene subtree was positioned side-by-side in scene space (mirroring the global layout) and each plane/cursor render explicitly subtracted the output's own scene placement to stay output-local. That correction was easy to miss at each new call site and left a class of "forgot to subtract the origin" bugs (shifted or black content, misplaced cursor) whenever a new render or hit-testing path was added. Making every output's subtree live at scene (0,0) and overlap — with each CRTC's render walking only its own subtree — removes the need for that correction entirely: scene coordinates are output-local by construction, not by convention. Global (Space/input) layout intentionally still stays side-by-side, since window management and input still need real, non-overlapping output geometry.
- **Per-output idle tracking with per-output kicking** replaced an earlier "kick only when every output is idle" rule. That rule reset every output's idle countdown on any input, but only issued an explicit render when *all* outputs were simultaneously idle; once one output idled out with no render scheduled, the "all idle" condition could never become true again on later input, silently wedging rendering across all outputs. Kicking exactly the outputs that are currently idle (while resetting every output's countdown so busy outputs still get their trailing-activity window extended) fixes this without reintroducing double-renders on outputs that already have a render pending.
- **A global damage-generation counter** replaced relying on the shared scene's damage flag being observed directly by every output. Because damage is a single shared signal consumed by whichever output's render tick runs first, a second (or later) output's own tick could see "no damage" even though real damage occurred — freezing that output on its first frame, or leaving it black. Tracking a monotonic generation per surface and forcing a render whenever a surface is behind the global counter (plus always drawing an output's very first frame unconditionally) closes that gap without requiring every output to observe every damage event directly.
- **Focused output tracked on every pointer-input path, not only the windowed backend** replaced tracking it solely from the winit backend's motion handler. On bare metal (udev, both relative and absolute motion) and via the virtual-pointer path used by test harnesses, the focused output was never updated, so it stayed permanently stale after the first real pointer move — any single-instance observer of "the current output" (expose, the workspace selector, the flattened model) would keep resolving to whatever output was focused at startup instead of following real input. Clamping virtual-pointer motion to the output bounds (rather than leaving it unclamped, which was originally a deliberate simplification for test harnesses) closes the matching gap where an unbounded synthesized move could park the pointer off-screen and make focused-output resolution fail entirely.
- **Per-output space lookups** replaced lookups that only consulted the primary output's workspace. With windows now genuinely distributed across per-output workspaces (rather than all living in one shared space), primary-only lookups silently failed for anything on a secondary output — pointer focus, frame callbacks, and drag scale factors would resolve incorrectly or not at all. Searching every output's workspaces (a window lives in exactly one, so this is unambiguous) fixes this at the small cost of a linear scan over the (small) number of outputs.
- **Cursor as pure per-output containment, plus one farewell frame** avoids needing a dedicated cross-output cursor-owner concept: geometric containment against each output's own coordinate space is sufficient for the non-overlapping layout that's actually produced today, and naturally extends to (and documents) the overlap/mirroring corner case rather than needing to special-case it. The farewell frame exists because a hardware cursor plane keeps scanning out its last buffer until told otherwise — without one extra cursor-omitted frame on exit, an output the pointer just left would keep showing a stale cursor frozen at the boundary.

## Open Questions

- Should output mirroring become a first-class, explicitly configured feature, given that the geometry-containment cursor logic already tolerates overlapping outputs, and virtual outputs already default to overlapping the first output?
- Is vertical/grid output arrangement worth supporting, or is horizontal-only sufficient for expected hardware setups?
- Should a dragged window that straddles two outputs show a mirror/preview on both while it's being dragged, instead of only migrating once its center crosses the boundary?
