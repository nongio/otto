# Otto Islands

An experimental notification manager for Otto, inspired by the iOS dynamic island. It renders notification groups as morphing pill-shaped surfaces anchored to the top-center of the screen.

![Otto Islands demo](https://private-user-images.githubusercontent.com/31436893/573857630-9fec43ac-3b17-49c5-8a3b-f0f52cc395a2.gif)

Otto Islands is a standalone Wayland app that takes advantage of Otto's **experimental surface style protocol** (`otto-surface-style-v1`). This protocol gives clients direct access to the compositor's animated scene graph — similar to Core Animation on iOS/macOS. The app draws flat content once with Skia, and the compositor handles all animation, clipping, shadows, and blur at display refresh rate.

## Surface Style Protocol

The surface style protocol provides:

- **Animated properties** — position, size, corner radius, opacity, with spring and easing curves, batched in transactions
- **Graphical properties** — background color, shadow, background blur (frosted glass), masks-to-bounds (clipping)
- **Content gravity** — how the client's buffer maps to the surface bounds on screen: resize, aspect fit, aspect fill, center (like `contentsGravity` in Core Animation)
- **Transactions** — group multiple property changes into a single animated transition with shared timing

Because animation runs compositor-side, the app has no render loop. It redraws a buffer only when notification content changes (new text, icon, count). All interpolation between states — spring physics, opacity fades, size morphing — is computed by the compositor every frame.

```rust
// Declare animation target — compositor interpolates
let timing = scene.create_timing_function(qh, ());
timing.set_spring(bounce, initial_velocity);
let anim = scene.begin_transaction(qh, ());
anim.set_duration(0.8);
anim.set_timing_function(&timing);
surface_style.set_size(w, h);
surface_style.set_position(x, y);
surface_style.set_corner_radius(radius);
surface_style.set_opacity(1.0);
anim.commit();  // compositor animates from current → target
```

> **Note:** The surface style protocol is Otto-specific and experimental. Otto Islands only works with the Otto compositor.

## Architecture

### Subsurface Model

Every visual element is its own **Wayland subsurface**, composed like Core Animation layers. The parent layer shell surface is a transparent container — all rendering happens in child subsurfaces.

```
Layer shell surface (480×400, transparent, Overlay layer)
  ├── pill subsurface (group: icon + app name + count)
  ├── pill subsurface (another group)
  ├── card subsurface (notification 1)
  ├── card subsurface (notification 2)
  └── ...
```

Each subsurface has:
- A Skia buffer for content (460×100 logical, 2× HiDPI)
- Compositor-managed size, position, corner radius, opacity via surface style
- Spring-animated transitions on all properties
- Center anchor point — all coordinates are center-based

### Surface Style Setup

```rust
// Island pill
ss.set_background_color(0.03, 0.03, 0.03, 1.0);  // near-black
ss.set_corner_radius(radius);
ss.set_masks_to_bounds(ClipMode::Enabled);         // clip to rounded rect
ss.set_shadow(0.2, 2.0, 0.0, 8.0, 0.0, 0.0, 0.0);
ss.set_blend_mode(BlendMode::BackgroundBlur);      // frosted glass
ss.set_contents_gravity(ContentsGravity::Center);  // buffer centered in bounds
ss.set_anchor_point(0.5, 0.5);                     // transform origin at center
```

The app draws flat content. The compositor adds shape, blur, shadow, and animates between states.

### Content Gravity

The buffer is larger than the visible content (460×100 vs the actual pill/card size). `ContentsGravity::Center` tells the compositor to center the buffer within the animated bounds. As the compositor spring-animates the size, more or less of the pre-drawn buffer is revealed — no client-side redraw needed.

### Surface Lifecycle

**Pill surfaces** are created when a notification group first appears and destroyed (deferred) when the group is dismissed.

**Card surfaces** are created lazily when the stack opens and reused across open/close cycles. On dismiss, the card surface is deferred-destroyed (0.8s delay for animation completion).

### Input Region

The Wayland input region controls pointer events. Updated on every layout change:

```rust
// Per-island rect (exact size/position per mode)
region.add(x, y, w, h);

// Expanded card stack: one continuous rect
region.add(card_x, stack_top, card_w, stack_h);
```

## Island Modes

Each island cycles through three display modes:

| Mode | Visual | Size |
|------|--------|------|
| **Mini** | Small pill with icon (+ count if > 1) | 28×28 or 44×28 |
| **Compact** | Full pill with icon, app name, title | dynamic width × 36 |
| **Expanded** | Compact pill + notification card stack | max(pill, 300) × 36 + cards |

Click cycle: **Mini → Expanded → Compact → Expanded → ...**

### Focus & Timing

- Only one island can be Expanded at a time
- Compact and Expanded islands coexist (e.g. a peeking notification alongside an open stack)
- After 4s of inactivity, focused island shrinks to Mini
- Pointer hover pauses the focus timer; it restarts on leave
- Focus loss closes expanded cards (0.3s slide-up), then Compact → Mini after 2s

### Peek Animation

When a new notification arrives in a Mini group:
1. Pulse (bump +6px, spring back)
2. Animate to Compact (show title preview)
3. After 3s, spring back to Mini

New notifications arriving while an island is Expanded or Compact refresh the content without changing mode.

## File Structure

- `main.rs` — IslandApp, Island/CardSurface structs, layout, hit testing, pointer events
- `renderer.rs` — Skia drawing (pills, cards, badges), animation helpers, surface style setup
- `activity.rs` — Activity data model
- `state.rs` — SharedState with notification grouping, CRUD operations
- `notifications.rs` — org.freedesktop.Notifications D-Bus daemon
- `dbus_service.rs` — org.otto.Island1 custom D-Bus API

## Running

```sh
# Run Otto compositor first
cargo run -- --winit &

# Then run islands
WAYLAND_DISPLAY=wayland-1 cargo run -p otto-islands

# Send test notifications
notify-send -a "Firefox" "New Tab" "You opened a new tab"
notify-send -a "Firefox" "Download" "file.zip completed"
```
