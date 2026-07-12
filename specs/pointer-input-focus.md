# Pointer Input Focus

**Status:** draft  
**Related specs:** dynamic-island, context-menus, workspaces-multi-output

## Summary

Defines how Otto resolves which surface receives a pointer button event. Focus must reflect what is visually under the cursor at the moment the button is pressed, even when the cursor has not moved since the target appeared.

## Goals

- A pointer button press is always delivered to the surface that is visually under the cursor at press time.
- Both the compositor's own UI layer (dock, islands, launcher, etc.) and application/Wayland surfaces resolve their focus consistently, against live on-screen positions.
- Keyboard focus and window stacking follow the surface clicked, in the same interaction.

## Non-Goals

- Changing behavior on pointer motion, scroll, or release beyond delivering the button to the correctly-focused target.
- Defining click semantics of any specific UI component (those live in that component's spec).
- Touch or gesture input.

## Behavior

- When a pointer button is pressed, before the press is dispatched to any surface, Otto re-resolves the pointer focus at the cursor's live location — for both the compositor UI layer and application surfaces — so that the surface currently under the cursor becomes the pointer focus.
- The press (and the subsequent release) are then delivered to that freshly-resolved surface.
- On press over an application window (when not in a mode that suppresses it, such as show-all/expose), the window under the cursor is raised and given keyboard focus. Clicking a subsurface or popup gives keyboard focus to the owning top-level surface.
- On press over a focusable Top or Overlay layer-shell surface, that surface receives keyboard focus instead, hit-tested against its live on-screen position and honoring its input region.
- When the cursor is over empty space (no surface), the press resolves to no focus and is not delivered to any surface.

## Constraints & Edge Cases

- A surface that animates, relayouts, or newly appears under a stationary cursor (e.g. dock launch bounce, autohide slide-in, magnification settle, a popup opening beneath the cursor) must still receive a press even though no motion event preceded it. The press must not be silently dropped or delivered to a stale target.
- Layer-shell hit testing is gated on the parent surface's input region: a subsurface extending outside the parent's input region must not intercept the press.
- Focus resolution for the compositor UI layer and for application surfaces are independent passes and must both run on press.

## Rationale

- Pointer focus was historically refreshed only on motion events. A cursor held still while the scene changed underneath it left focus pointing at whatever was there before, causing presses to land on the wrong surface or none — perceived by users as "random missed clicks." Re-resolving focus at press time makes the delivered target match what the user sees.
- Focus is resolved for the compositor UI layer and for application surfaces separately because they are tracked independently; refreshing only one would still drop clicks on the other.

## Open Questions

- Whether releases (and drags started before a scene change) need the same live re-resolution, or whether press-time resolution is sufficient in practice.
