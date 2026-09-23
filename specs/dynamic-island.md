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
| `CreateActivity` | `(s app_id, s title, s icon, d progress, u timeout_ms, s priority, b live, b quiet) -> t` | Create an activity, returning its id. `progress`: 0.0–1.0, negative for none. `priority`: `low`/`normal`/`high`/`critical` (`urgent` is accepted as `critical`). |
| `UpdateActivity` | `(t id, s title, d progress) -> b` | Update title and/or progress. Empty title = no change. Negative progress = clear. |
| `SetActivityQuiet` | `(t id, b quiet) -> b` | Show or hide a running activity's island without ending it. |
| `DismissActivity` | `(t id) -> b` | Dismiss by id. |

Any process can call these. Each mutation wakes the event loop via
`AppContext::request_wakeup()`.

An activity created this way gets an island like any notification.

### Running tasks

A `live` activity is a job rather than something that happened: it does not
expire on a timeout, and it ends when the app that started it says so. Its
`progress` is drawn — a ring around the icon in Mini and Compact, a bar with a
percentage beside it on the open card — and it takes its turn in the row like
anything else, so a job the user is not attending to shrinks to a ring on a
dot.

The same activity is what fills the app's dock icon (see **Dock overlays**),
which is why `quiet` exists: an app whose own window is in front of the user
has already said what it is doing there, and a bubble would only be a second
copy of it. A quiet activity is reported to the dock and not to the row. The
app flips it as its window gains and loses focus; the job is unaffected.

### Dock overlays

otto-islands is the session's notification daemon and the home of its
activities, so it is the one process that knows both what an app has
outstanding and what it is busy with. It publishes both through
`otto_dock_v1`: `set_badge` with the count of unread notifications, and
`set_progress` with how far along that app's live activities are — their mean,
so an app running two jobs gets one honest bar. Quiet counts here; only the
island is quiet. Changes smaller than a percent are not sent.

**Accepted but not yet honoured.** These are part of the interface contract and
callers may pass them, but nothing reads them yet:

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
whole layer so clicks cannot fall through while a decision is pending. A
non-modal one takes the keyboard as it opens (a brief exclusive request, then
on-demand), catches clicks only on itself, and shrinks into a Mini-sized circle
at the end of the island row when the user moves on, still pending. Hovering
the circle peeks at the Compact pill size (icon and title), as an island does;
clicking opens it again. Option rows, buttons and the circle show the hand
cursor. `Esc` denies, `Enter` grants, `Up`/`Down`/`Tab` move a focus ring
through the options — the only keyboard interaction in the app; notifications
never take the keyboard. Text wraps and the panel grows, scrolling past a
maximum height; see [portal-access-dialog](portal-access-dialog.md).

When otto-islands is not running the portal falls back to the GTK, GNOME and KDE
Access backends in that order; only with none of them available does the request
resolve to denied. See [portal-access-dialog](portal-access-dialog.md).

### Notification daemon

otto-islands claims `org.freedesktop.Notifications` at startup; failing to claim
it is logged and the app continues without serving notifications. Notification
presentation, click targets, decks, arrival behaviour and dock badges are
specified in [notification-island](notification-island.md); the daemon contract
itself is in [notification-daemon](notification-daemon.md).

### Music activity

While an MPRIS player has a track loaded, the island shows a persistent live
activity for it:
- **Mini**: three bars in a circle, in the accent colour.
- **Compact**: album art, title, artist and four bars.
- **Expanded**: larger art, title and artist, eight bars, a progress bar with the
  elapsed and remaining time, and previous/play-pause/next controls.

The accent colour is extracted from the album art and used for the bars and the
progress fill.

Players are followed over D-Bus (`mpris.rs`), with no external tool. The island
listens for players appearing and going (`NameOwnerChanged`), and for their
`PropertiesChanged` and `Seeked` signals, and reads every player when one fires. It
wakes only when what it shows changed. `Position` is not signalled, so while a track
plays it is read again once a second for the progress bar; with nothing playing the
island does no work at all.

Browsers publish the same track from two players (an engine player with little
metadata and an integration shim that carries the art), so every player is read and
the entries for one track are merged. A playing player wins over a paused one, and a
paused one over a stopped one. The transport controls call the MPRIS methods on the
player the island shows (`PlayPause`, `Next`, `Previous`, and `SetPosition` for a
seek on the progress bar).

Art comes from the MPRIS `mpris:artUrl` field. The URL is whatever the player
publishes, which for a browser is the web page, so it is loaded with care:
- It is loaded on a worker thread; the island draws the ♪ placeholder until it
  arrives.
- `file://` (optionally `file://localhost/`) is read only if it is a regular file,
  up to 8 MiB.
- `https://` is fetched from public hosts only, never localhost or the local
  network, with a 5 s timeout and an 8 MiB limit. Plain `http://` and other schemes
  are not loaded.
- Images wider or taller than 4096 px are not decoded; the rest are kept at
  256 × 256.

The music island is not news: it arrives as a pill rather than opening to announce
itself, and rests there. It drops to a dot only to make room, while another island is
open, hovered or focused, and goes back to a pill once the row is free again.
Assistive technologies can read it but it is not a live region, so a new track does
not interrupt a screen reader. Clicking the open player outside its controls puts it
back to a pill; nothing about it dismisses the music. It goes as soon as the player
quits, and three seconds after a still-running player stops, so skipping tracks
doesn't close and reopen it. Through those three seconds it keeps showing the last
track.

It is hidden, not dismissed, while the player's own window is focused
(`focus_watcher` in otto-kit): the activity goes quiet and comes back as the same
island. A player goes by its bus name ("spotify", "chromium") and its MPRIS
`DesktopEntry` ("com.spotify.Client"). When a window's app_id matches one of those
names, only such windows are the player's. Otherwise the window whose title holds the
track title is: browsers name their player after the engine whatever the browser's
app_id, but title their window after the active tab, so the island hides only while
the playing tab is showing. Track titles shorter than three characters don't match by
title.

Clicking the album art brings the player forward. It activates the player's own
window (the one showing the track, if it has several), or else the window showing the
track, through `focus_watcher::activate_window` in otto-kit. With no such window it
asks the player over MPRIS `Raise`, which lets a browser switch to the playing tab.

An island is music by its `app_id` (`org.otto.music`), not by coming from inside
otto-islands.

The bars come from `audio_viz`: a PipeWire level meter on the monitor of the default
output, an animator that turns one level into eight independently moving bars, and
the bar drawing. The capture stream names no target, so the session manager links the
default output and moves the stream when the default changes. The bars sit on a child
subsurface of the island, redrawn at ~24 fps only while a track plays and the island
is on screen, so the island buffer itself stays retained. The capture stream is open
only for as long as the bars move: a connected stream keeps the output running, so
leaving it open would stop the sound card from suspending.

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
