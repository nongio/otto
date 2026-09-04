# Portal Access Dialog

**Status:** draft
**Related specs:** [notification-island](./notification-island.md), [dynamic-island](./dynamic-island.md)

## Summary

A generic permission/choice dialog for Otto, modeled 1:1 on the freedesktop
`org.freedesktop.impl.portal.Access` interface. `otto-portal` owns the request
and decides which registered renderer presents it; `otto-islands` is the first
(and default) renderer. One primitive — "title/subtitle/body/icon + optional
list(s) of choices → response + selection" — serves every case: screenshare
permission, screencast output selection, RDP output, and AirPlay receiver
selection.

## Goals

- `otto-portal` exposes `org.freedesktop.impl.portal.Access` at
  `/org/freedesktop/portal/desktop` (bus name `org.freedesktop.impl.portal.desktop.otto`),
  so real xdg-desktop-portal Access requests (Flatpak/browser screensharing
  permission, camera, location) route through Otto's own UI.
- `otto-portal` also accepts *internal* Otto callers (compositor, otto-rdp,
  AirPlay, its own ScreenCast backend) via the same contract, so those flows
  present the identical dialog.
- The dialog contract matches `impl.portal.Access` field-for-field:
  `(handle, app_id, parent_window, title, subtitle, body, options) → (response, results)`,
  with `response ∈ {0 = granted/confirmed, 1 = cancelled, 2 = other}` and
  `options.choices` carrying selectable lists.
- `otto-portal` selects a renderer from a set of registered dialog clients;
  when none is available the request resolves to a safe default (deny / cancel),
  never a hang.
- `otto-islands` presents the dialog as an interactive on-top island panel and
  returns the user's decision.
- Cancellation is bidirectional: the caller can withdraw a pending request, and
  the user can dismiss the dialog.

## Non-Goals

- The visual *source* picker for ScreenCast/RemoteDesktop `SelectSources`
  (window thumbnails, region selection). This spec covers list-of-choices
  selection only; a thumbnail picker is a separate future surface.
- Multi-renderer arbitration policy beyond "first available registered client".
  Ranking/user-preference between renderers is out of scope for v1.
- Persisting past grants (the portal's `remember`/token machinery). The dialog
  reports one decision; whether to cache it is the caller's concern.

## Behavior

### Roles

- **Caller** — any of: the real `xdg-desktop-portal` frontend (forwarding an
  app's Access request), the compositor, otto-rdp, the AirPlay bridge, or
  otto-portal's own ScreenCast backend.
- **Broker** — `otto-portal`. Receives the request, chooses a renderer, relays
  the result. Holds all policy about which renderer to use.
- **Renderer** — a registered dialog client. `otto-islands` is the default.

### Request contract (caller → broker)

A request carries:
- `app_id` — the requesting application's identifier (may be empty for system).
- `parent_window` — opaque parent handle (may be empty).
- `title`, `subtitle`, `body` — text. `title` is required; the others optional.
- `icon` — themed icon name or empty.
- `modal` — whether the dialog grabs input until answered (default true).
- `grant_label`, `deny_label` — confirm/cancel button text (defaults:
  "Allow" / "Deny", or "OK" / "Cancel" when there are no choices to grant).
- `choices` — zero or more choice groups. Each group has an `id`, a `label`, a
  list of options `(option_id, option_label, option_icon)`, and an optional
  `default` option_id. A group with options renders as a single-select list; a
  group with no options renders as a boolean toggle (matching Access
  semantics where an empty choice list means a checkbox).

### Response contract (broker → caller)

- `response` — `0` the user confirmed (granted), `1` the user cancelled/denied,
  `2` the request ended for another reason (renderer died, timeout, withdrawn).
- `results` — a map. For each choice group the user interacted with, the key is
  the group `id` and the value is the selected `option_id` (or `"true"`/`"false"`
  for a boolean group). Absent groups fall back to their `default`.

### Flow

1. Caller invokes the broker's Access method (async; returns only when the user
   answers, the request is withdrawn, or it fails).
2. Broker validates fields, picks the first available registered renderer.
   - If no renderer is available, broker resolves `response = 1` (deny/cancel)
     immediately. It must never block indefinitely waiting for a renderer.
3. Broker relays the request to the renderer and awaits its decision.
4. Renderer presents the dialog:
   - If `modal`, it takes exclusive keyboard focus on an on-top layer and must
     not be occluded or click-through while pending (anti-spoofing).
   - It shows title/subtitle/body/icon and any choice groups as interactive
     controls, plus grant and deny actions.
   - Default selections are pre-highlighted.
5. User confirms → renderer returns `response = 0` with the current selections.
   User cancels / dismisses / presses Escape → renderer returns `response = 1`.
6. Broker returns `(response, results)` to the caller.

### Withdrawal / cancellation

- If the caller withdraws the request (e.g. the freedesktop `Request.Close`, or
  the internal caller drops the call), the broker tells the renderer to dismiss
  the dialog, and the pending call resolves with `response = 2`.
- If the renderer disappears while a request is pending, the broker resolves the
  pending call with `response = 2`.

### Mapping the four launch cases

- **Screenshare permission** — one boolean intent: title "Share your screen?",
  body naming the app; no choice groups (grant/deny only). `response = 0` grants.
- **Screencast output selection** — one single-select choice group `output`
  whose options are the available connectors. Replaces the current
  `~/.config/otto/screencast-output` file override and the "auto-pick first
  output" behavior. `results["output"]` is the chosen connector.
- **RDP output selection** — identical shape, choice group `output`, options are
  the outputs eligible for the RDP session.
- **AirPlay receiver selection** — choice group `receiver`, options are the
  discovered receivers (id = service identifier, label = friendly name).

## Constraints & Edge Cases

- **Anti-spoofing:** a modal grant must be unspoofable — on top, input-grabbing,
  and visually attributable to Otto, not the requesting app. The renderer owns
  this; the broker must set `modal = true` for permission grants regardless of
  caller-supplied value unless the caller is trusted-internal.
- **No renderer running:** deny-by-default, promptly. A screenshare that can't
  prompt must fail closed, not silently grant.
- **Empty choice options:** a choice group whose option list is empty at request
  time (e.g. no outputs available) must resolve as an error/cancel rather than
  presenting an unanswerable list.
- **Concurrent requests:** the renderer serializes dialogs (one active grant at a
  time); queued requests present in arrival order. A pending modal grant blocks
  interaction with lower dialogs but must not deadlock the broker.
- **Field parity with freedesktop Access:** field names and the
  `(response, results)` shape must stay a superset-compatible with
  `org.freedesktop.impl.portal.Access` so external portal Access requests can be
  served without translation loss.

## Rationale

- **Why `impl.portal.Access` and not a bespoke API:** it is the established
  freedesktop pattern for permission + choice dialogs, already carries a
  `choices` mechanism, and matching it means Otto can serve *real* external
  portal Access requests (browser/Flatpak screensharing consent) through the
  same island UI with no adapter.
- **Why the portal brokers instead of callers talking to islands directly:**
  keeps all "which client renders, is one available, fall back how" policy in
  one place (otto-portal), makes renderers swappable, and gives every caller
  (compositor, rdp, airplay, external apps) one contract. otto-islands stays a
  dumb-but-pretty renderer.
- **Why one primitive for four cases:** permission and selection collapse to
  "prompt + optional choices → decision"; four dialogs would duplicate the
  anti-spoofing, focus, and lifecycle logic four times.

## Implementation status

Stages 1–3 implemented (compiling; runtime verification pending):

- **otto-islands** renders dialogs via `org.otto.Dialog1` at `/org/otto/Dialog`
  (`present_access(app_id, title, subtitle, body, icon, grant_label,
  deny_label, modal, choices) → (response, results)`), a typed superset of
  Access. Panel is a modal dropdown below the island bar.
- **otto-portal** exposes `org.freedesktop.impl.portal.Access` (`AccessDialog`)
  and brokers to the renderer, translating `a{sv}` options/results ↔ the typed
  call. Denies if no renderer is reachable.
- **Screencast** `SelectSources` prompts via the renderer (consent + output
  choice); the `~/.config/otto/screencast-output` override now only *skips* the
  prompt for headless/testing.

## Resolved decisions

- Internal broker→renderer interface: a dedicated `org.otto.Dialog1` (not an
  overload of `org.otto.Island1`), with a strongly-typed signature (choices as
  `(id, label, [(id, label, icon)], default)`) rather than `a{sv}`, since both
  ends are owned.
- Renderer registration is **static** for v1 (islands is the hardcoded default);
  the broker connects to the well-known `org.otto.Island` name on demand.
- Internal selection callers (screencast today) call `org.otto.Dialog1`
  directly rather than round-tripping through the portal's own Access interface.

### Fullscreen windows

A fullscreen window normally takes the screen alone: the compositor fades out
the layer-shell top and overlay layers, and — on the udev backend — scans the
window out on the primary plane with all chrome planes dropped. Both would make
the dialog invisible, which is how a screenshare prompt raised behind a
fullscreen capture app went unanswered.

So while a modal dialog is presented, the renderer requests **exclusive**
keyboard interactivity on its overlay layer surface. The compositor treats an
overlay layer surface with exclusive interactivity as a modal prompt on screen
and, for as long as it is up:

- keeps (or brings back) the layer-shell chrome that fullscreen hides, and
  hides it again once the dialog is answered, if still fullscreen;
- forces a full composite for the frame, disabling fullscreen direct scanout
  and plane promotion — the same treatment the lock plane gets.

## Open Questions

- "Trusted-internal" caller distinction: how the broker decides a caller may
  override `modal` or skip the grant UI for selection-only dialogs.
- Whether AirPlay/RDP receiver lists update *live* while the dialog is open
  (discovery is asynchronous) or are snapshotted at request time.
- True modality: the modal input-capture only covers the (non-fullscreen) layer
  rect. A full-screen dim/catch would need the island layer to span the output.
