# Dynamic Island (Otto Islands)

**Status:** draft
**Related specs:** notification-daemon, topbar

## Summary

Otto Islands is a persistent, morphing UI element anchored to the top-center of the screen. It provides a unified surface for live status, notifications, and contextual controls. Activities are submitted by any process via a D-Bus API (`org.otto.Island1`); the island app owns all rendering. It runs as a standalone otto-kit application using a layer shell surface.

## Terminology

| Term | Definition |
|------|-----------|
| **Island** | The otto-kit app that renders the pill-shaped UI element with background blur, rounded corners, and drop shadow. |
| **Activity** | A data-driven unit of content displayed in the island. Activities have a title, icon, priority, optional progress, and optional timeout. |
| **Live** | An activity attribute. Live activities are actively updating (e.g., elapsed time, audio levels). The island may show an animated indicator. |
| **Presentation Mode** | The visual size of an activity: Idle, Compact, Expanded, or Banner. |

### Presentation Modes

| Mode | Description |
|------|-------------|
| **Idle** | No activity. Two small circles ("O o") displayed at top-center within the topbar area. |
| **Compact** | Default active state. Small pill with content (e.g., album art + title + equalizer). Vertically centered in the topbar area. |
| **Expanded** | Hover state. Taller pill showing more detail (e.g., bigger art, progress bar). Pinned to top, grows downward. |
| **Banner** | Click/open state. Full card with interactive controls (e.g., playback controls, action buttons). Pinned to top, grows downward. |

## Architecture

### Standalone App

The island is **not** compositor-internal. It is a separate binary (`otto-islands`) that:
- Uses a `Layer::Overlay` layer shell surface anchored to the top-center of the screen.
- Creates two subsurfaces for the two activity slots (left "O" and right "o").
- Controls visual appearance (size, position, corner radius, background, blur, shadow) via the `otto-surface-style-v1` protocol with spring-animated transactions.
- Draws content with Skia via otto-kit.
- Accepts activities over D-Bus.

The compositor has zero knowledge of the island beyond treating it as a regular layer shell client.

### Two-Surface Layout

The island has two Wayland subsurfaces side by side:

- **Left ("O")** — the **active** activity. Larger circle at idle, morphs to a pill when hosting an activity.
- **Right ("o")** — the **previous/secondary** activity. Smaller circle, renders in minimal mode (e.g., 3-bar equalizer for music, first letter for generic).

When idle, the two circles resemble the Otto logo: a larger "O" and a smaller "o" side by side.

When a single activity arrives:
1. The left surface morphs from circle to compact pill.
2. The right "o" circle slides behind the left pill and hides (overlaps, becoming invisible behind the larger surface).

When a second activity arrives:
1. The first activity shifts to the right surface (shrinks to minimal, slides out from behind).
2. The new activity takes the left surface.

When the active activity is dismissed:
1. If there is a previous activity in the right surface, it moves left and morphs to compact. The right "o" hides behind again.
2. If nothing remains, both surfaces return to idle circles ("O o").

### Surface Style Rendering

The island does **not** draw its own pill shape with Skia. Instead, it uses the `otto-surface-style-v1` protocol:
- `set_background_color` — dark fill (near-black, 0.03 alpha)
- `set_corner_radius` — circle when idle (radius = half size), pill when active
- `set_masks_to_bounds` — clips content to the rounded shape
- `set_shadow` — drop shadow
- `set_blend_mode(BackgroundBlur)` — frosted glass effect
- `set_contents_gravity(TopLeft)` — content pinned to top-left; visual size reveals/hides content

All size and position transitions use **spring-animated transactions**:
```
timing = create_timing_function()
timing.set_spring(damping, stiffness)
animation = begin_transaction()
animation.set_duration(duration)
animation.set_timing_function(timing)
style.set_size(w, h)
style.set_position(x, y)
style.set_corner_radius(r)
animation.commit()
```

Content is drawn at the target size **before** the animation starts. The compositor reveals the pre-drawn content as the spring animation expands the visual bounds.

### D-Bus API

Interface: `org.otto.Island1` at path `/org/otto/Island`

#### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `CreateActivity` | `(s app_id, s title, s icon, d progress, u timeout_ms, s priority, b live) -> t` | Create a new activity. Returns the activity ID. `progress`: 0.0-1.0 for a progress bar, negative for none. `priority`: "low", "normal", "high", "critical". `timeout_ms`: 0 for persistent, >0 for auto-dismiss. |
| `UpdateActivity` | `(t id, s title, d progress) -> b` | Update an existing activity's title and/or progress. Empty title = no change. Negative progress = clear. |
| `DismissActivity` | `(t id) -> b` | Dismiss an activity by ID. |

Any process can call these methods. The island wakes its event loop via `AppContext::request_wakeup()` on each mutation.

### ActivityRenderer Trait

Each activity type knows how to draw itself at each presentation size:

```rust
pub trait ActivityRenderer {
    fn size(&self, mode: PresentationMode) -> (f32, f32);
    fn draw(&self, canvas: &Canvas, mode: PresentationMode, w: f32, h: f32);
}
```

Built-in renderers:
- **GenericActivityRenderer** — title text + optional progress bar + optional live dot. Used for notifications, timers, downloads.
- **MusicActivityRenderer** — album art + title/artist + equalizer bars + progress bar + playback controls. Uses MPRIS (playerctl) for metadata and PipeWire for audio levels.

Future renderers: TimerRenderer (countdown), RecordingRenderer (duration + stop), VoiceRenderer (waveform).

## Behavior

### Idle State

When no activities are present, the island shows two small circles ("O o") vertically centered within the topbar area (30px height, matching topbar dimensions). The circles have dark background, blur, shadow, and rounded corners via surface style.

### Activity Lifecycle

1. A client calls `CreateActivity` over D-Bus with app_id, title, icon, priority, timeout, and live flag.
2. The island assigns the activity to the left surface. If an activity was already there, it shifts to the right surface.
3. The left surface animates from its current state to the compact pill size (spring animation on size, position, corner radius).
4. The right surface animates to reposition next to the left pill.
5. If `timeout_ms > 0`, a timer starts. When it fires, the activity is dismissed.
6. The client can call `UpdateActivity` to change title/progress at any time.
7. The client calls `DismissActivity` or the timeout fires. The activity is removed. Layout adjusts.

### Activity Priority

Activities are sorted by priority (critical > high > normal > low), then by recency (most recent first). The top activity gets the left surface. The second gets the right surface.

### Pointer Interaction

| Interaction | Behavior |
|-------------|----------|
| **Hover** | Left surface expands from Compact to Expanded mode (spring animation). Shows more detail. |
| **Click** | Left surface expands from Expanded to Banner mode (spring animation). Shows full controls. |
| **Click outside / Leave** | Returns to Compact mode. |

The island sets a Wayland input region matching the visible pill and circle bounds. The compositor respects this region for pointer hit testing (via `surface_under` checking input regions).

### Hover background

On hover, the surface style background color animates from dark (0.03) to slightly lighter (0.10) with a spring transition. On leave, it animates back.

### Music Activity

When music is playing (detected via `playerctl`), the island automatically creates a persistent live activity:
- **Compact**: album art + title + artist + 8-bar equalizer (PipeWire audio levels)
- **Expanded**: larger art + title/artist + equalizer + progress bar
- **Banner**: full card with art + title/artist + prev/play-pause/next controls + progress bar
- **Minimal** (right surface): 3-bar mini equalizer with accent color

Album art accent color is extracted and used for equalizer bars and progress fill.

### Positioning

- The layer shell surface is anchored to `Top` with a small margin (matching topbar).
- In Idle/Compact mode, content is vertically centered within the topbar bar height (30px).
- In Expanded/Banner mode, content is pinned to top (y=0) and grows downward below the bar area.
- The left surface is horizontally centered in the layer.
- The right surface positions itself to the right of the left surface with a gap.

## Integration

### Portal Notifications (future)

`xdg-desktop-portal-otto` will implement `org.freedesktop.impl.portal.Notification` and forward notifications to the island by calling `org.otto.Island1.CreateActivity` over D-Bus. This keeps the island as the single rendering surface and the portal as the bridge to the freedesktop notification spec.

### Direct Notifications (future)

The island may also claim `org.freedesktop.Notifications` directly for non-sandboxed apps that bypass the portal.

## Constraints & Edge Cases

- **Multi-output:** The island appears on the primary output. Multi-output support is a future milestone.
- **Fullscreen windows:** The island is on the Overlay layer. Behavior with fullscreen windows is TBD (may need compositor cooperation to hide).
- **Scaling:** All dimensions are in logical pixels. The surface style uses physical pixels (2x buffer scale for HiDPI).
- **Activity limit:** Maximum concurrent activities per client (default 5) and globally (default 20).
- **D-Bus name collision:** If started twice, the second instance fails to claim `org.otto.Island` — single instance by design.
- **PipeWire sink changes:** The audio level monitor connects to the default sink at startup. If the user changes audio output, the equalizer may go flat until restart.

## Rationale

- **Standalone app vs compositor-internal:** Faster iteration, cleaner separation of concerns. The compositor doesn't need island-specific code. Layer shell provides correct z-ordering.
- **D-Bus vs Wayland protocol:** A data-driven D-Bus API covers all identified use cases (music, notifications, timers, recordings, downloads, system alerts). No client needs to render its own surface inside the island. D-Bus is language-agnostic and simple to use from any process.
- **Surface style for chrome:** The compositor handles the pill shape, blur, shadow, and spring animations. The island app only draws flat content. This gives smooth compositor-level animations without client-side rendering of the chrome.
- **Two surfaces:** Allows showing the active + previous activity simultaneously, with independent animations. The right circle provides continuity when activities change.
- **ActivityRenderer trait:** Extensible pattern for different activity types. Each type defines its own rendering for all presentation modes. New renderers can be added without changing the core layout logic.

## Experiment: Activity Stack Model

**Status:** proposed — replaces the two-surface swap model

### Concept

Activities are arranged as a **horizontal stack** of circles/pills. Only **N** activities are expanded at a time (default N=1). New activities always enter from the right. The stack shifts left to make room.

The key rules:

1. **New activity arrives** → slides in from the right as a circle, then expands to compact pill. The currently expanded activity shrinks to a circle and shifts left.
2. **When an activity expands** → all others shrink to circles.
3. **When an activity shrinks/dismisses** → the previous activity (next in the stack to the left) expands.
4. **N=1** means at most one pill at a time. The rest are minimal circles.

### Idle

Two circles side by side: "O" (30px) and "o" (20px), vertically centered in the topbar area. The Otto logo mark.

### Activity Arrives

1. Current pill **shrinks** in place to a circle and **slides left**.
2. New activity **slides in from the right** as a circle, then **expands** to compact pill.
3. The stack is now: `[old circle] [new pill]`

### Activity Dismissed

1. The dismissed activity **shrinks** to a circle and **slides out right** (fades out).
2. The next activity in the stack (to the left) **expands** from circle to compact pill.
3. If no activities remain, both circles return to idle "O o" positions.

### Example: Music + Notification

```
State 0: Music playing
  [===music pill===]  (o hidden behind)

State 1: Notification arrives
  [o music] [===notification pill===]

State 2: Notification dismissed (5s timeout)
  [===music pill===]  (o hidden behind)
```

### Example: Music + Two Notifications

```
State 0: Music pill
  [===music pill===]

State 1: Notification A arrives
  [o music] [===notif A pill===]

State 2: Notification B arrives (different app)
  [o music] [o notif A] [===notif B pill===]

State 3: Notification B dismissed
  [o music] [===notif A pill===]

State 4: Notification A dismissed
  [===music pill===]
```

### Stack Rules

- The **rightmost** activity is always the expanded one (the most recent arrival).
- Activities to the left are minimal circles, ordered by arrival time (oldest on the left).
- When the expanded activity is dismissed, the one immediately to its left expands.
- `max_visible_activities` (config) limits how many circles + pill are shown. Overflow activities are hidden with a badge counter.

### Animation Choreography

All transitions use spring-animated surface style transactions.

**New arrival:**
1. Current pill shrinks (size only, in place) → completion callback
2. Current circle slides left + new circle slides in from right (simultaneous)
3. New circle expands to pill

**Dismissal:**
1. Dismissed pill shrinks to circle → slides right + fades out → completion callback
2. Previous circle expands to pill

### Implementation Change

This model requires **N+1 subsurfaces** instead of exactly 2 — one for each visible activity slot plus one for arrivals/departures. Each activity is assigned a surface and keeps it for its lifetime. Surfaces are recycled when activities are dismissed.

Alternatively, keep 2 surfaces but treat them as "expanded slot" and "circle slot" — on arrival, the circle slot shows the old activity shrinking while the expanded slot shows the new one. This matches the current architecture.

### Open Questions

- Should N be configurable or always 1?
- When N>1, how should the expanded activities be arranged? Side by side as pills?
- Should clicking a circle expand it (and shrink the current pill)?
- Should the stack scroll/wrap when there are many activities?

---

## Open Questions

1. **Keyboard navigation:** Should there be a global shortcut to focus the island?
2. **Theming:** Should the island inherit from a global theme, or have its own style overrides?
3. **Drag-and-drop:** Should files be droppable onto island activities?
4. **Multi-output:** How should the island behave across multiple outputs?
5. **Persistence:** Should activity state survive compositor/island restarts?
6. **Notification grouping:** Should multiple notifications from the same app_id be grouped/replaced or stacked?
