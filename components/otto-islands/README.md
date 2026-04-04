# Otto Islands

A standalone otto-kit application that renders notification groups and live activities (music, timers) as a morphing pill-shaped UI anchored to the top-center of the screen.

## Architecture

### Subsurface Model (CALayer-style)

Every visual element is its own **Wayland subsurface**, composed like Core Animation layers. The parent layer shell surface is a transparent container — all rendering happens in child subsurfaces.

```
Layer shell surface (480x400, transparent, Overlay layer)
  |
  +-- pill subsurface (group tab: icon + app name + count + chevron)
  +-- pill subsurface (another group)
  +-- card subsurface (notification 1)
  +-- card subsurface (notification 2)
  +-- ...
```

Each subsurface is a `SubsurfaceSurface` from otto-kit with:
- Its own Skia buffer for content rendering (460x100 logical pixels, 2x HiDPI)
- Independent size, position, corner radius, opacity via `otto-surface-style-v1`
- Spring-animated transitions on all properties
- Center anchor point (`set_anchor_point(0.5, 0.5)`) so all coordinates are center-based

### Surface Lifecycle

**Pill surfaces** are created when a notification group first appears and destroyed (deferred) when the group is dismissed.

**Card surfaces** are created lazily when the stack opens and **reused across open/close cycles**:
- On close: cards animate out (fade + slide up) but surfaces stay alive
- On reopen: same surfaces are repositioned and faded back in
- On notification dismiss: that card surface is deferred-destroyed (0.8s delay for animation)

### Surface Style Protocol

All visual chrome is handled by the compositor via `otto-surface-style-v1`:

```rust
// Island pill style
ss.set_background_color(0.03, 0.03, 0.03, 1.0);  // near-black
ss.set_corner_radius(radius * BUFFER_SCALE);
ss.set_masks_to_bounds(ClipMode::Enabled);
ss.set_shadow(0.2, 2.0, 0.0, 8.0, 0.0, 0.0, 0.0);
ss.set_blend_mode(BlendMode::BackgroundBlur);      // frosted glass
ss.set_contents_gravity(ContentsGravity::Center);
ss.set_anchor_point(0.5, 0.5);                     // center anchor

// Card style uses theme material colors
ss.set_background_color(r, g, b, a);  // from theme.material_medium
```

The app draws flat content with Skia. The compositor handles shape, blur, shadow, and spring animations.

### Animations

All animations use spring-animated transactions:

```rust
// Example: animate to new position + size
let timing = scene.create_timing_function(qh, ());
timing.set_spring(0.25, 0.0);  // damping, stiffness
let anim = scene.begin_transaction(qh, ());
anim.set_duration(0.6);
anim.set_delay(delay);
anim.set_timing_function(&timing);
scene_surface.set_size(w, h);
scene_surface.set_position(x, y);  // center coords with anchor 0.5,0.5
anim.commit();
```

Key animation helpers in `renderer.rs`:
- `animate_to()` — spring position + size + corner radius
- `animate_position_opacity()` — spring position + opacity only (size set instantly)
- `animate_pulse()` — instantly grow, spring back to target (bump effect)
- `animate_dismiss()` — scale up 1.2x + fade out (card dismiss)

### Content Rendering

Content is drawn with Skia into the subsurface buffer using `draw_centered()`:

```rust
draw_centered(&surface, content_w, content_h, |canvas| {
    // Draw at (0,0). draw_centered translates to center in the buffer.
    renderer::draw_pill(canvas, app_id, icon, count, expanded, w, h);
});
```

The buffer is larger than the content (460x100) so content can be pre-drawn at full size. The compositor reveals it via spring-animated bounds.

### Input Region

The Wayland input region controls which areas receive pointer events. It must be updated on every layout change:

```rust
// Per-island rect (exact size/position per mode)
region.add(x, y, w, h);  // top-left coords, not center

// Card stack: one continuous rect covering all cards + gaps
region.add(card_x, stack_top, card_w, stack_h);
```

The region uses **top-left coordinates** (standard Wayland), not center coords. The layer surface is fixed at 480x400 so the region is never clipped by surface bounds.

### Z-Order

Subsurface stacking is controlled by `place_above`/`place_below`:

```rust
card.surface.place_below(pill.surface.wl_surface());
```

Cards are placed below the pill so the pill always renders on top of the stack.

### Hit Testing

`hit_test(px, py)` mirrors the layout math to determine what's under the pointer:
- Returns `(app_id, None)` for pill/circle hits
- Returns `(app_id, Some(activity_id))` for card hits
- `island.cards` vec is kept sorted to match layout order

The hit test uses **top-left coordinates** (Wayland pointer position), while layout uses center coords for animations. The hit test computes top-left bounds from the centered layout.

## Island Modes

Each island cycles through three modes:

| Mode | Visual | Size |
|------|--------|------|
| **Mini** | Small pill with icon + count | 44x28 (MINI_W x MINI_H) |
| **Compact** | Full pill with icon + name + count + chevron | dynamic width x 36 |
| **Expanded** | Compact pill + card stack below | max(pill_width, 300) x 36 |

Click cycle: **Mini -> Expanded -> Compact -> Expanded -> ...**

### Focus Rules

- Only 1 island can be Compact/Expanded at a time
- New notifications auto-focus their island
- After 2s of no interaction, focused island shrinks to Mini
- Focus loss (keyboard leave) closes expanded stack with 0.5s delay to Mini

### Peek Animation

When a new notification arrives in a Mini group:
1. Pulse (bump +6px, spring back)
2. Animate to Compact size (show content)
3. After 3s, spring back to Mini

## File Structure

- `main.rs` — IslandApp, Island/CardSurface structs, layout, hit testing, pointer events
- `renderer.rs` — Skia drawing (pills, cards, badges), animation helpers, surface style setup
- `activity.rs` — Activity data model, PresentationMode enum
- `state.rs` — SharedState with notification grouping, CRUD operations
- `notifications.rs` — org.freedesktop.Notifications D-Bus daemon
- `dbus_service.rs` — org.otto.Island1 custom D-Bus API
- `music.rs` — MPRIS/PipeWire music integration

## Running

```sh
# Run otto compositor first
cargo run -- --winit &

# Then run islands
WAYLAND_DISPLAY=wayland-1 cargo run -p otto-islands

# Send test notifications
notify-send -a "Firefox" "New Tab" "You opened a new tab"
notify-send -a "Firefox" "Download" "file.zip completed"
```
