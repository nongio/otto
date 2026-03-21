# Notification Daemon (Island Integration)

**Status:** draft  
**Related specs:** dynamic-island

## Summary

`otto-notification-daemon` is a standalone Wayland client that bridges the standard Linux `org.freedesktop.Notifications` D-Bus interface to Otto's Dynamic Island protocol. It receives structured notification data from any application, renders each notification as a Wayland surface, and pushes it to the island as a transient activity. The compositor handles layout, stacking, and badge display.

## Goals

- Implement `org.freedesktop.Notifications` (v1.2) so all existing Linux desktop applications can send notifications without modification.
- Map each D-Bus notification to an island activity with an independently rendered `wl_surface`.
- Support notification urgency mapping to island priority levels.
- Support notification actions (buttons) rendered within the notification surface.
- Let the compositor manage stacking, dismissal, and the badge counter — the daemon does not manage layout.

## Non-Goals

- Notification grouping or threading — each notification is an independent activity. The compositor stacks them in the dismissed stack.
- Notification persistence across daemon restarts (defer to a later version).
- Sound playback — the daemon may delegate to an audio service or ignore the `sound-file` hint for now.
- Replacing the island protocol — this daemon is a regular Wayland client; it has no special compositor privileges beyond its permission grant.

## Behavior

### D-Bus to Island Mapping

When the daemon receives a `Notify` call on D-Bus, it:

1. Creates (or updates) an island activity for the notification.
2. Renders a `wl_surface` containing the notification content.
3. Pushes the activity to the island with appropriate priority and metadata.

#### Field Mapping

| D-Bus field | Island activity property | Notes |
|-------------|--------------------------|-------|
| `app_name` | Activity `title` | Used as the heading. |
| `app_icon` | Activity `icon` | Loaded from icon theme or path. |
| `summary` | Surface content (bold title line) | Rendered by daemon into the surface. |
| `body` | Surface content (body text with markup) | Daemon parses `<b>`, `<i>`, `<a>` tags. |
| `actions` | Surface content (button row) | Rendered as interactive buttons in the surface. |
| `hints.urgency` | Activity `priority` | `0` (low) → `low`, `1` (normal) → `normal`, `2` (critical) → `critical`. |
| `expire_timeout` | Activity auto-dismiss timeout | `-1` = daemon default (5s), `0` = no auto-dismiss (persistent), `>0` = ms. |
| `replaces_id` | Updates existing activity | If `replaces_id > 0`, the daemon updates the matching activity instead of creating a new one. |

#### Priority Mapping

| D-Bus urgency | Island priority | Banner behavior |
|---------------|-----------------|-----------------|
| `low` (0) | `low` | Shows as banner, auto-dismisses quickly (3s). |
| `normal` (1) | `normal` | Shows as banner, auto-dismisses at default timeout (5s). |
| `critical` (2) | `critical` | Shows as banner, does **not** auto-dismiss — requires user interaction. |

### Surface Rendering

The daemon renders each notification into its own `wl_surface`:

- **Layout:** icon (left) + title + body (right) + optional action buttons (bottom row).
- **Styling:** the daemon uses a simple, consistent style. The compositor's island chrome (pill, blur, shadow) wraps the surface — the daemon does **not** draw its own background or rounded corners.
- **Transparent background:** the surface has a transparent background so it blends with the island's chrome.
- **Size:** the daemon requests the surface size based on content, but respects the `resize` event from the compositor. Text truncates with ellipsis if the allocated size is too small.

### Notification Lifecycle on the Island

1. D-Bus `Notify` call → daemon creates an activity with a rendered surface → pushed to island.
2. Island shows the activity as a **banner** (primary slot). Previous banner (if any, from the same daemon) moves to the **dismissed stack**.
3. After the timeout, the banner auto-dismisses → enters the dismissed stack → island badge increments.
4. If the user clicks the badge and opens the dismissed stack, the compositor arranges all notification surfaces vertically. Each surface remains interactive (action buttons still work).
5. If the user clicks an action button:
   - The daemon receives the click via Wayland input on its surface.
   - The daemon emits `ActionInvoked` on D-Bus back to the originating application.
   - The daemon destroys the activity (or keeps it if the notification has the `resident` hint).
6. If the user dismisses a notification (swipe, close button, or "clear all"):
   - The compositor sends a `dismissed` event to the activity.
   - The daemon emits `NotificationClosed` on D-Bus with reason `2` (dismissed by user).
   - The daemon destroys the activity.
7. If the originating app calls `CloseNotification`:
   - The daemon destroys the matching activity.
   - The island removes it from the dismissed stack (badge decrements).

### D-Bus Interface

The daemon implements `org.freedesktop.Notifications` at the well-known name `org.freedesktop.Notifications`:

**Methods:**
- `Notify(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout) → uint32 notification_id`
- `CloseNotification(uint32 id)`
- `GetCapabilities() → string[]` — returns: `["body", "body-markup", "actions", "icon-static", "persistence"]`
- `GetServerInformation() → (name, vendor, version, spec_version)` — returns `("otto-notification-daemon", "otto", "0.1", "1.2")`

**Signals:**
- `NotificationClosed(uint32 id, uint32 reason)` — reasons: `1` (expired), `2` (dismissed), `3` (closed by app), `4` (undefined).
- `ActionInvoked(uint32 id, string action_key)`

### Island Permissions Required

The notification daemon registers with `app_id = "org.otto.NotificationDaemon"` and requires:

| Permission | Value | Reason |
|------------|-------|--------|
| `island.show` | `true` | Must create activities. |
| `island.surface` | `true` | Renders notification surfaces. |
| `island.critical` | `true` | Must present critical-urgency notifications. |
| `island.drop_target` | `false` | Not needed. |

This should be pre-configured in the default Otto configuration:

```toml
[island.permissions.apps."org.otto.NotificationDaemon"]
show = true
surface = true
critical = true
drop_target = false
```

## Constraints & Edge Cases

- **Notification flood:** if a single app sends many notifications rapidly (>10 in 5s), the daemon should rate-limit: batch them into a summary notification ("App X: 15 new notifications") rather than creating 15 individual activities.
- **Missing icon:** if `app_icon` is empty or not found in the icon theme, the daemon uses a generic notification icon.
- **Markup sanitization:** the daemon strips unsupported HTML tags from `body`. Only `<b>`, `<i>`, `<u>`, and `<a>` are rendered.
- **`replaces_id`:** when a notification is replaced, the daemon updates the existing activity's surface in-place (no new activity created). If the replaced notification is in the dismissed stack, it stays there with updated content.
- **Daemon crash recovery:** if the daemon restarts, all previous activities are lost. The island clears the badge. Apps may re-send notifications on reconnect (app-side responsibility).
- **Multiple notification daemons:** only one process can own the `org.freedesktop.Notifications` D-Bus name. If another daemon is running, this one fails to start and logs an error.

## Rationale

- **Standalone daemon, not compositor-built-in:** keeps the compositor focused on layout and rendering. The daemon can be replaced or extended independently. It also dogfoods the island protocol.
- **One activity per notification:** gives the compositor full control over stack layout. The daemon doesn't need to implement a notification center UI — it just renders individual notification cards.
- **Transparent surface with no background:** the island's chrome provides the visual container. This ensures all island activities look consistent regardless of which client created them.
- **Rate limiting:** prevents a misbehaving app from flooding the island and making it unusable.

## Open Questions

1. **Inline reply:** should the daemon support an inline reply action? This would require a text input field in the notification surface. The D-Bus spec doesn't natively support this, but some daemons add a `inline-reply` hint.
2. **Notification history:** should the daemon maintain a history beyond the island's dismissed stack? (e.g., a separate notification center panel)
3. **Do Not Disturb:** should the daemon respect a DND mode (suppress banners, queue silently)? Where does DND state live — in the daemon config, in the compositor, or both?
4. **Image hints:** D-Bus notifications can include `image-data` (raw pixels). Should the daemon render these inline in the surface?
5. **Sounds:** should the daemon play notification sounds, or delegate to a separate audio service?
