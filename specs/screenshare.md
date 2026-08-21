# Screenshare (PipeWire Screencast)

**Status:** draft
**Related specs:** multi-output.md

## Summary

Otto exposes a compositor-side `org.otto.ScreenCast` D-Bus service that a portal backend
(`xdg-desktop-portal-otto`) uses to implement `org.freedesktop.portal.ScreenCast` for
applications (OBS, Chrome, Firefox, etc.). This spec covers two pieces of that path: how
the PipeWire video stream negotiates a DMA-BUF format/modifier with the consuming client,
and how the portal picks which source (an output or a single window) gets shared when a
session starts. Everything else
about the screenshare pipeline (D-Bus service shape, GPU blit, damage metadata) is
documented in `docs/developer/screenshare.md`.

## Goals

- A PipeWire screencast stream must negotiate a DMA-BUF format with any client whose GPU
  video importer requires an explicit (non-implicit) modifier, not only clients that accept
  `DRM_FORMAT_MOD_LINEAR` or implicit/undefined modifiers.
- Every modifier Otto offers a client must be one that the compositor's own renderer/GBM
  device can actually allocate and that the DMA-BUF buffer-params path can describe (single
  plane only).
- The modifier a client actually negotiates must be read correctly and unambiguously; Otto
  must never allocate a buffer in one layout while the client reads it as another.
- A user must be able to choose what a screencast session records — any output, or any
  single toplevel window — from a picker presented at `SelectSources` time.
- A shared window must be captured from its own surfaces, so the stream shows neither
  windows stacked above it nor the desktop behind it, and keeps updating while the window
  is occluded, on another workspace, or minimized.

## Non-Goals

- Live thumbnails in the source picker (the list is text + app icon only; previews are
  future work, most likely via `ext-image-copy-capture-v1`).
- Region capture, and capture of a window's sub-surface or a specific tab.
- Renegotiating the PipeWire stream format when a captured window resizes.
- Negotiating DMA-BUF for pixel formats other than `Argb8888`/BGRA.
- A CPU/SHM fill path for the DMA-BUF format when no GBM device is available beyond the
  existing SHM-only fallback (see Constraints).
- Multiple simultaneous sources in one session (the portal always resolves to a single
  source; see `multiple` handling below).

## Behavior

### DMA-BUF format negotiation (PipeWire stream)

- When a screenshare session starts, Otto builds a list of DRM modifiers to advertise for
  `Argb8888` (SPA `BGRA`): `DRM_FORMAT_MOD_LINEAR` first, followed by every modifier the
  active renderer backend's EGL reports as supported for that format
  (`Backend::get_format_modifiers`).
- Each candidate modifier is filtered by a real allocation test: Otto attempts to create a
  GBM buffer object of the stream's dimensions with exactly that modifier. The modifier is
  kept only if the allocation succeeds **and** the resulting buffer has exactly one plane.
  Modifiers that require multiple planes (e.g. Intel CCS auxiliary-plane compressed
  modifiers) are dropped even though EGL reports them as supported, because the PipeWire
  buffer-params/`add_buffer` mechanism Otto uses can only describe a single-plane buffer per
  DMA-BUF.
- `DRM_FORMAT_MOD_INVALID` is never included in the offered list, even if it appears in the
  EGL-reported set.
- For each surviving modifier, Otto emits one `SPA_PARAM_EnumFormat` pod carrying that single
  modifier as a `Long` value with the `MANDATORY` property flag set — i.e. one fully-fixed
  format pod per modifier, not a single pod listing multiple modifier choices via
  `SPA_POD_PROP_FLAG_DONT_FIXATE`. The pods are emitted in preference order with LINEAR
  always first, followed by the tiled/vendor modifiers in the order EGL reported them.
- If no GBM device is available, or every modifier probe fails, Otto falls back to an
  SHM-only format offer (no modifier property at all).
- When a client selects one of the offered format pods, Otto parses the negotiated
  `SPA_FORMAT_VideoModifier` property:
  - If it is a fixed `Long` value, that value is the negotiated modifier.
  - If it is a `Choice` pod (the client's own proposal, not yet fixated by Otto down to a
    single value), Otto reads the **first/default value** of the choice — that is the value
    PipeWire specifies the client will actually use — and errors out if that choice's child
    pod is not of type `Long`.
  - If the `VideoModifier` property is present in the negotiated param but its value cannot
    be read as either of the above, format negotiation is a hard failure. Otto never falls
    back to assuming `DRM_FORMAT_MOD_LINEAR` in this case.
  - If the `VideoModifier` property is absent entirely, the stream is treated as SHM (no
    DMA-BUF).
  - A negotiated modifier value of `DRM_FORMAT_MOD_INVALID` is accepted and treated as
    "implicit modifier" DMA-BUF, distinct from a parse failure.

### Window identity

- A capturable window is named by its `ext-foreign-toplevel-list-v1` **identifier** — the
  opaque, stable, cross-process handle the protocol defines for exactly this purpose. Otto
  never exposes surface ids, PIDs, or window titles as the addressing key.
- `org.otto.ScreenCast.ListWindows` returns `(identifier, app_id, title)` for every mapped
  toplevel that has a foreign-toplevel handle. A toplevel without one is not capturable.
- `org.otto.ScreenCast.Session.RecordWindow` takes a `window-id` property carrying that
  identifier, and a `cursor-mode` property, mirroring `RecordMonitor`.
- An identifier that no longer resolves (window closed or unmapped between the picker and
  `Start`) fails `RecordWindow` rather than falling back to any other window.

### Portal implementation properties and cursor modes

- `xdg-desktop-portal-otto` exports the `org.freedesktop.impl.portal.ScreenCast` properties
  exactly as the impl portal interface spells them: `AvailableSourceTypes`,
  `AvailableCursorModes`, and **`version`** in lowercase. The same lowercase `version`
  applies to every impl interface Otto exports (e.g. `…impl.portal.Settings`).
- The name matters: xdg-desktop-portal reads `version` from the backend and only binds
  `AvailableCursorModes` through to the client-facing portal when that version is ≥ 2. A
  backend that spells it `Version` is seen as version 0, the frontend's
  `AvailableCursorModes` stays 0, and every `SelectSources` carrying a `cursor_mode` is
  rejected with `Unavailable cursor mode <n>` before it ever reaches Otto.
- Otto advertises `AvailableCursorModes` = `HIDDEN | EMBEDDED` (3). `METADATA` is not
  implemented and is not advertised.
- The frontend binds those properties **once**, when it loads the implementation, and only
  picks `otto.portal` when `XDG_CURRENT_DESKTOP` contains `otto` (its `UseIn=`). A frontend
  that started before the backend claimed its bus name — or before that env reached
  `systemd --user` — keeps reporting 0 and produces the same
  `Unavailable cursor mode <n>` rejection even though the backend is correct. The session
  script restarts `xdg-desktop-portal.service` after the backend is up
  (`portal_frontend_reload` in `scripts/portal.sh`); `scripts/portal-refresh.sh` does the
  same on demand after reinstalling the backend mid-session. Verify with
  `busctl --user get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop
  org.freedesktop.portal.ScreenCast AvailableCursorModes` — it must not be 0.
- Cursor modes keep the portal's bitmask numbering end to end — `HIDDEN` = 1, `EMBEDDED` = 2,
  `METADATA` = 4. The portal forwards the requested value verbatim as the compositor-side
  `cursor-mode` property of `CreateSession` / `RecordMonitor` / `RecordWindow`, and a missing
  `cursor-mode` defaults to `EMBEDDED` on both sides. Only `EMBEDDED` draws the cursor into
  the stream; any other value (including `METADATA`) leaves it out.

### Portal source selection (`SelectSources`)

- `SelectSources` accepts `types` containing `MONITOR` (1), `WINDOW` (2), or both. A request
  for neither is rejected as before.
- On every call the portal asks the compositor for the current sources: `ListOutputs` when
  monitors are allowed, `ListWindows` when windows are allowed. A `ListWindows` failure
  (e.g. an older compositor) degrades to an empty window list rather than failing the call.
- If both lists are empty, the portal returns response `3` (no sources).
- The portal then presents a **single radio list** containing every allowed source: each
  output (labelled `Entire screen` when there is exactly one, else `Screen — <connector>`),
  followed by each window (labelled by title, falling back to app_id, with app_id as the
  icon hint). Monitors and windows are offered together regardless of which `types` bits
  the app set, so the user is never forced back to the app to change the request.
- The list is presented through the `org.otto.Dialog1` renderer (otto-islands), the same
  Access-style permission/choice dialog used by the portal's Access implementation. Option
  ids are namespaced `monitor:<connector>` and `window:<identifier>`.
- The user's answer resolves as:
  - **Chose a source** → stored on the session; response `0`.
  - **Dismissed the dialog** → response `1` (cancelled). Nothing is captured.
  - **No dialog renderer answered on the bus** → the portal falls back to the pre-picker
    behaviour for monitors (override file, else first output) so a session without
    otto-islands still shares a screen. A window is **never** selected on the user's behalf
    in this path; if only windows were available the call returns response `2`.
- `Start` calls `RecordMonitor` or `RecordWindow` according to the stored selection, and the
  resulting portal stream carries `source_type` = 1 or 2 to match.

### Portal bus name ownership

- The backend claims `org.freedesktop.impl.portal.desktop.otto` with
  `ReplaceExisting | AllowReplacement | DoNotQueue`, and only after every interface is
  exported. A running backend that loses the name to a newer instance exits.
- The reason is that the **session bus outlives the graphical session**: a backend left
  running by an earlier login, or one started before an upgrade, otherwise holds the name
  for as long as the user is logged in. Every later instance died on startup while the
  desktop kept talking to the stale one — an installed fix simply never took effect.
- A backend predating this flag still cannot be replaced (replacement needs the current
  owner's consent). Startup then fails with an explicit message naming the conflict rather
  than continuing without the name.

### Session persistence (`restore_data`)

- `AvailableSourceTypes` advertises `MONITOR | WINDOW`. Persistence additionally depends on
  the lowercase `version` property above: the frontend reads a missing property as `0` and
  then gates off everything the spec added after interface version 1, `restore_data`
  (version 4) included.
- When the app asked for persistence (`persist_mode` 1 or 2), `Start` returns `restore_data`
  as `("otto", 1, <a{sv}>)` where the payload carries `source-type` (1 or 2) and `id` (the
  connector name or the foreign-toplevel identifier), plus the effective `persist_mode`. The
  portal keeps no state of its own: the frontend hands the app a token for that tuple and
  gives the tuple back on a later `SelectSources`.
- A `SelectSources` carrying `restore_data` skips the picker when **all** of the following
  hold; otherwise the user is prompted normally:
  - the vendor field is `otto` and the payload version is one this build understands;
  - the source's type is among the `types` the app requested on this call;
  - the source is still available — the connector is still in `ListOutputs`, or the window
    identifier is still in `ListWindows`.
- This is what keeps a single approval from being asked twice. Chrome's own picker creates
  one session to render the preview thumbnail, then closes it and creates a second session
  for the real capture; without a restorable token the second one re-opened the dialog on
  top of an already-running share, and dismissing it tore the share down.

### Window capture

- A window stream renders the window's **own surface tree** (toplevel + subsurfaces, plus
  its popups) into the PipeWire DMA-BUF; it does not blit the composited output. Content
  stacked above the window, the dock/topbar, and the desktop behind it are therefore never
  in the capture.
- The window's geometry origin is placed at (0, 0) of the stream buffer, so client-side
  decoration shadow margins are cropped out.
- The buffer is cleared to opaque black before each frame.
- The stream size starts at the window's geometry in physical pixels, rounded **down to
  even** dimensions, and follows the window when it is resized: the compositor requests the
  new size each frame and the PipeWire thread renegotiates the format, debounced by 250 ms so
  an interactive drag renegotiates once when it settles rather than every frame. Frames
  rendered between the resize and the renegotiation land in the old-size buffers, so the
  window is briefly cropped (grow) or letterboxed (shrink). Renegotiation frees the whole
  buffer set; the compositor only ever blits into a buffer PipeWire currently owns, at that
  buffer's own dimensions.
- With cursor mode `EMBEDDED`, the cursor is drawn into the window stream at window-relative
  coordinates, so it appears only while the pointer is over the captured window, and only
  while that window holds keyboard focus. An unfocused window's stream never shows the
  cursor, even when the pointer passes over it.
- Window streams are serviced from the render frame of whichever output currently hosts the
  window, so moving a window between outputs does not interrupt its stream. They force that
  output to composite (no direct scanout) and to keep painting when otherwise idle, exactly
  as a monitor stream does.
- A window with an active stream is classified `Captured` by the frame-callback throttler:
  it receives frame callbacks at full rate even when occluded, on an inactive workspace, or
  minimized, and is **not** reported as `activated` (it has no keyboard focus).
- A locked session suspends all capture, window streams included.

### Sharing indicator on the titlebar

- A window that is the target of at least one active screencast stream shows a **sharing
  badge** at the trailing end of its server-side titlebar: a tinted pill with a display
  glyph, in the system green, dimmed along with the rest of the bar when the window is
  unfocused. It mirrors the way macOS marks a shared window.
- The badge is drawn by otto-kit's `WindowDecoration` (`sharing: bool`), so an otto-kit
  client drawing its own titlebar renders the identical badge.
- It is a trailing titlebar group, so it reserves its width and a long title is pushed clear
  of it instead of running underneath.
- The badge is not a control: it has no hit region and clicking it drags the window like any
  other empty part of the bar.
- The flag is recomputed on every commit of a decorated window, and pushed to **all** windows
  whenever the set of streams changes (stream started, stream stopped, session destroyed) —
  a window that gains or loses the badge may be idle and never commit on its own.
- Only window streams raise it. A monitor stream leaves every titlebar unmarked.
- Windows with client-side decorations get no badge; the compositor draws no bar for them.

### Output-selection override file

- Before falling back to the default, the portal checks for an override file at
  `$XDG_CONFIG_HOME/otto/screencast-output` (falling back to `~/.config/otto/screencast-output`
  if `XDG_CONFIG_HOME` is unset). The file is read fresh on every `SelectSources` call — there
  is no caching — so a user (or script) can change it between screencast sessions without
  restarting the compositor or the portal.
- The file's contents, trimmed of surrounding whitespace, are treated as a single output
  connector name (e.g. `virtual-1`).
- If the override names a connector present in the current `ListOutputs` result, that output
  is selected and used for the rest of the session.
- If the override file is missing, empty, or names a connector **not** present in
  `ListOutputs`, the portal logs a warning and falls back to selecting the first output from
  `ListOutputs`, exactly as if no override existed.
- If the app requests `multiple` source selection, the portal logs that it is limiting
  selection to a single source and proceeds with the single-choice picker above; it never
  selects more than one source.
- The override only applies to the no-renderer fallback path; when the picker is reachable
  the user's answer wins.

## Constraints & Edge Cases

- **No SHM fallback pods alongside DMA-BUF pods:** when a GBM device is available and at
  least one modifier survives the allocation probe, Otto advertises *only* DMA-BUF format
  pods — it does not also offer a plain SHM pod as an additional fallback choice in the same
  negotiation. A client whose GStreamer pipeline cannot negotiate `DMA_DRM` caps at all (e.g.
  a plain `videoconvert`-based pipeline predating `gst-plugin-pipewire` 1.2's explicit
  DMA_DRM support) currently cannot negotiate a format with Otto and fails outright rather
  than degrading to CPU-copied SHM frames. This is a known limitation; a CPU fill path is
  future work.
- **Single-plane-only modifiers:** any modifier requiring more than one plane (Intel CCS
  compressed render/media modifiers being the concrete case observed) is excluded from the
  offered list regardless of whether the renderer would otherwise prefer it, because the
  DMA-BUF buffer-params mechanism used here has no way to describe multiple planes per
  buffer.
- **Choice-pod modifiers must resolve to the default, not be guessed:** guessing any
  modifier value other than a Choice pod's declared default risks Otto allocating one memory
  layout while the client reads the buffer as another (tiled-read-as-linear), producing
  visibly corrupted frames rather than a clean failure. This is why an unreadable
  `VideoModifier` is a hard error instead of a silent default to LINEAR.
- **Output-selection override has no schema/validation beyond existence:** the override file
  is a single trimmed line with no quoting, comments, or multi-output support; an invalid or
  stale connector name degrades gracefully (falls back to first output with a warning) rather
  than failing the session.
- **The override is now only a fallback:** with the picker in place it applies solely when
  no `org.otto.Dialog1` renderer answers, keeping headless/islands-less sessions working.
- **A resized captured window is not renegotiated:** the stream keeps its original
  dimensions for its lifetime, cropping or letterboxing instead. Renegotiating mid-stream
  would require tearing down and re-announcing the PipeWire format, which many consumers
  handle poorly; a fixed size trades fidelity for a stream that never drops.
- **Window capture bypasses the compositor's effects:** because it renders the client's own
  surfaces rather than the composited scene, rounded corners, shadows, blur and any other
  lay-rs scene decoration are absent from the capture. This is intentional — the consumer
  wants the window's content, not Otto's presentation of it.
- **A captured window is pinned at full frame rate**, which removes the power savings the
  throttler would otherwise get from occlusion/minimization for that one window. This is
  required: the remote viewer sees the window even when the local user does not.
- **The picker is modal and blocking:** `SelectSources` does not return until the user
  answers. An app that times out its portal call will fail the session; this matches the
  behaviour of every other portal implementation's chooser.

## Rationale

- **One fixed-modifier pod per modifier, instead of a single DONT_FIXATE choice pod,** was
  chosen because `gst-plugin-pipewire` >= 1.2 only negotiates DMA-BUF via explicit
  `video/x-raw(memory:DMABuf)` / DMA_DRM caps, and Intel's VAAPI GStreamer importer
  (`vapostproc`) enumerates only Y-tiled RGB DRM formats from its caps — it does not perform
  its own fixation over a `DONT_FIXATE` choice. Offering a single choice pod containing all
  modifiers left `vapostproc`-based consumers unable to find a match and Chrome/WebRTC
  reporting "no more input formats"; offering one fully-fixed pod per modifier lets each
  candidate be evaluated independently by consumers with different negotiation strategies.
- **LINEAR is always probed and offered first** to keep existing, already-working consumers
  (OBS) negotiating exactly the layout they always have; tiled/vendor modifiers are appended
  after it purely as additional options for consumers that need them, not as a replacement.
- **Real GBM allocation as the filter, not a static capability table,** because the only
  ground truth for "is this modifier actually usable for a buffer of this size on this GPU"
  is attempting the allocation; a static list would drift from what the hardware/driver
  combination actually supports.
- **A hard parse error instead of defaulting to LINEAR** was chosen after observing that a
  silent LINEAR default produced tiled-content-read-as-linear corrupted frames when the
  actual negotiated modifier could not be determined — a visible failure (session refuses to
  start) is preferable to a silently wrong render.
- **Re-reading the override file on every `SelectSources` call** avoids requiring a portal
  restart to pick a different output, which matters for the primary use case (switching
  which virtual/monitor output gets captured across ad hoc RDP/screenshare testing sessions)
  where restarting the whole D-Bus session is disruptive.

- **Reusing the Access-style choice dialog for the picker** avoids building a bespoke
  source-picker UI: `org.freedesktop.impl.portal.Access` already defines a labelled radio
  list (`choices`), Otto's `org.otto.Dialog1` mirrors that shape, and otto-islands already
  renders it with app icons. A dedicated picker with live previews can replace it later
  without changing the compositor-side contract.
- **`ext-foreign-toplevel-list-v1` identifiers as the window key** were chosen over any
  internal id because the protocol defines them precisely for naming a window to another
  process, Otto already implements the protocol, and it keeps the door open for a picker
  that enumerates windows itself instead of via `ListWindows`.
- **Re-rendering the window's surfaces instead of cropping the output framebuffer** is what
  makes occluded, off-workspace and minimized capture work at all, and prevents leaking
  whatever happens to be stacked on top of the shared window to the remote viewer — the
  privacy property users assume when they pick "share a window" over "share a screen".

## Open Questions

- Should the SHM fallback path also be re-enabled as an *additional* pod when DMA-BUF pods
  are offered, so a client that can't negotiate `DMA_DRM` caps still gets a (CPU-copied)
  stream instead of failing outright?
- Should the picker show live thumbnails, and if so, does that arrive via
  `ext-image-copy-capture-v1` (picker captures for itself) or a compositor-side thumbnail
  API?
- Should a captured window that is resized renegotiate the stream format rather than being
  cropped, and is any consumer robust enough to make that worthwhile?
- A window being cast now carries a titlebar badge (see *Sharing indicator on the titlebar*).
  Should a monitor cast get an equivalent persistent indicator, and where — the topbar, since
  there is no single window to mark?
