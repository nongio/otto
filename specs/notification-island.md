# Notification Island — Spec

**Status:** draft
**Parent:** dynamic-island

## Summary

A notification island **is** one notification. There is no separate group header and no separate card: the island is the notification, and opening it grows that same bubble into the full title/body/actions layout. Islands sit side by side in a centered horizontal row at the top of the screen. Notifications from the same app are grouped *visually*, by overlapping their islands into a deck — not by collapsing them into one representative.

## Core Concepts

### Every element is a subsurface

The island follows a Core Animation-like model: each visual element is its own Wayland subsurface with independent size, position, corner radius, and spring animations via `otto-surface-style-v1`. The parent layer shell surface is just a transparent container.

There is exactly one subsurface per notification. It is never destroyed and recreated to change mode — the same bubble grows and shrinks.

### Presentation Modes

Each island is in one of three modes:

| Mode | Visual | Trigger |
|------|--------|---------|
| **Mini** | Circle (28px). This notification's app icon. No count badge — how many bubbles are stacked behind conveys the count. | Default at rest. |
| **Compact** | Pill (36px tall, width from its own title). Icon + title on one line. | Hover, focus, or an arrival that could not open. |
| **Expanded** | Card (300px wide, height from its content). Icon, title, wrapped body, inline actions, elapsed time, Close zone. | Arrival, or a click on a Compact pill. |

Click grows: **Mini → Compact → Expanded**. Nothing is dismissed until the notification is actually open; a click on an Expanded island acts on it (see *Click targets*).

Exactly **one** island may be Compact at a time, and one Expanded. The Compact slot goes to the pointer first, then whatever was last clicked, then an arrival that could not open. A burst of notifications therefore never blows the row up into a wall of pills.

### Arrival

A new notification opens **Expanded**, so it can be read on sight rather than after a click.

- It stays open for `ARRIVAL_READ_SECS` (6s), then settles back into its stack.
- The window is held open while the pointer is on that island, so it never collapses out from under someone reading it.
- An arrival never takes over an island the **user** opened themselves. In that case it announces itself Compact instead and the open island is left alone.
- Only a user-opened island stays Expanded indefinitely. One that opened on arrival closes again when its window runs out.

### Hover

Hovering an island grows it to Compact and fans out its app's deck so each bubble can be aimed at individually. Hover pauses the focus timeout.

## Layout

Islands are arranged as a **centered horizontal row**, ordered by arrival:

```
Layer surface (anchored top-center):

    [o] [o] [==compact pill==]  [o]
    ←──────── centered as a row ────────→
```

- The row is centred, so an island growing pushes its neighbours **both ways**.
- Each bubble covers a fixed slice of the one before it, so the step comes from the bubble's own width: growing — or opening fully — pushes the newer ones along instead of expanding underneath them. Nothing ever changes places.
- The push **cascades** outward from whichever island grew, `CASCADE_STAGGER_SECS` per island, so the row ripples rather than moving in lockstep. When that island shrinks again the row cascades back in the same order.
- Moving and resizing get their own springs: a bubble shoved aside by its neighbour overshoots and rocks back, while the same bubble growing just settles.

### Same-app decks

Islands sharing an `app_id` overlap into one deck, newest at the front (right-hand end), older ones peeking out to its left.

- At rest the offset is `PEEK_STEP` — a sliver, so the deck reads as one object.
- While the group is hovered it is `FAN_STEP`, far enough that each bubble can be clicked individually.
- Past `MAX_STACK` from the front, bubbles pile up at the same offset, so a huge group cannot grow the row without bound.
- Groups are ordered by their oldest notification, so a group keeps its place in the row as notifications come and go.

## Notification Behavior

### Lifecycle

1. Notification arrives → island created, opens Expanded (see *Arrival*).
2. Another notification from the same app → its own island, joining that app's deck at the front.
3. Notification times out → the island stops announcing itself and settles
   into its deck. It is *not* removed: an unread notification stays, and keeps
   counting towards the app's dock badge.
4. User clicks an open island → invokes an action or closes it, and the notification is dismissed.
5. `replaces_id` → updates the notification in place (no reorder).

### Click targets

On an Expanded island:

- **Close zone** (right `CARD_CLOSE_ZONE` px, behind the separator) → dismiss. Emits `NotificationClosed` with reason 2 (dismissed by the user).
- **An inline action button** → emits `ActionInvoked` with that action's id, focuses the app, dismisses.
- **Anywhere else (the body)** → the default action: same as above with the notification's default action id, or `"default"`.

An action button always wins over the close zone; the buttons sit in the body row, clear of it.

### Dock badges

An island lives at the top of the screen and eventually settles into a Mini
circle, which is a poor place to notice three unread messages from an hour ago.
The count therefore also rides on the app's dock icon, the way a macOS dock
badge does.

otto-islands is the session's notification daemon, so it is the only thing that
knows the counts. It publishes them through `otto_dock_v1`: one
`otto_dock_item_v1` per app, `set_badge` carrying the number of that app's
notifications that have not been read.

- The count is the number of the app's outstanding notifications — those still
  in the island's state. A notification that timed out has *not* been read, so
  it keeps counting; only dismissing it (Close zone, an inline action, the
  default action, or `CloseNotification`) takes it off the badge.
- The badge clears with the last of them.
- A notification that identifies itself no other way — no `desktop-entry` hint
  and no app name, as when a terminal forwards an escape sequence on an app's
  behalf — is attributed to the process that made the D-Bus call, resolved
  through its executable name (`ghostty` → `com.mitchellh.ghostty`). Without
  this, everything sent that way is anonymous and badges nothing.
- **Transient** notifications never badge: they opt out of persistence by
  definition. Neither do non-notification activities (a media island, a
  progress readout) — nothing there is waiting to be read.
- Counts above 99 read as `99+`.
- Badges are diffed against what is already applied, so a state change that
  leaves the counts alone sends nothing.

On the compositor side:

- The `app_id` a notification carries is whatever the sending app put in its
  `desktop-entry` hint — `com.mitchellh.ghostty`, `Ghostty`,
  `ghostty.desktop` — while the dock files its icons under the desktop file
  stem. `AppIconsManager` resolves the id to that key before applying a badge.
- A badge for an app the dock has not drawn yet is remembered and applied when
  its icon stack appears, so a notification that arrives before its app is
  running is not lost.
- The badge belongs to the client that set it: when the dock item is destroyed
  — including a daemon that exits — the badge is cleared, rather than leaving a
  stale count stuck on an icon.

### Focus timeout

After `FOCUS_TIMEOUT_SECS` of no interaction, the focused island shrinks back to Mini. The timer is paused while the pointer is over any island, and does not run while an island is Expanded.

## Subsurface Style

All subsurfaces use `otto-surface-style-v1`:

- `set_background_color(...)` — near-black bubble material
- `set_corner_radius(r)` — circle for Mini, pill radius for Compact, `CARD_RADIUS` for Expanded (noticeably squarer than the pill it grew out of). Radii are **logical**: the compositor multiplies them by the output scale itself, so a client that pre-scales gets a radius scaled twice
- `set_masks_to_bounds(Enabled)` — clip content
- `set_shadow(...)` — drop shadow
- `set_blend_mode(BackgroundBlur)` — frosted glass
- `set_contents_gravity(TopLeft)` — buffer (0,0) is the island's top-left corner, so where content lands never depends on the layer's current (animating) size. `Center` was found to place the visible slice of the buffer at a different, size-dependent origin per island and is not used

No opacity manipulation. All elements are always fully visible.

## Rendering

Content is drawn with Skia into each subsurface's buffer. Before each draw the buffer is **resized to exactly the mode's content size** (`draw_content`), and content is drawn from the origin; the compositor spring-animates the visual bounds over the retained buffer. `SLOT_BUF_W` x `SLOT_BUF_H` is only the size a subsurface is created at, before its first draw.

Geometry pushed to `otto-surface-style-v1` is in **physical pixels** and must use the real output scale (`AppContext::fractional_scale()`), not the client's fixed 2x raster buffer scale.

Rendering is fully retained-mode — the client never pushes frames continuously:

- Each island stores a content signature (mode, icon, title, body, time label, actions, size). A buffer is redrawn and committed only when its signature changes; all motion (springs, resizes, moves) runs compositor-side via the surface-style protocol against the retained buffer.
- The parent layer surface is committed only when its size, input region, or subsurface stacking actually changed.
- The event loop sleeps until the next real deadline (arrival window, focus timeout, deferred destroy, notification timeout) or an external wakeup (Wayland event, D-Bus via `request_wakeup`). With no pending deadlines the process is fully idle — no periodic tick, no frame callbacks.

### Mini

- App icon centered in the circle.

### Compact

- App icon (left) + this notification's own title (bold, 11pt), truncated to the pill width.

### Expanded

Drawn straight into the bubble — there is no separate card background.

- App icon (`CARD_ICON` px, top-left)
- Title (bold, 12pt, white), truncated to the text column
- Body (regular, 11pt, dimmed), **wrapped** over up to `MAX_BODY_LINES` lines at `BODY_LINE_H` spacing; the last line is ellipsised if it still doesn't fit. Words longer than the column are broken mid-word rather than overflowing.
- Inline action buttons, on their own row under the body — not in place of it
- Elapsed time (9pt, bottom-right of the text area)
- Separator line and a `Close` label in the right-hand zone

The card **grows to fit**: its height is the one-line height plus a line for each extra body line, plus the action row when it has actions. Layout, drawing, and hit-testing all size the card through the same function, so a button is clickable exactly where it was painted.

## D-Bus Integration

- `org.otto.Island1` — custom API for creating arbitrary activities.
- `org.freedesktop.Notifications` — standard notification daemon. Each notification becomes its own island; `app_id` only decides which deck it joins.
- `NotificationClosed` and `ActionInvoked` are emitted so senders learn what the user did.
- `otto_dock_v1` — outbound: the unread count per app, published as a dock badge (see *Dock badges*).

## Constants

| Name | Value | Description |
|------|-------|-------------|
| LAYER_W | 800 | Layer surface width |
| LAYER_H | 400 | Layer surface height (tall enough for an open notification) |
| BAR_HEIGHT | 36 | Topbar area height |
| GAP | 6 | Space between islands |
| MINI_H | 28 | Mini circle diameter |
| COMPACT_H | 36 | Compact pill height (width comes from the title) |
| CARD_W | 300 | Expanded card width |
| CARD_H | 68 | Expanded card height with a one-line body and no actions |
| CARD_RADIUS | 12 | Expanded corner radius |
| BODY_LINE_H | 14 | Baseline-to-baseline spacing of wrapped body lines |
| MAX_BODY_LINES | 3 | Body lines before the text is ellipsised |
| CARD_CLOSE_ZONE | 40 | Width of the right-hand Close zone |
| ACTION_ROW_H | 18 | Height of the inline action button row |
| PEEK_STEP | 8 | Deck offset at rest |
| FAN_STEP | 20 | Deck offset while the group is hovered |
| MAX_COUNT | 99 | Dock badge count above which it reads `99+` |
| MAX_STACK | 5 | Deck depth past which bubbles pile up at one offset |
| ARRIVAL_READ_SECS | 6 | How long a new notification stays open to be read |
| FOCUS_TIMEOUT_SECS | 4 | Inactivity before the focused island shrinks to Mini |
| CASCADE_STAGGER_SECS | 0.035 | Per-island delay as a push travels outward |
| DESTROY_DELAY_SECS | 0.8 | How long a destroyed surface is kept for its exit animation |
| SLOT_BUF_W | 460 | Subsurface buffer width at creation (resized to content on first draw) |
| SLOT_BUF_H | 140 | Subsurface buffer height at creation |
