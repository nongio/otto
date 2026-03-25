# Dynamic Island — Implementation Milestones

**Status:** draft  
**Related specs:** dynamic-island, notification-daemon

## Summary

Incremental implementation plan for the Dynamic Island. Each milestone is a self-contained, demoable step. Later milestones build on earlier ones but each delivers visible value.

---

## Milestone 1 — End-to-End Activity Lifecycle

**Goal:** A client pushes an activity into the island, it renders, and it auto-dismisses after a timeout.

**Scope:**

*Island rendering:*
- Compositor renders a pill-shaped element: rounded rectangle, background blur, drop shadow.
- Positioned top-center of the primary output with a configurable vertical offset.
- Read `[island] enabled` and `offset_y` from config.
- Rendered in the scene graph as a new layer above windows, below popups.
- The pill morphs (animates size) to fit the activity surface, and shrinks/hides when empty.

*Protocol:*
- Write `protocols/otto-island-v1.xml` with core interfaces:
  - `otto_island_manager_v1` — global: `create_activity`, `destroy`.
  - `otto_island_activity_v1` — per-activity: `set_title`, `set_surface`, `ack_configure`, `destroy`.
  - Compositor events: `configure(serial, mode, width, height)`, `dismissed`.
- Generate Rust bindings via `wayland_scanner`.
- Register the global in `state/mod.rs`.

*Activity stack:*
- Compositor maintains an ordered list of active activities.
- When an activity is created, it is pushed onto the stack.
- The top activity is shown in the island at compact size.
- When an activity is dismissed (timeout or client destroy), it is removed from the stack. The next activity in the stack (if any) becomes visible.
- If the stack is empty, the island shrinks to its idle state (or hides).

*Configure flow:*
- On creation, compositor sends `configure(serial, compact, w, h)` to the top activity.
- Client acks and commits a surface buffer. Compositor renders it inside the pill.
- Only compact mode for this milestone — no minimal, expanded, or banner yet.

*Timeout:*
- Activity `timeout > 0` starts a timer on creation.
- When the timer fires, compositor sends `dismissed` event and removes the activity from the stack.

*Test client:*
- A minimal Wayland client in `sample-clients/` that:
  - Connects, creates one activity with `timeout = 5000` (5 seconds).
  - Renders a simple colored surface with text (e.g., "Hello Island").
  - Handles `configure`, redraws at the given size.
  - Exits when it receives `dismissed`.

**Delivers:** The complete round-trip: client → protocol → surface in island → timeout → dismissed. Proves the protocol, rendering, and lifecycle work end-to-end.

---

## Milestone 2 — Modes & Interaction

**Goal:** User can interact with activities via pointer, all four modes work.

**Scope:**
- Compositor sends `configure` with correct mode: `minimal`, `compact`, `expanded`, `banner`.
- Client acks and redraws. Compositor waits for ack before showing new buffer.
- Hover over minimal → compositor sends `configure(compact, ...)` (temporary promotion).
- Click on compact → `configure(expanded, ...)`.
- Click outside → activity returns to previous mode.
- Pointer leave → minimal activities return to minimal.
- Input events forwarded to the client surface when expanded.
- Support `preferred_size` hint from client.

**Delivers:** The full configure lifecycle and pointer interaction work.

---

## Milestone 3 — Multiple Activities & Slots

**Goal:** Multiple activities coexist in the island.

**Scope:**
- Read `max_visible_activities` from config.
- Multiple compact slots side by side.
- Activity arrival pushes into slot 1, others shift right.
- Overflow to minimal.
- Same-`app_id` transient activities replace each other.
- Sorting by priority then recency.

**Delivers:** The full layout model works.

---

## Milestone 4 — Dismissed Stack & Badge

**Goal:** Transient activities that auto-dismiss are recoverable.

**Scope:**
- Dismissed activities enter a stack, badge counter shows on the island chrome.
- Click badge → island expands vertically, compositor lays out dismissed activity surfaces.
- User can dismiss individual items or clear all.
- Compositor sends `dismissed` event to clients.

**Delivers:** Notifications don't get lost.

---

## Milestone 5 — Permissions

**Goal:** Per-app permission control.

**Scope:**
- Read `[island.permissions]` from config.
- Enforce `island.show`, `island.surface`, `island.critical`, `island.exclusive`, `island.drop_target`.
- Send `permission_denied` event when a client exceeds its permissions.
- Default policy: `prompt` / `allow` / `deny`.

**Delivers:** Security model in place.

---

## Milestone 6 — Exclusive Mode

**Goal:** An activity can lock the island.

**Scope:**
- `exclusive = true` hides all other activities, queues incoming ones.
- On destroy / release, previous layout restores, queued banners play.
- Only one exclusive activity at a time.

**Delivers:** Fingerprint auth and similar use cases work.

---

## Milestone 7 — Drag & Drop

**Goal:** Files can be dropped onto island activities.

**Scope:**
- Activities with `drop_target` permission receive `drop` events.
- Visual feedback during drag-over (highlight on the activity).
- File descriptors / MIME types forwarded to client.

**Delivers:** The "drop files into island" concept works.

---

## Milestone 8 — Draggable Island

**Goal:** User can reposition the island.

**Scope:**
- Read `draggable` config (`horizontal`, `free`, `none`).
- Drag gesture on the island chrome (not on activity surfaces) repositions it.
- Position persists across sessions (saved to config or state file).

**Delivers:** User can customize island placement.

---

## Future milestones (not scoped yet)

- Notification daemon client
- Media controls client (MPRIS bridge)
- Multi-output support
- Keyboard navigation
- Theming integration
- Animation polish (morph curves, spring physics)
