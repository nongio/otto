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
- `modal` — whether the dialog grabs input until answered (default true). A
  non-modal dialog can be ignored — see [Non-modal dialogs](#non-modal-dialogs).
  otto-agentsd asks non-modal; portal and screencast callers ask modal.
- `grant_label`, `deny_label` — confirm/cancel button text (defaults:
  "Allow" / "Deny", or "OK" / "Cancel" when there are no choices to grant).
- `choices` — zero or more choice groups. Each group has an `id`, a `label`, a
  list of options `(option_id, option_label, option_icon)`, and an optional
  `default` option_id. An option label may carry a description after its first
  line break (`"Postgres\nRelational, battle-tested"`); the renderer draws it
  smaller, under the label. A group with options renders as a single-select list; a
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
   - If not `modal`, it takes the keyboard on arrival without grabbing it,
     catches clicks only on itself, and can shrink out of the way without
     answering.
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
  Access. Panel is a dropdown below the island bar, modal or not per `modal`.
  It also serves `present_question(app_id, title, subtitle, body, icon,
  grant_label, deny_label, open_label, modal, choices) → (response, results)`
  (bus signature `ssssssssba(ssa(sss)s)` → `ua(ss)`) — see
  [Questions](#questions-presentquestion).
- **otto-portal** exposes `org.freedesktop.impl.portal.Access` (`AccessDialog`)
  and brokers to the renderer, translating `a{sv}` options/results ↔ the typed
  call. Denies if no renderer is reachable.
- **Screencast** `SelectSources` prompts via the renderer (consent + output
  choice); the `~/.config/otto/screencast-output` override now only *skips* the
  prompt for headless/testing.

### Questions (`PresentQuestion`)

`org.otto.Dialog1.PresentQuestion` is the same dialog for questions another
app can answer at more length — otto-agentsd uses it so an agent's question can
be picked up in otto-ask. It takes the `PresentAccess` arguments with one more,
`open_label`, after `deny_label`, and shares the implementation: choices,
defaults, modality, queueing and withdrawal behave identically.

Button rules:

- An empty `grant_label` **hides** the grant button (unlike `PresentAccess`,
  where it falls back to "Allow"/"Continue"). Enter then does nothing.
- An empty `deny_label` falls back to "Deny", as in `PresentAccess`.
- An empty `open_label` hides the open button; a non-empty one adds it.

Layout — the grant button is always the default (accent, right of the main
row, confirmed by Enter); the open button never is:

- grant + deny: `[deny][grant]`, as `PresentAccess`.
- grant + deny + open: `[deny][grant]`, then a full-width open button on its
  own row below, drawn borderless with accent-coloured text.
- deny + open: `[deny][open]`, both in the neutral fill.
- deny only: one full-width deny button.

Responses:

- `0` confirmed — `results` carries the selected option per choice group.
- `1` cancelled, denied, dismissed, or Escape.
- `2` ended without an answer (caller withdrew, renderer went away).
- `3` the open button was pressed — `results` is empty and the dialog closes.
  Opening the other app is the caller's job.

`PresentAccess` never returns `3`.

### Several questions and multi-select (`PresentQuestions`)

`org.otto.Dialog1.PresentQuestions` is the question dialog for a whole set of
questions. It takes `PresentQuestion`'s arguments with a `labels` map after
`open_label`, and its choice groups carry a `multi` flag:

```
PresentQuestions(app_id s, title s, subtitle s, body s, icon s,
                 grant_label s, deny_label s, open_label s,
                 labels a{ss}, modal b,
                 questions a(ssba(sss)as)) -> (response u, results a(ss))
```

A question is `(id, label, multi, options, default_option_ids)`, with options
`(option_id, option_label, option_icon)` exactly as in `PresentQuestion`. A
single-select question starts on the first of its defaults; a multi-select one
starts with all of them picked.

`labels` (every key optional) lets the caller name what the renderer would
otherwise have to invent:

| key | meaning |
| --- | --- |
| `next` | the grant button's label on every page but the last |
| `back` | the back button's label, from the second page on |
| `page` | the page counter, with `{current}` and `{total}` |
| `multi-hint` | a line under a multi-select question's label |
| `body-align` | `start` for a left-aligned body (a list), else centred |

**Multi-select.** Its options are toggles, drawn as the same rows: a picked one
takes the accent fill. A click, `Space` on the row the keyboard is on, or its
digit flips one option and leaves the rest; the keyboard moves onto it rather
than moving on. `results` then carries one `(group_id, option_id)` per picked
option, and none at all when the user picked nothing — which is an answer, not
a refusal.

**One question a page.** With more than one question the dialog shows one at a
time: the page counter ("2 of 3") and a back button on the left and right of a
row above the question, the question's own options below, and `[Skip][Next]`
with **Next** becoming the grant label on the last page. `Enter`, the Next
button, or a digit on a single-select question turns the page; `Left`, the back
button, or `Shift+Tab` onto it goes back. The panel resizes to each page with
the usual spring. Tab stops are the page's options, then its buttons. Nothing
is sent until the last page is confirmed, so paging is free.

A single question looks exactly as `PresentQuestion` does; no counter, no back
button, and the grant label throughout.

### Text and height

Nothing the caller sends is cut off. The title, subtitle, body, each group's
label (for a question, the question itself) and each option's label and
description wrap to the panel's width, keeping the caller's own line breaks.
The panel grows to fit, up to 620 points tall. Past that, the text and choices
scroll (pointer wheel or touchpad over the panel) under the button row, which
stays put; a hairline marks the edge. Only button labels are ellipsised.
Line caps bound pathological input (title 3 lines, subtitle 12, body 40, group
label 12, option label 3, description 4); text past a cap ends in an ellipsis.

### Non-modal dialogs

A dialog presented with `modal = false` asks without taking over:

- It opens as the usual panel below the island bar and its input region covers
  only the panel: clicks beside it reach whatever is behind.
- It **takes the keyboard** as it opens, so it can be answered straight away.
  The island layer switches to *exclusive* keyboard interactivity just long
  enough for the compositor to focus it (Otto focuses an overlay surface when
  it switches to exclusive), then back to *on-demand* — once it has the focus,
  or after 400 ms if the focus never comes (a locked session). The focus stays;
  nothing is grabbed. The same happens when it opens again from its circle.
  Notification islands never take the keyboard.
- Keys while it holds the keyboard:
  - **Tab** / **Shift+Tab** rotate through the stops, wrapping around: each
    question's options as one stop, then deny, grant and open. Tab lands on a
    question's selected option.
  - **Up** / **Down** move within the focused question's options, stopping at
    its first and last, selecting the option they land on and scrolling it
    into view.
  - **1**–**9** (top row or keypad) pick that option of the focused question.
    Each option row shows its digit in a badge on its left (the first nine per
    question). With a single question and a grant button, the digit answers
    the dialog at once; with several, it moves on to the next question.
  - **Enter** (or **Space**) presses the focused button; with no button
    focused, **Enter** confirms (when there is a grant button). **Esc** denies.
  - The focused option or button gets a focus ring (accent stroke just outside
    it), drawn only while the panel holds the keyboard and only once a
    navigation key has been pressed. The first such key reveals the ring on
    the current selection without moving it; later ones move it.
- Leaving: an answered dialog (grant, or open in Ask) slings up out of the top
  of the screen — a short draw-back, then it accelerates away, fully opaque.
  A denied, withdrawn or replaced dialog, or one answered while shrunk, fades.
- The dialog has no free-text field: a question that takes typed text (such as
  an agent's "Other" answer) is answered from the Ask window through **open**.
- Clicking an option or button shows the hand cursor over it. Clicking an
  option moves the keyboard to it and hides the ring until a navigation key is
  pressed again.
- It **shrinks into a circle** — a Mini-sized island showing the dialog's icon,
  at the end of the island row, using the same springs an island moves and
  resizes with — when the user moves on:
  - the keyboard focus leaves (a click on another window, or on the desktop:
    the compositor hands the keyboard back from an on-demand overlay surface the
    same way it does from a top-layer panel), or
  - it never got the keyboard and 12 seconds pass without the pointer on it
    (the pointer resting on the panel, or scrolling it, restarts the count).
- Hovering the circle **peeks**: it grows to the Compact island pill — icon and
  the dialog's title — as a hovered island does, with the hand cursor, and
  shrinks back when the pointer leaves. The row makes room for it.
- Shrinking is **not an answer**: the D-Bus call stays pending, and queued
  dialogs behind it keep waiting. Clicking the circle (or its peek) opens the
  panel again, with the keyboard, and it then only shrinks on focus loss.
- While shrunk, keys do nothing, even if the island layer holds the keyboard
  for a notification.
- A non-modal dialog does not claim the modal-overlay treatment below, so it is
  not shown over a fullscreen window. Its brief exclusive request while taking
  the keyboard does count as a modal overlay for that moment.

A modal dialog never shrinks.

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
