# Dynamic Island (Otto Islands)

**Status:** draft
**Related specs:** notification-island, notification-daemon, portal-access-dialog, topbar

## Summary

Otto Islands is a morphing UI element anchored to the top-centre of the screen.
It is the session's notification daemon, a surface for arbitrary activities
submitted over D-Bus (`org.otto.Island1`), and the renderer for Access-style
permission dialogs (`org.otto.Dialog1`). It runs as a standalone otto-kit
application on a layer shell surface.

This spec covers the architecture and the D-Bus surface. **How notifications are
presented — modes, layout, decks, arrival, dock badges — lives in
[notification-island](notification-island.md), which is the authority on
presentation.** Implementation milestones are in
[dynamic-island-milestones](dynamic-island-milestones.md).

## Terminology

| Term | Definition |
|------|-----------|
| **Island** | One bubble. Exactly one Wayland subsurface, one notification or activity. |
| **Activity** | The data behind an island: app id, title, body, icon, priority, optional progress, optional timeout. Notifications become activities too. |
| **Deck** | The overlapping stack of islands sharing an `app_id`. |
| **Presentation Mode** | The visual size of an island: Mini, Compact or Expanded. |

## Architecture

### Standalone app

The island is **not** compositor-internal. It is a separate binary
(`otto-islands`) that:

- Uses a `Layer::Overlay` layer shell surface anchored `Anchor::Top`, top margin
  2, exclusive zone 0, `KeyboardInteractivity::OnDemand`. The parent surface is
  a fully transparent container; it is resized to fit the current content and
  carries an input region covering only the visible islands, so clicks pass
  through everywhere else.
- Creates **one subsurface per island**, plus one for a dialog panel. A
  subsurface is never destroyed and recreated to change mode — the same bubble
  grows and shrinks.
- Controls visual appearance (size, position, corner radius, background, blur,
  shadow) via `otto-surface-style-unstable-v1` with spring-animated
  transactions.
- Draws content with Skia via otto-kit, retained-mode: a buffer is redrawn only
  when its content signature changes.
- Accepts activities, notifications and dialog requests over D-Bus.

The compositor has no island-specific knowledge beyond treating it as a regular
layer shell client.

Nothing autostarts it implicitly: it is an `[[exec_once]]` entry, present in the
shipped `otto_config.example.toml`.

### Surface style rendering

The island does not draw its own pill shape with Skia. It uses
`otto-surface-style-unstable-v1` (interfaces `otto_surface_style_v1`,
`otto_style_transaction_v1`, `otto_timing_function_v1`) for background colour,
corner radius, `masks_to_bounds`, shadow, `BackgroundBlur` blend mode and
`contents_gravity(TopLeft)`. All size and position transitions are
spring-animated transactions committed by the compositor. See
[notification-island](notification-island.md#subsurface-style) for the exact
per-mode values and the scale/gravity pitfalls.

### D-Bus API — `org.otto.Island1`

Path `/org/otto/Island`.

| Method | Signature | Description |
|--------|-----------|-------------|
| `CreateActivity` | `(s app_id, s title, s icon, d progress, u timeout_ms, s priority, b live) -> t` | Create an activity, returning its id. `progress`: 0.0–1.0, negative for none. `priority`: `low`/`normal`/`high`/`critical` (`urgent` is accepted as `critical`). |
| `UpdateActivity` | `(t id, s title, d progress) -> b` | Update title and/or progress. Empty title = no change. Negative progress = clear. |
| `DismissActivity` | `(t id) -> b` | Dismiss by id. |

Any process can call these. Each mutation wakes the event loop via
`AppContext::request_wakeup()`.

An activity created this way gets an island like any notification.

**Accepted but not yet honoured.** These are part of the interface contract and
callers may pass them, but nothing reads them yet:

- `live` — intended to drive an animated indicator.
- `progress` — stored and exposed to accessibility as a numeric value; no
  progress bar is drawn.
- `timeout_ms` — expiry marks the activity `expired` (which stops it announcing
  itself) but never removes it. `IslandState::expire_timeouts` exists and is not
  called. Removal is `DismissActivity`, a user dismissal, or `CloseNotification`.
- `Priority::rank` and `IslandState::top_activity`/`second_activity` — left from
  the superseded two-slot model, kept for a future priority-based selection.

### D-Bus API — `org.otto.Dialog1`

Path `/org/otto/Dialog`. Mirrors `org.freedesktop.impl.portal.Access` so
`otto-portal` can route both external portal Access requests and internal ones
through the island UI. `PresentAccess` blocks until the user answers or the
caller disappears, returning `(response, results)` with response `0` granted,
`1` denied/cancelled, `2` ended. A modal dialog takes an input region over the
whole layer so clicks cannot fall through while a decision is pending. `Esc`
denies, `Enter` grants — the only keyboard interaction in the app.

When otto-islands is not running the portal falls back to the GTK, GNOME and KDE
Access backends in that order; only with none of them available does the request
resolve to denied. See [portal-access-dialog](portal-access-dialog.md).

### Notification daemon

otto-islands claims `org.freedesktop.Notifications` at startup; failing to claim
it is logged and the app continues without serving notifications. Notification
presentation, click targets, decks, arrival behaviour and dock badges are
specified in [notification-island](notification-island.md); the daemon contract
itself is in [notification-daemon](notification-daemon.md).

## Constraints & Edge Cases

- **Multi-output:** the island appears on the primary output. Layer-shell scene
  containers are primary-only chrome — see
  [multi-output](multi-output.md).
- **Sizing:** the island centres its row in a 36pt band while otto-bar is 30pt.
  The two constants live in different crates and are not shared; a taller custom
  panel overlaps the island.
- **Configuration:** none. Every dimension and timeout is compiled in.
- **Scaling:** layout is in logical pixels; geometry pushed to the surface-style
  protocol is physical and must use `AppContext::fractional_scale()`, not the
  fixed 2x raster buffer scale.
- **D-Bus name collision:** a second instance fails to claim `org.otto.Island` —
  single instance by design.
- **Fullscreen windows:** the island is on the Overlay layer. Behaviour with
  fullscreen windows is TBD.

## Rationale

- **Standalone app vs compositor-internal:** faster iteration, cleaner
  separation. The compositor needs no island-specific code, and layer shell
  gives correct z-ordering.
- **D-Bus vs a Wayland protocol:** a data-driven D-Bus API covers every
  identified use case (notifications, timers, recordings, downloads, system
  alerts) without any client rendering its own surface inside the island, and is
  callable from any language.
- **Surface style for chrome:** the compositor owns the shape, blur, shadow and
  spring animations; the island app only draws flat content. Smooth
  compositor-level motion without client-side chrome rendering — and, with
  retained-mode drawing, without the client pushing frames at all.
- **One subsurface per notification:** grouping notifications into a single
  representative loses which one you are looking at. Overlapping them into a deck
  keeps each addressable while still reading as one object.

## Superseded designs

Recorded so they are not re-proposed:

- **Idle "O o" circles.** The island once drew a large `O` and small `o` at rest,
  echoing the Otto logo. It draws nothing at rest now: an empty island list
  short-circuits layout and the parent surface is cleared transparent.
- **Two fixed slots** (active left, secondary right), with activities shifting
  between them. Replaced by the centred row of per-notification islands.
- **Banner mode** as a fourth presentation above Expanded, and the
  `ActivityRenderer` trait with per-type `GenericActivityRenderer` /
  `MusicActivityRenderer` implementations. The trait and `PresentationMode` enum
  survive in `activity.rs` as dead code.
- **Music activity.** There is no MPRIS integration in otto-islands; nothing
  populates a now-playing island automatically.

## Open Questions

1. **Keyboard navigation:** should there be a global shortcut to focus the
   island, and keyboard traversal of the row?
2. **Theming:** inherit from a global theme, or keep its own style overrides?
3. **Multi-output:** how should the island behave across multiple outputs?
4. **Persistence:** should notification state survive an island restart? The
   daemon currently advertises the `persistence` capability without implementing
   it.
5. **History and do-not-disturb:** where does a dismissed notification go?
6. **Activity limits:** should there be a per-client and global cap on
   concurrent activities?
