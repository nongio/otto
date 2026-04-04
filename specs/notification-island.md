# Notification Island — Spec

**Status:** draft
**Parent:** dynamic-island

## Summary

A notification island is a group of subsurfaces that represents all notifications from one `app_id`. Multiple islands (notification groups + music) coexist side by side as a centered horizontal row. Only one island at a time can be in **Compact** (focused) mode; the rest are **Mini** circles.

## Core Concepts

### Every element is a subsurface

The island follows a Core Animation-like model: each visual element is its own Wayland subsurface with independent size, position, corner radius, and spring animations via `otto-surface-style-v1`. The parent layer shell surface is just a transparent container.

Elements:
- **Group pill** — one subsurface per notification group (the tab header)
- **Notification card** — one subsurface per notification in the stack
- **Music pill** — one subsurface for the music activity

### Presentation Modes

Each island (group) cycles through three modes via click:

| Mode | Visual | Trigger |
|------|--------|---------|
| **Mini** | Small circle (~28px). App icon + count badge (when count > 1). | Default for unfocused islands. |
| **Compact** | Pill (~240x36). App icon + app name + count badge + chevron. | Click a Mini circle. |
| **Expanded** | Compact pill stays visible. Notification cards slide down below it as a stack. | Click the Compact pill. |

Click cycles: **Mini → Compact → Expanded → Compact** (clicking the pill toggles the stack).

Only **1** island can be Compact/Expanded at a time (configurable, default 1). When one island becomes Compact, the previously focused one shrinks to Mini.

### Hover

Hover causes a subtle size increase on any island element (pill or circle) to invite interaction. No mode change on hover.

## Layout

All islands are arranged as a **centered horizontal row** at the top of the screen:

```
Layer surface (480px wide, anchored top-center):

    [o] [o] [===compact pill===]
    ←────── centered as group ──────→
```

- Elements are ordered left-to-right by arrival time (oldest left, newest right).
- The focused (Compact/Expanded) island is the rightmost non-mini, or whichever was last clicked.
- Gap between elements: 6px.
- Vertically centered within the bar height (30px) for Mini/Compact.

When a stack is open (Expanded mode), cards appear below the pill:

```
    [o] [o] [===compact pill===]
              [  card 1        ]
              [  card 2        ]
              [  card 3        ]
```

Cards are centered under the pill.

## Notification Group Behavior

### One group = one `app_id`

All notifications from the same `app_id` are grouped into one island. The group pill shows the app icon, app name, and count.

### Notification lifecycle

1. First notification from an app → group created, island appears as Mini (or Compact if no other island is focused).
2. More notifications from same app → count increments, pill/circle redraws.
3. Notification times out → stays in the group (not removed). Only dismissed by user action.
4. User clicks a card → invokes default action + dismisses that notification.
5. All notifications dismissed → group disappears, island removed from row.
6. `replaces_id` → updates the notification in place (no reorder).

### Stack (Expanded mode)

- Default max visible cards: **5** (configurable).
- If more than max: show a "+N more" indicator at the bottom.
- Cards are created as subsurfaces **lazily** when the stack opens.
- Cards are destroyed when the stack closes.

### Stack open animation

1. Each card starts at zero height, positioned at the pill bottom.
2. Cards slide down to their final position with staggered delay (0.05s per card).
3. Spring animation (same parameters as all other transitions).

### Stack close animation

1. Cards collapse height to zero, sliding up toward the pill.
2. Card subsurfaces are destroyed after animation.

### Closing the stack

The stack closes when:
- The pill is clicked again (toggle).
- The user clicks outside the island.
- The island loses focus (e.g., user focuses another app/window).

## Subsurface Style

All subsurfaces use `otto-surface-style-v1`:

- `set_background_color(0.03, 0.03, 0.03, 1.0)` — near-black
- `set_corner_radius(r)` — circle for Mini, pill radius for Compact, card radius for cards
- `set_masks_to_bounds(Enabled)` — clip content
- `set_shadow(0.2, 2.0, 0.0, 8.0, 0.0, 0.0, 0.0)` — drop shadow
- `set_blend_mode(BackgroundBlur)` — frosted glass
- `set_contents_gravity(Center)` — content anchored center

No opacity manipulation. All elements are always fully visible.

## Rendering

Content is drawn with Skia into each subsurface's buffer. The buffer is larger than needed (460x100) so content can be drawn at target size before the compositor spring-animates the visual bounds.

### Group pill (Compact)

- App icon (rounded, left)
- App name text (bold, 12pt)
- Count badge (when > 1): semi-transparent pill with white count text
- Chevron indicator (right edge)

### Group circle (Mini)

- App icon centered (60% of circle size)
- Count badge overlay (bottom-right, when > 1)

### Notification card

- Card background: semi-transparent white (alpha 40), rounded corners (10px)
- App icon (24px, left)
- Title (bold, 12pt, white)
- Body (regular, 11pt, dimmed)
- Elapsed time (9pt, bottom-right)
- Card dimensions: 300x60px, gap between cards: 4px

## Coexistence with Music

Music is another island in the same row. It follows the same Mini/Compact/Expanded rules:
- When music is Compact, notification groups are Mini circles.
- When a notification group is clicked to Compact, music shrinks to Mini.
- Music Mini: 3-bar equalizer in a circle.
- Music Compact: album art + title + artist + equalizer.

## D-Bus Integration

- `org.otto.Island1` — custom API for creating arbitrary activities.
- `org.freedesktop.Notifications` — standard notification daemon. Notifications come in here and are grouped by `app_id`.

## Constants

| Name | Value | Description |
|------|-------|-------------|
| LAYER_W | 480 | Layer surface width |
| BAR_HEIGHT | 30 | Topbar area height |
| GAP | 6 | Space between islands |
| MINI_SIZE | 28 | Mini circle diameter |
| COMPACT_W | 240 | Compact pill width |
| COMPACT_H | 36 | Compact pill height |
| CARD_W | 300 | Notification card width |
| CARD_H | 60 | Notification card height |
| CARD_GAP | 4 | Gap between cards |
| CARD_RADIUS | 10 | Card corner radius |
| MAX_VISIBLE_CARDS | 5 | Default max cards in stack |
| MAX_FOCUSED | 1 | Max islands in Compact mode |
| SLOT_BUF_W | 460 | Subsurface buffer width |
| SLOT_BUF_H | 100 | Subsurface buffer height |
| BUFFER_SCALE | 2.0 | HiDPI scale factor |
