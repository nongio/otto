# Dynamic Island — Implementation Milestones

**Status:** draft
**Related specs:** dynamic-island, notification-daemon

## Summary

Incremental implementation plan for the Dynamic Island. Each milestone is self-contained and demoable.

---

## Milestone 1 — End-to-End Activity Lifecycle (DONE)

**Goal:** D-Bus client submits an activity, the island renders it.

**Delivered:**
- Standalone `otto-islands` component with D-Bus interface (`org.otto.Island1`)
- Subsurfaces with surface style chrome (blur, shadow, rounded corners)
- Spring-animated transitions between presentation modes
- Pointer hover and click interaction
- Wayland input region support (compositor fix: honouring `surface_under` for layer shells)

**Since superseded:** this milestone was built on the two-slot "O o" model with
an `ActivityRenderer` trait, a `MusicActivityRenderer` (MPRIS + PipeWire levels)
and a Banner mode. All of that was replaced in Milestone 2/3 by the
per-notification island row; see [dynamic-island](dynamic-island.md#superseded-designs).
Activity timeouts do not currently auto-dismiss.

---

## Milestone 2 — Notification Integration (DONE, differently)

**Goal:** Notifications from desktop apps appear as island activities.

**Scope:**
- `xdg-desktop-portal-otto` implements `org.freedesktop.impl.portal.Notification`
- Portal forwards notifications to island via `CreateActivity` D-Bus call
- Map notification urgency to island priority (low/normal/critical)
- Map notification timeout to island timeout
- Notification actions mapped to island actions (future: clickable buttons)
- Optionally: island claims `org.freedesktop.Notifications` directly for non-portal apps

**Delivered:** otto-islands claims `org.freedesktop.Notifications` directly and
*is* the notification daemon; the portal forwarding route was not taken. Actions
render as inline buttons on an expanded island. See
[notification-island](notification-island.md).

---

## Milestone 3 — Multiple Activities (DONE, differently)

**Goal:** Multiple activities coexist, dismissed ones are recoverable.

**Scope:**
- Support more than 2 concurrent activities (stack model)
- Dismissed transient activities enter a stack with a badge counter
- Click badge → island expands vertically showing dismissed activities
- User can dismiss individual items or clear all
- Same-`app_id` transient activities replace each other (notification grouping)

**Delivered:** an unbounded centred row of per-notification islands, with
same-app islands overlapping into decks. Unread counts ride on the dock icon via
`otto_dock_v1` rather than on an island badge. Same-app notifications are *not*
replaced — each keeps its own island. A dismissed-notification stack is still
open (see history/DND).

---

## Milestone 4 — Playback Controls

**Goal:** Music player controls are interactive.

**Scope:**
- Click prev/play-pause/next buttons in Banner mode sends `playerctl` commands
- Hit-test buttons within the Banner surface
- Scrub the progress bar (drag to seek)
- Album art click opens the player app

**Delivers:** Full media control from the island without switching to the player app.

---

## Milestone 5 — Timer & Recording Activities

**Goal:** Built-in activity types for common use cases.

**Scope:**
- `TimerActivityRenderer` — countdown display, pause/resume/cancel actions
- `RecordingActivityRenderer` — duration counter, recording indicator, stop button
- D-Bus methods: `CreateTimer(duration_ms)`, `CreateRecording(label)`
- Screen recording integration with the screenshare system

**Delivers:** Timer and recording indicators in the island.

---

## Milestone 6 — Permissions & Configuration

**Goal:** Per-app control over island access.

**Scope:**
- Configuration in `otto_config.toml`:
  ```toml
  [island]
  enabled = true
  max_visible_activities = 3

  [island.permissions]
  default_policy = "allow"  # allow | deny | prompt
  ```
- Per-app overrides for priority limits, timeout minimums
- D-Bus introspection for permission queries

**Delivers:** Users control which apps can use the island and how.

---

## Milestone 7 — Exclusive Mode

**Goal:** An activity can lock the island for focused UI.

**Scope:**
- `exclusive` flag on `CreateActivity`
- When exclusive: all other activities hidden, incoming ones queued
- On dismiss: previous layout restores, queued banners play
- Only one exclusive activity at a time
- Use case: fingerprint auth, voice assistant listening

**Delivers:** Security-sensitive and high-focus activities work.

---

## Milestone 8 — Multi-Output & Draggable Position

**Goal:** Island works across outputs and can be repositioned.

**Scope:**
- Island follows focused output (animated transition)
- Drag gesture on idle circles repositions the island horizontally
- Position persists across sessions
- Configuration: `draggable = "horizontal" | "free" | "none"`

**Delivers:** Multi-monitor users and custom placement.

---

## Future (not scoped)

- Voice/AI agent activity with waveform visualization
- Clipboard peek activity
- Calendar/reminder activity
- System alerts (low battery, disk full, updates)
- VPN/network status activity
- Volume/brightness OSD replacement
- Theming integration
- Keyboard navigation
- Animation polish (morph curves, spring tuning)
