# Dynamic Island

**Status:** draft  
**Related specs:** (none yet — may interact with dock, notifications)

## Summary

The Dynamic Island is a persistent, morphing UI element anchored to the top-center of the screen. It hosts **activities** — live, contextual surfaces provided by Wayland clients through a custom protocol (`otto-island-v1`). The compositor owns the island's visual chrome; clients provide content.

### Terminology

| Term | Definition |
|------|-----------|
| **Island** | The compositor-rendered UI element (pill shape, blur, shadow). Manages layout and stacking of activities. |
| **Client** | A Wayland client registered with the island via `app_id`. Clients are the natural grouping unit — like an app with multiple windows. |
| **Activity** | An individual surface pushed to the island by a client. The atomic unit of content. A client may have one activity (media player: "now playing") or many (notification daemon: one per notification). |
| **Live** | An attribute of an activity. A live activity is actively updating its content (e.g., elapsed time, download progress). The compositor may show a live indicator. A non-live activity is static or transient. |

#### Presentation Sizes

| Size | Description |
|------|-------------|
| **Minimal** | Just the activity icon. Used when the activity is in the background and others have focus. |
| **Compact** | Small pill — icon + one line of text or a progress indicator. The default resting state. |
| **Expanded** | Larger area showing the client-provided surface with full interaction and keyboard focus. |
| **Banner** | Full-width momentary takeover for transient activities. Auto-dismisses after timeout. |

## Goals

- Provide a single, consistent location for live status, notifications, and contextual controls.
- Allow any Wayland client to publish activities to the island through a well-defined protocol.
- Support multiple concurrent activities, sorted by time and priority, with graceful transitions between presentation sizes.
- Allow activities to embed actual Wayland surfaces (mini-apps) inside the island, with compositor-enforced size constraints.
- Support rich interaction: click-to-expand, context menus, drag-and-drop onto activities, swipe/scroll navigation, hover preview.
- Use a permission model (configurable allowlist) so the user controls which apps can use the island and what they are allowed to do.
- Dogfood the protocol: all built-in island features (clock, media controls, system status) use the same protocol as third-party clients.

## Non-Goals

- Full window management — the island is not a tiling/floating container. Activities have constrained sizing.
- Replacing the notification daemon entirely — the island is a *presentation surface* for activities; notification routing/filtering is a separate concern.
- Defining the visual design language (colors, typography, animations) — that is a theming concern.

## Behavior

### Island Element

1. The island is a compositor-rendered, pill-shaped element displayed on the primary output.
2. **Default position:** horizontally centered, anchored to the top edge of the screen, with a small vertical offset (configurable).
3. **Draggable:** the user may drag the island horizontally along the top edge. An optional configuration allows free placement anywhere on screen.
4. The compositor renders the island's chrome: pill shape, background blur, rounded corners, drop shadow. Client surfaces are inset within this chrome.
5. The island morphs (animates size and shape) when activities change presentation size or when activities are added/removed.

### Activities

An **activity** is to the island what a **window** is to the workspace. It is an independent unit of content published by a client. A single client can create multiple activities, just as a single application can open multiple windows. The compositor groups activities by `app_id` and manages their layout — clients do not control positioning or stacking.

Each activity has:

| Property | Type | Description |
|----------|------|-------------|
| `app_id` | string | Identifies the owning application. Activities sharing an `app_id` are grouped. |
| `activity_id` | uint | Unique handle for this activity instance. |
| `priority` | enum | `low`, `normal`, `high`, `critical`. Determines sort order and interruption behavior. |
| `title` | string | Short label (e.g., "Now Playing", "Download 3/5"). |
| `icon` | buffer/surface | Small icon for minimal/compact representation. |
| `surface` | wl_surface | The activity's surface. The client provides one surface and redraws it when the compositor sends a `configure` event. |
| `preferred_size` | enum | Hint: `minimal`, `compact`, or `expanded`. The compositor may assign a different size. |
| `timeout` | int | Auto-dismiss timeout in ms. `0` = persistent (no auto-dismiss). `> 0` = transient. |
| `live` | bool | Whether the activity is actively updating. The compositor may show a live indicator (e.g., pulsing dot). |
| `exclusive` | bool | When `true`, the island is locked to this activity — all others are hidden and incoming activities queue silently. Requires `island.exclusive` permission. |

Activities are either **persistent** (`timeout = 0`) or **transient** (`timeout > 0`). This distinction drives their lifecycle in the island layout (see Layout Model below).

#### Exclusive Mode

When an activity sets `exclusive = true`:

1. The island shows **only** that activity — all other compact/minimal activities are hidden (not dismissed, just temporarily invisible).
2. Incoming activities are **queued** — they do not banner or push into slots. The badge counter still increments for transient arrivals.
3. When the exclusive activity is destroyed or sets `exclusive = false`, the island restores the previous layout and any queued banners play in sequence.
4. Only one activity can be exclusive at a time. If a second exclusive activity arrives, the compositor rejects it with a `permission_denied` event (first-come-first-served).

Exclusive mode is intended for short-lived, security-sensitive or high-focus UI (fingerprint auth, screen lock, voice assistant listening). It requires the `island.exclusive` permission.

The compositor decides which presentation size to use for each activity (minimal, compact, expanded, banner) based on priority, recency, and user interaction. Size negotiation follows the Wayland configure pattern:

#### Configure / Ack Flow

Size negotiation follows the Wayland configure pattern (like `xdg_toplevel.configure`):

1. Compositor sends **`configure(serial, mode, width, height)`**:
   - `mode`: presentation mode — `minimal`, `compact`, `expanded`, `banner`.
   - `width, height`: pixel dimensions for the surface.
2. Client must **`ack_configure(serial)`** and commit a new surface buffer matching the given dimensions, with content appropriate for the mode.
3. Until the client acks, the compositor may show the previous buffer (scaled/cropped) or a placeholder.
4. The compositor may send a new `configure` at any time (layout change, user interaction, output scale change).

The mode fully describes the interaction state — there is no separate hover flag. Hovering over a minimal activity promotes it to compact. Clicking promotes to expanded.

Example — minimal → hover → click → leave:

```
compositor → client:  configure(serial=1, minimal, 32, 32)
client:               [draws app icon at 32x32]

[user hovers over the minimal activity]
compositor → client:  configure(serial=2, compact, 320, 48)
client:               [draws compact content — title + one-liner]

[user clicks]
compositor → client:  configure(serial=3, expanded, 400, 200)
client:               [draws full interactive surface]

[user clicks away]
compositor → client:  configure(serial=4, minimal, 32, 32)
client:               [back to icon]
```

#### Activity Grouping

Activities from the same `app_id` form a **group**. Grouping affects layout:

- In the **primary slot**, only one activity from a group is shown at a time (the newest or highest-priority within the group).
- In the **dismissed stack**, activities from the same group are shown together under a group header (the `app_id`'s name and icon).
- The island's badge counter reflects total dismissed activities across all groups.

This mirrors how a taskbar groups windows by application.

The compositor sends a `resize` event to the activity when its presentation size changes, including the available pixel dimensions. The client must respect these dimensions for its surface.

#### Activity Lifecycle

1. Client binds `otto_island_manager_v1` global.
2. Client calls `create_activity` with `app_id`, `priority`, `title`, `preferred_size`, and `timeout` → receives an `otto_island_activity_v1` object.
3. Compositor sends an initial **`configure(serial, size, width, height)`**.
4. Client **`ack_configure(serial)`**, attaches a `wl_surface`, and commits a buffer matching the dimensions.
5. Client updates activity state as needed: `set_title`, `set_icon`, `set_priority`, `set_progress`, `set_metadata`.
6. Compositor sends new `configure` events when layout changes (slot shifts, user expand/collapse, output scale change).
7. Compositor sends other events: `focus` / `unfocus`, `drop` (file dropped onto activity), `dismissed` (user or timeout dismissed it).
8. Client destroys the activity with `destroy` when done, or the compositor may revoke it.
9. A client may have many activities alive simultaneously — each is independent.

#### Island Layout Model

The island manages activities the way a workspace manages windows — the compositor controls placement, stacking, and transitions; clients just provide content.

Unlike mobile (which limits to ~2 visible activities), the desktop island can show **multiple activities side by side** at compact size. The number of visible compact slots is configurable:

```toml
[island]
max_visible_activities = 3   # how many activities shown at compact size simultaneously
```

The island has these layout tiers:

- **Compact slots** — up to `max_visible_activities` activities shown at compact size, arranged horizontally in the pill. Sorted by priority then recency.
- **Minimal overflow** — activities beyond the compact limit are shown as minimal (icon-only) indicators at the edges of the pill.
- **Expanded** — when the user clicks a compact activity, it expands. Other compact activities shrink to minimal to make room.

This means on a wide desktop screen, a user could see their media player, a file transfer, and a timer all at compact size simultaneously — no cycling needed.

#### Banner Behavior

A banner does **not** float separately — it **pushes into the compact slots**. When a transient activity arrives as a banner:

1. It takes the **first compact slot**.
2. Existing compact activities shift right. The last compact activity overflows to **minimal**.
3. After the banner phase (auto-dismiss timeout), the activity either:
   - **Stays in its compact slot** if its priority is high enough to hold a slot — other activities shift back.
   - **Moves to the dismissed stack** if it is low priority or the slots are full of higher-priority activities.

This means a banner is not a separate layout mode — it is a compact-sized activity with an entrance animation (the morph/expand effect) that settles into the normal slot flow.

#### Activity Arrival & Slot Assignment

When a new activity arrives:

1. It enters as a **banner** (transient) or **compact** (persistent), pushing into the first compact slot.
2. Existing compact activities shift right. If this overflows past `max_visible_activities`, the last activity moves to **minimal**.
3. If all compact slots are full of higher-priority activities, the new activity enters directly as **minimal**.
4. Transient activities from the **same `app_id`** replace each other in-place — the newest one takes the slot. The replaced activity moves to the dismissed stack.

#### Dismissed Stack & Badge

- When a transient activity (banner) auto-dismisses after its timeout, it enters the **dismissed stack** — an ordered list of activities that are no longer visible but not yet acknowledged by the user.
- The island displays a **badge counter** showing the number of items in the dismissed stack (e.g., "3").
- The badge is visible on the island chrome itself, not attached to any specific activity.
- When the user clicks the badge (or the island when a badge is present), the island **expands to show the dismissed stack** — the compositor lays out the individual activity surfaces in a vertical scrollable list.
- The user can dismiss individual items or clear all. Dismissing removes the activity entirely (compositor sends `dismissed` event to the client).
- When the dismissed stack is empty, the badge disappears.

#### Multiple Persistent Activities

- Persistent activities coexist in compact slots up to the configured limit.
- Beyond the limit, lower-priority activities appear as minimal indicators.
- The user can swipe/scroll horizontally to rotate which activities occupy the compact slots.
- Persistent activities are sorted by: **priority** (descending), then **last interaction time** (most recent first).

#### Example Flow (with `max_visible_activities = 3`)

1. **Media player** is in slot 1 (compact). **Timer** is in slot 2 (compact). Slot 3 is free.
2. **Notification 1** arrives (transient, `normal` priority). It pushes into slot 1 as a **banner**. Media player shifts to slot 2, timer shifts to slot 3. All fit.
3. Notification 1's banner animation finishes → it settles as **compact** in slot 1.
4. **Notification 2** arrives (transient, same `app_id`). It **replaces** notification 1 in slot 1 (same client). Notification 1 moves to the **dismissed stack**. Badge shows **"1"**.
5. Notification 2 auto-dismisses → moves to dismissed stack. Badge shows **"2"**. Media player shifts back to slot 1, timer to slot 2.
6. **Download activity** arrives (persistent, `normal`). Takes slot 3. Island now shows: media player | timer | download — all compact.
7. **Critical alert** arrives (transient, `critical`). Pushes into slot 1 as a banner. Media player → slot 2, timer → slot 3, download → **minimal**. 
8. User clicks the badge → island expands vertically showing the 2 dismissed notification surfaces, laid out by the compositor.

### Permissions

- The compositor maintains a per-`app_id` allowlist stored in configuration.
- Permissions are fine-grained:

| Permission | Controls |
|------------|----------|
| `island.show` | Can the app create activities at all? |
| `island.surface` | Can the app embed a Wayland surface (mini-app)? |
| `island.critical` | Can the app use `critical` priority (interrupting)? |
| `island.exclusive` | Can the app lock the island in exclusive mode? |
| `island.drop_target` | Can the app receive drag-and-drop file drops? |

- If a client attempts an action it lacks permission for, the compositor ignores the request and sends a `permission_denied` event.
- When an unknown `app_id` first requests island access, the compositor may prompt the user (implementation-defined).

### Interactions

| Interaction | Behavior |
|-------------|----------|
| **Click / Tap** | Expands the activity under the cursor to **expanded** size. If already expanded, collapses to **compact**. |
| **Long press / Right-click** | Opens a compositor-rendered context menu with actions: dismiss activity, pin/unpin, open app, manage permissions. |
| **Hover** | Shows a preview tooltip (compact → expanded preview) after a short delay. |
| **Swipe / Scroll** | Navigates between activities when multiple are present. |
| **Drag (on island itself)** | Repositions the island (horizontal by default, configurable to free). |
| **Drag files onto activity** | If the activity has `drop_target` permission, the compositor sends a `drop` event with the file descriptors / MIME types. The activity's surface receives focus during drag-over. |

### Configuration

```toml
[island]
enabled = true
position = "top-center"       # "top-center" | "top-left" | "top-right" | custom coords
draggable = "horizontal"      # "horizontal" | "free" | "none"
offset_y = 8                  # vertical offset from screen edge in logical pixels
max_visible_activities = 3    # how many activities shown at compact size simultaneously

[island.permissions]
# Per-app permission overrides
# Unlisted apps use the default policy
default_policy = "prompt"     # "allow" | "deny" | "prompt"

[island.permissions.apps."com.spotify.Client"]
show = true
surface = true
critical = false
drop_target = false

[island.permissions.apps."org.otto.MediaControls"]
show = true
surface = true
critical = true
drop_target = false

[island.permissions.apps."org.otto.FingerprintAuth"]
show = true
surface = true
critical = true
exclusive = true
drop_target = false
```

### Use Case: Fingerprint Authentication

A fingerprint auth daemon (`org.otto.FingerprintAuth`) uses the island to show a brief authentication animation:

1. PAM or a polkit agent triggers fingerprint auth.
2. The daemon creates an **exclusive**, **live**, **critical** activity with a short timeout (e.g., 10s fallback).
3. The island locks — all other activities hidden, incoming banners queued.
4. The surface shows: fingerprint icon → scanning ripple animation → ✓ success (green) or ✗ failure (red).
5. On success/failure, the daemon destroys the activity after a brief result animation (~500ms).
6. The island unlocks — previous layout restores, any queued banners play.

## Constraints & Edge Cases

- **Multi-output:** the island appears on the focused output. When focus moves between outputs, the island follows (animated transition).
- **Fullscreen windows:** the island hides when a window is fullscreen, unless an activity with `critical` priority is active.
- **Scaling:** all island dimensions are in logical pixels; the compositor converts to physical pixels per output scale.
- **Surface misbehavior:** if a client surface exceeds its allocated size, the compositor clips it. If a client stops responding, the compositor shows a placeholder and marks the activity as unresponsive after a timeout.
- **Activity limit:** the compositor enforces a maximum number of concurrent activities per client (configurable, default 5) and globally (default 20).
- **Island minimum size:** even with zero activities, the island may remain visible as a minimal pill (showing clock or system indicators), or hide entirely — configurable.

## Rationale

- **Compositor-owned chrome:** ensures visual consistency regardless of client quality. Clients focus on content, not window decoration.
- **Protocol-driven built-ins:** guarantees the protocol is expressive enough for real use cases and prevents a two-tier system where built-in features have unfair advantages.
- **Priority + time sorting:** balances urgency with fairness — critical alerts surface immediately, but old low-priority items don't permanently occlude newer ones.
- **Fine-grained permissions:** the island is prime screen real estate and can interrupt the user. Granular control prevents abuse (e.g., ad-like notifications from untrusted apps).
- **Draggable position:** desktop users have diverse workflows; a fixed position may conflict with other UI elements or preferences.

## Open Questions

1. **Persistence across restarts:** should activity state survive compositor restarts (e.g., serialize active activities)?
2. **Keyboard navigation:** how should keyboard focus interact with the island? Should there be a global shortcut to focus it?
3. **Theming integration:** should the island chrome inherit from a global theme, or have its own style overrides?
4. **Animation specification:** should the spec define morph animation durations/curves, or leave that to implementation?
5. **Protocol versioning:** start at v1 — what is the minimal v1 surface (perhaps without mini-app surfaces) vs. a fuller v2?
6. **Notification integration:** should the island consume `org.freedesktop.Notifications` D-Bus calls and present them as banner activities automatically?
7. **Multi-seat:** how does the island behave in multi-seat configurations?
