# Screenshare (PipeWire Screencast)

**Status:** draft
**Related specs:** multi-output.md

## Summary

Otto exposes a compositor-side `org.otto.ScreenCast` D-Bus service that a portal backend
(`xdg-desktop-portal-otto`) uses to implement `org.freedesktop.portal.ScreenCast` for
applications (OBS, Chrome, Firefox, etc.). This spec covers two pieces of that path: how
the PipeWire video stream negotiates a DMA-BUF format/modifier with the consuming client,
and how the portal picks which output gets shared when a session starts. Everything else
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
- A user must be able to choose which output a screencast session records without a
  source-picker UI, and change that choice between sessions without restarting anything.

## Non-Goals

- A graphical source-picker UI for choosing the shared output (tracked as future work).
- Negotiating DMA-BUF for pixel formats other than `Argb8888`/BGRA.
- A CPU/SHM fill path for the DMA-BUF format when no GBM device is available beyond the
  existing SHM-only fallback (see Constraints).
- Multi-monitor / multiple simultaneous output selection (the portal always resolves to a
  single output; see `multiple` handling below).
- Window or region capture (`RecordWindow` is unimplemented; only monitor capture is
  supported).

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

### Portal output selection (`SelectSources`)

- On every `SelectSources` call, the portal backend asks the compositor for the current
  output list (`ListOutputs`) and by default selects the **first** output in that list.
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
  selection to a single output and proceeds with the same first-output-or-override
  resolution above; it never selects more than one output.

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
- **This override is a stopgap:** it exists only because there is no interactive
  source-picker in the portal flow yet. It is expected to be superseded by a real UI that
  lets the user choose the output per-session.

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

## Open Questions

- Should the SHM fallback path also be re-enabled as an *additional* pod when DMA-BUF pods
  are offered, so a client that can't negotiate `DMA_DRM` caps still gets a (CPU-copied)
  stream instead of failing outright?
- Should the output-selection override file be replaced by an actual portal-side UI, and if
  so, does the override file stay as a scriptable/testing escape hatch afterward?
