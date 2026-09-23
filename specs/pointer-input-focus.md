# Pointer Input Focus

**Status:** draft  
**Related specs:** dynamic-island, context-menus, workspaces-multi-output, window-focus-navigation

## Summary

Defines how Otto resolves which surface receives a pointer button event. Focus must reflect what is visually under the cursor at the moment the button is pressed, even when the cursor has not moved since the target appeared.

## Goals

- A pointer button press is always delivered to the surface that is visually under the cursor at press time.
- Both the compositor's own UI layer (dock, islands, launcher, etc.) and application/Wayland surfaces resolve their focus consistently, against live on-screen positions.
- Keyboard focus and window stacking follow the surface clicked, in the same interaction, except that a drag started from a window behind others leaves it where it is.

## Non-Goals

- Changing behavior on pointer motion, scroll, or release beyond delivering the button to the correctly-focused target.
- Defining click semantics of any specific UI component (those live in that component's spec).
- Touch or gesture input.

## Behavior

- When a pointer button is pressed, before the press is dispatched to any surface, Otto re-resolves the pointer focus at the cursor's live location — for both the compositor UI layer and application surfaces — so that the surface currently under the cursor becomes the pointer focus.
- The press (and the subsequent release) are then delivered to that freshly-resolved surface.
- On press over an application window (when not in a mode that suppresses it, such as show-all/expose), the window under the cursor is raised and given keyboard focus. Clicking a subsurface or popup gives keyboard focus to the owning top-level surface.
- For the left button on a Wayland window, the raise and the keyboard focus wait for the release. The press itself is delivered at once, so the client can select and start a drag. If the press starts a drag (`wl_data_device.start_drag`), the raise is dropped: the source window keeps its place in the stack and the keyboard stays where it was. If the press turns into an interactive move or resize (client- or server-side decorations) or opens a popup with a grab, the raise happens right then, before the grab starts.
- Other buttons, tablet tools and X11 windows raise at the press. So does a left press made while a Top or Overlay layer-shell surface holds the keyboard, so the panel loses it straight away.
- A raise waiting for its release is skipped if the window has meanwhile closed, been minimized or gone fullscreen, or if the session has been locked or expose opened.
- On press over a focusable Top or Overlay layer-shell surface, that surface receives keyboard focus instead, hit-tested against its live on-screen position and honoring its input region.
- When the cursor is over empty space (no surface), the press resolves to no focus and is not delivered to any surface.
- A press on a popup owned by a Top or Overlay layer-shell surface (a bar menu) leaves keyboard focus where it is: it must not move to a window that happens to lie under the popup.
- A press that lands on nothing focusable (empty desktop, the dock) while a Top layer-shell surface holds keyboard focus hands the keyboard back to the top window of the current workspace, or clears it when the workspace is empty. The panel then receives `wl_keyboard.leave`, which is how a bar learns the user clicked away from its menu. Overlay surfaces are modal and keep the keyboard.
- A Top or Overlay layer-shell surface that commits `exclusive` keyboard interactivity is given the keyboard on that commit, each time it switches into `exclusive` from another mode. A repeated commit while it stays `exclusive` does not take the keyboard back from a window it was handed to.
- While such a surface holds the keyboard it receives every key, except the app switcher, volume and brightness shortcuts, which Otto handles because their UI draws above the overlay layer. Once the app switcher is open it keeps the keys until its modifier is released.
- Focus changes that are not pointer-driven — closing the focused window, the app switcher, cycling an application's windows — are specified in window-focus-navigation.

## Constraints & Edge Cases

- A surface that animates, relayouts, or newly appears under a stationary cursor (e.g. dock launch bounce, autohide slide-in, magnification settle, a popup opening beneath the cursor) must still receive a press even though no motion event preceded it. The press must not be silently dropped or delivered to a stale target.
- Layer-shell hit testing is gated on the parent surface's input region: a subsurface extending outside the parent's input region must not intercept the press.
- Compositor-UI hit testing runs against the scene graph, whose coordinates are output-local (see `multi-output.md`), while the pointer position is global. Focus resolution therefore rebases the position onto the output under the pointer before probing the scene, and only for that part of the lookup — application surfaces and windows resolve in global coordinates. On an output that is not at the global origin, skipping the rebase makes every dock and layer-shell press miss.
- Focus resolution for the compositor UI layer and for application surfaces are independent passes and must both run on press.
- A fullscreen window is hit-tested ahead of everything else on its output, but only while it is on the workspace that output is showing. Fullscreening moves the window onto a workspace of its own; after the user switches away, presses resolve against the visible desktop as usual and the off-screen fullscreen window neither takes the press nor the keyboard focus.

## Rationale

- Pointer focus was historically refreshed only on motion events. A cursor held still while the scene changed underneath it left focus pointing at whatever was there before, causing presses to land on the wrong surface or none — perceived by users as "random missed clicks." Re-resolving focus at press time makes the delivered target match what the user sees.
- Focus is resolved for the compositor UI layer and for application surfaces separately because they are tracked independently; refreshing only one would still drop clicks on the other.
- Raising at the press brings a window forward before its client has even seen the button, so nothing can be dragged out of a window behind others: it covers the place the drag was meant to go. Waiting for the release keeps clicks behaving as before, a little later, and lets a drag leave its source where it was. Right and middle presses often open a context menu at the press, and that popup's grab needs its window to hold the keyboard, so they raise at once.

## Open Questions

- Whether releases (and drags started before a scene change) need the same live re-resolution, or whether press-time resolution is sufficient in practice.
