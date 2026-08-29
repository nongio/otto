# RDP Bridge (otto-rdp)

**Status:** draft  
**Related specs:** multi-output.md, screenshare.md, topbar.md

## Summary

`otto-rdp` is a standalone bridge that exposes an Otto output to a remote
Microsoft Remote Desktop (RDP) client. It captures an output's frames, encodes
them, and serves them over RDP while forwarding the remote client's keyboard and
pointer input back into the compositor. It supports two transport paths: a
hardware H.264 path over RDP's Graphics Pipeline Extension (EGFX, AVC420 codec)
and a legacy raw-bitmap path. By default the bridge chooses the path per client
connection, automatically falling back to bitmaps for clients that cannot do
AVC420; the `--bitmap` flag forces the legacy path for every client.

## Goals

- Let a remote RDP client view and interact with an Otto output (typically an
  interactive virtual output) over a standard RDP connection.
- Default to hardware-accelerated H.264 video so a modern RDP client receives an
  efficient encoded stream rather than raw bitmaps.
- Preserve the previously working raw-bitmap transport as a fallback for
  clients that cannot use H.264 — automatically, per connection, based on what
  the client advertises, or forced for every client via `--bitmap`.
- Forward remote keyboard and pointer input to the captured output so the remote
  session is fully interactive.
- Tell the local user, unmistakably and without configuration, whenever a remote
  party can see the screen — and give them a way to end it.

## Non-Goals

- Runtime transport switching for an already-connected client: the path is
  decided once, from that client's EGFX capability advertisement, when it
  connects, and does not change for the lifetime of that connection.
- Audio redirection, clipboard, or drive/device redirection.
- Multi-monitor RDP sessions: the bridge serves a single output per connection.
- Defining how the compositor produces virtual outputs or paces their frames —
  that is covered by `multi-output.md`.

## Behavior

### Transport selection

- By default the bridge decides per client connection, not once at process
  startup. Each client's transport is chosen from the capabilities it
  advertises on the RDP Graphics Pipeline Extension (EGFX) channel when it
  connects, and does not change for the lifetime of that connection.
- A client that advertises AVC support — a V8.1 capability set with
  `AVC420_ENABLED`, or any V10+ set without the `AVC_DISABLED` flag — is served
  the hardware H.264 path over EGFX (AVC420).
- A client that disables AVC falls back to the bitmap path automatically,
  served over the main RDP channel; its EGFX graphics channel is left idle (the
  bridge sends it no surface creation or `ResetGraphics` commands). All
  AVC-capable capability versions carry an `AVC_DISABLED` flag for this case;
  Microsoft's mobile / Windows App / iOS / Android clients set it because they
  don't implement H.264.
- A client that never opens the EGFX channel at all also falls back to
  bitmaps, after a short grace period.
- When confirming capabilities to a client that disabled AVC, the server
  confirms a matching AVC-disabled capability set (e.g. `V10 { SMALL_CACHE |
  AVC_DISABLED }`) rather than an AVC-enabled one. Confirming AVC-enabled to a
  client that disabled it makes that client close its graphics channel and
  drop the connection, so the confirm must never contradict what the client
  advertised.
- The `--bitmap` flag overrides all of the above: it forces every client onto
  the legacy bitmap path and skips EGFX entirely, regardless of what a client
  advertises. It remains useful as an explicit override, or when the host has
  no working VAAPI encoder.

### H.264 (EGFX / AVC420) path — default

- The served RDP desktop is the captured output's native size, rounded to an even
  width and height (H.264 requires even dimensions). No letterboxing or
  pillarboxing is applied by the bridge; a client whose window differs in size
  scales the image itself.
- Frames are hardware-encoded as an H.264 Annex-B stream (NV12) and delivered to
  the client as AVC420 graphics-pipeline frames.
- The path is used for a client that advertises AVC support (see Transport
  selection above) — for example `mstsc`, or FreeRDP started with
  `/gfx:avc420`.
- A client that does not advertise AVC support is never put on this path: it is
  automatically served the bitmap path instead (see Transport selection above),
  with no operator action required.
- The first frame sent to a client, and the first frame sent after any dropped
  frame, must be a keyframe (IDR); non-keyframes are withheld until the next
  keyframe once the stream is not in a known-good state.
- Whenever a client has nothing decodable — its graphics surface was just
  created and mapped, or delivery is waiting to resume after a drop — the
  bridge asks the encoder for a keyframe immediately rather than waiting for
  the encoder's scheduled one. A connecting client therefore sees the desktop
  without any user interaction. Relying on the encoder's own keyframe interval
  is not sufficient: that interval counts encoded frames, and an idle desktop
  produces frames slowly, so the wait is unbounded in wall-clock time and the
  client stays black until something on screen moves. These requests are
  rate-limited so a long keyframe wait cannot flood the encoder.
- Frame delivery is backpressure-aware: if the client/transport cannot accept a
  frame, that frame is dropped rather than queued, and delivery resumes on the
  next keyframe.

### Bitmap path

- Used whenever a client ends up on bitmaps, whether by automatic per-client
  fallback (see Transport selection above) or because the bridge was started
  with `--bitmap`.
- The served desktop size and any letterbox/pillarbox handling follow the
  existing per-client desktop-size negotiation of the legacy bitmap path,
  unchanged by the addition of the H.264 path.
- The raw capture pipeline behind this path (CPU capture plus box-filter scale)
  starts lazily, only once a client actually falls back to it. A client that is
  served H.264 for the whole session never causes it to start.
- A client subscribing for display updates is sent the most recent captured
  frame right away, when that frame was composed for the desktop size it
  negotiated; it does not wait for the next capture to arrive.
- Capture is rate-limited to a target frame rate, but the first frame composed
  for a newly negotiated desktop layout bypasses that limit — a client that
  just connected has nothing on screen and must not wait out the interval for
  its first picture.

### Input forwarding

- Remote keyboard and pointer input from the RDP client is forwarded into the
  compositor and aimed at the captured output (see `multi-output.md` for how
  absolute pointer input is mapped into a specific output's geometry). This
  behavior is identical for both transport modes.

### Sharing indicator

- While — and only while — a connected client is being served frames, the bridge
  publishes a screen-sharing indicator: a red dot in the desktop's tray with a
  menu that reports what is shared, with whom, and since when, plus a
  `Stop Sharing` action.
- The indicator is not configurable and cannot be turned off. It is a privacy
  signal, so it is present whenever a remote party can see the screen,
  regardless of how the bridge or the bar was started.
- The indicator appears when the client requests display updates, not when its
  TCP connection is accepted: a port scan or an abandoned handshake must not
  claim that someone is watching.
- The indicator disappears when the client disconnects, when the bridge exits,
  and when the bridge dies without cleaning up. A bridge that is listening with
  no client attached shows nothing.
- Several bridges (one per output) each publish their own indicator.
- The indicator is published as a StatusNotifierItem with a `com.canonical.dbusmenu`
  menu — the protocols the top bar already implements for tray icons — rather
  than as an Otto-specific interface, so no bar-side support is needed and any
  SNI host shows it. The full contract is in
  `docs/developer/remote-desktop-indicator.md`.
- `Stop Sharing` terminates the bridge: the client is disconnected, the listening
  port is closed, and the process exits.

### Capture format negotiation

- The bridge subscribes to the output's PipeWire node with a DMA-BUF format offer
  that carries a DRM modifier property, because Otto's stream marks that property
  MANDATORY: an offer with no modifier at all never intersects Otto's and the link
  fails to negotiate.
- The offer accepts both `DRM_FORMAT_MOD_LINEAR` (preferred — linear dmabufs stay
  CPU-mappable, which the capture path relies on) and `DRM_FORMAT_MOD_INVALID`, the
  implicit modifier. A host with no explicit-modifier support (software GL, older
  drivers) advertises only the implicit one, and a LINEAR-only offer never links
  there.

### Configuration

- `OTTO_RDP_FPS` — target capture/encode frame rate. Default is 30 on the H.264
  path.
- `OTTO_RDP_BITRATE` — target H.264 bitrate in kbps.
- `OTTO_RDP_H264_ENCODER` — selects the hardware H.264 encoder element. Default is
  the standard VAAPI H.264 encoder; an alternative low-power encoder may be named
  instead.
- `OTTO_RDP_DUMP` — debug aid: writes the first captured frame to the named path as
  a binary PPM and never writes again, so the capture path can be inspected with no
  RDP client involved.

## Constraints & Edge Cases

- The H.264 path requires a GStreamer runtime with the VA plugin (providing the
  VAAPI H.264 encoder) available on the host running the bridge. This is a new
  runtime dependency introduced with H.264 support; without it the H.264 path
  cannot start and the operator must use `--bitmap`.
- H.264 requires even frame dimensions, so the served native size is rounded down
  (or to the nearest even value) on both axes.
- A client that connects without AVC support is never placed on the H.264
  path, so it does not see a blank/non-updating screen: it is automatically
  served bitmaps instead (see Transport selection). Getting the capability
  confirm wrong here is a correctness hazard, not just a cosmetic one —
  confirming an AVC-enabled capability set to a client that advertised
  `AVC_DISABLED` makes that client close its graphics channel and drop the
  connection outright.
- Dropping a non-keyframe under backpressure or after a client join means the
  remote image is stale until the next keyframe arrives; keyframe cadence
  therefore bounds worst-case recovery latency.
- The indicator needs a StatusNotifierWatcher to be running. If none is (no bar,
  or the bar restarted), the bridge still serves — there is simply nowhere to
  draw the icon — and it re-registers as soon as a host appears. A bridge that
  serves while no host is running therefore shares without a visible indicator;
  this is a property of there being no UI, not of the bridge suppressing the
  signal.
- The indicator's menu is fixed for the lifetime of a session because hosts
  cache the layout at registration. Anything that is not yet known then (the
  negotiated transport codec) is therefore left out rather than shown stale, and
  the session start time is shown as an absolute clock time rather than a live
  elapsed duration.

## Rationale

- Hardware H.264 over EGFX is the default because it is dramatically more
  bandwidth- and CPU-efficient than raw bitmaps for a full-desktop stream, and
  modern RDP clients (`mstsc`, FreeRDP) support AVC420.
- The raw-bitmap path is retained — served automatically to clients that
  disable AVC, and available behind `--bitmap` as an explicit override — so
  that clients without AVC420 support, or hosts without a working VAAPI H.264
  encoder, still have a functional transport.
- Transport selection is automatic and per client, driven by each client's own
  EGFX capability advertisement, so a mixed fleet (e.g. desktop `mstsc`
  alongside Microsoft's mobile clients, which disable AVC) can connect to the
  same bridge process without the operator needing to know in advance which
  path each client needs.
- The decision is still made once, at connection time, and held for that
  connection's lifetime rather than re-evaluated mid-session, to keep the two
  encode pipelines fully separate and avoid mid-session renegotiation
  complexity.
- The indicator reuses StatusNotifierItem and dbusmenu instead of a dedicated
  `org.otto.*` interface: the top bar already implements both, so the feature
  needs no bar-side code, works in any SNI host, and adds no permanent
  Otto-specific D-Bus surface to maintain. Bus-name ownership then gives crash
  safety for free — a bridge that dies has its name released by the bus daemon,
  so the indicator cannot outlive the process that raised it.
- `Stop Sharing` ends the bridge rather than just dropping the client, because a
  bridge left listening would let the remote party reconnect immediately, which
  is not what a user means by stopping sharing.
- The bitmap capture pipeline (CPU capture plus box-filter scale) is started
  lazily rather than always running, since most connections are expected to use
  H.264 and there is no reason to pay that cost for a client that never falls
  back.
- Serving the output's native size (instead of letterboxing to the client window)
  on the H.264 path keeps the encoded image at full fidelity and delegates scaling
  to the client, which every RDP client already does well.
- Keyframe gating and backpressure-aware dropping are used (instead of queuing) to
  keep the stream low-latency and self-correcting: a client always resynchronizes
  at the next IDR rather than replaying a backlog of stale frames.

## Open Questions

- (Resolved) Should the bridge detect a missing AVC420 negotiation and
  automatically fall back to bitmap mode, rather than only logging a warning?
  Yes — implemented as automatic per-client transport selection driven by the
  client's EGFX capability advertisement (see Transport selection).
- Should the served H.264 desktop size adapt to the client's requested window size
  rather than always serving the output's native size?
