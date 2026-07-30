# RDP Bridge (otto-rdp)

**Status:** draft  
**Related specs:** multi-output.md, screenshare.md

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

### Input forwarding

- Remote keyboard and pointer input from the RDP client is forwarded into the
  compositor and aimed at the captured output (see `multi-output.md` for how
  absolute pointer input is mapped into a specific output's geometry). This
  behavior is identical for both transport modes.

### Configuration

- `OTTO_RDP_FPS` — target capture/encode frame rate. Default is 30 on the H.264
  path.
- `OTTO_RDP_BITRATE` — target H.264 bitrate in kbps.
- `OTTO_RDP_H264_ENCODER` — selects the hardware H.264 encoder element. Default is
  the standard VAAPI H.264 encoder; an alternative low-power encoder may be named
  instead.

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
