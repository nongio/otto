# AirPlay video screenshare — design exploration

Status: **exploration; nothing implemented in Otto.** Phase 0 below — sending a
virtual output to an Apple TV via doubletake, with no Otto code at all — has
been validated against real hardware. Everything past that is still design
space.

This doc maps that space in both directions and recommends a path. It follows
the `otto-rdp` pattern ([rdp-virtual-output.md](rdp-virtual-output.md)): a
standalone component that consumes a virtual output's PipeWire node, with the
compositor knowing nothing about the remote protocol.

## Two directions, two features

| | Sender (Otto → Apple device) | Receiver (Apple device → Otto) |
|---|---|---|
| What it gives | Apple TV / AirPlay TV becomes an extra (virtual) Otto display | iPhone/iPad/Mac screen appears as a window in Otto |
| Input backchannel | none — AirPlay mirroring is display-only | none |
| Protocol difficulty | hard (FairPlay SAP on the sender side) | solved (UxPlay, RPiPlay, shairplay) |
| Prior art | doubletake (Go, active), AirplayMirroringGo (dormant) | UxPlay (GPLv3), shairplay Rust crate (LGPL, mirroring experimental) |

Unlike RDP there is **no input path in either direction** — AirPlay
mirroring has no HID backchannel a third party can use. So the RDP bridge
remains the "remote-controllable screen" story; AirPlay is view-only:
`interactive = false` virtual outputs are the right source, and none of the
virtual-pointer/keyboard compositor fixes are needed.

## Sender: Otto virtual output → Apple TV

### Protocol reality

AirPlay 2 mirroring is mDNS discovery (`_airplay._tcp`) + RTSP-like control
on port 7000 + an H.264 elementary stream (128-byte-header packets, encrypted)
on a separate TCP port, with PTP/NTP timing. The gatekeeping steps:

1. **pair-setup / pair-verify** — HomeKit-style SRP-6a with a PIN on first
   contact (mandatory since tvOS 10.2), Ed25519/X25519 session keys,
   ChaCha20-Poly1305 channel encryption. Fully reverse-engineered; pyatv and
   doubletake implement it; all the crypto exists as Rust crates.
2. **fp-setup (FairPlay SAP)** — the sender must produce a valid FairPlay
   handshake that a genuine Apple TV accepts. This is *the* blocker for a
   clean-room sender: receivers get away with a small reverse-engineered
   decryptor (playfair), but the sender side has only been done by executing
   snapshots of Apple's own ARM64 FairPlay code (doubletake calls this
   "snapshot-backed Go ARM64 execution", the Slave-in-the-Magic-Mirror
   approach). Legally gray, unpleasant to port, breakable by tvOS updates.

### The pragmatic observation: doubletake already works with Otto

[doubletake](https://github.com/omarroth/doubletake) (LGPL-3.0, v0.4.0
July 2026) is a Linux AirPlay 2 mirroring sender that captures via
**xdg-desktop-portal / PipeWire** and encodes with VA-API/NVENC. Otto already
ships `xdg-desktop-portal-otto` and exposes every output — including virtual
outputs — as portal monitors backed by PipeWire nodes. So in principle:

```
[[virtual_outputs]]            # otto_config.toml — view-only is fine
name = "airplay-1"
resolution = { width = 1920, height = 1080 }
refresh_hz = 30.0
position = { x = 1440, y = 0 }

$ doubletake   # pick "airplay-1" in the portal dialog, pick the Apple TV
```

gives "Apple TV as a second Otto monitor" with **zero Otto code**. Caveats:
largely LLM-written by its author's own admission, security posture unknown,
tested against Apple TV 4K / M-series Macs / some TVs (Xiaomi broken).

### Native sender (`otto-airplay-tx`) — if we ever want one

Reuses two of otto-rdp's three modules nearly verbatim:

```
virtual output PipeWire node ──▶ pipewire_capture.rs (BGRx frames)
        ──▶ H.264 encode (VA-API; new)
        ──▶ AirPlay2 session: mDNS + pair-verify + fp-setup + RTSP  (new)
        ──▶ encrypted 128-byte-header video stream + PTP timing     (new)
```

- Capture + `org.otto.ScreenCast` glue: exists (`pipewire_capture.rs`,
  `screencast.rs`).
- Encoding: new dependency surface. Otto has no gstreamer/ffmpeg today;
  the lean option is direct `libva` bindings (or spawning `ffmpeg`), not a
  gstreamer stack. 1080p30 BGRx → H.264 at ~10–20 Mbit is the target; AirPlay
  receivers expect baseline/main H.264, SPS/PPS in-band.
- Protocol: pairing/crypto is a week of work with existing crates
  (`srp`, `x25519-dalek`, `ed25519-dalek`, `chacha20poly1305`, mDNS, bplist).
  **FairPlay SAP is not** — the realistic route is linking or spawning
  doubletake's fairplay module rather than reimplementing it. That keeps the
  gray-area code out of Otto's MIT tree (LGPL helper process).
- Latency: AirPlay mirroring lands around 150–500 ms end-to-end (encode +
  receiver buffering). Fine for video/slides/reference material on a TV,
  not for typing at — set expectations accordingly in docs/UI.

### Non-mirroring fallback: AirPlay Video (HLS push)

`play_url`-style AirPlay (what pyatv/open-airplay do) needs **no FairPlay**
for non-DRM content: tell the Apple TV to fetch an HLS URL we serve. We could
encode a virtual output (or a single window) into LL-HLS and cast it. Latency
is seconds — useless as a display, plausible as "cast this video/window to
the TV". Cheap to build, but a different feature than screenshare; parked.

## Receiver: iPhone/iPad/Mac screen as an Otto window (`otto-airplay-rx`)

The reverse direction is well-trodden: advertise `_airplay._tcp` +
`_raop._tcp`, accept the mirroring session (receiver-side FairPlay is the
solved playfair problem), decode H.264, present. Two integration shapes:

- **Wayland-client component** (recommended): each incoming session creates
  an xdg toplevel and renders decoded frames (VA-API decode → dmabuf →
  `wp_linux_dmabuf`). No compositor changes at all; it composes with
  everything (expose, workspaces, screenshare re-share).
- Deeper compositor integration (dedicated layer/scene node) buys nothing
  the window doesn't already give.

Building blocks: [shairplay crate](https://docs.rs/shairplay) (LGPL-3.0,
pure Rust, AirPlay 1+2, mirroring behind an experimental `video` feature —
evaluate first) or porting UxPlay's session logic (GPLv3 — **cannot** be
vendored into MIT Otto; process-boundary only). A macOS sender can also use
an AirPlay receiver as an **extended** display, which would make an
Otto machine a wireless external monitor for a MacBook — worth verifying
against UxPlay first to confirm third-party receivers get that mode.

## Recommendation

1. **Phase 0 — validate with doubletake (no code). Done.** Virtual output +
   portal + doubletake against a real Apple TV works: Otto's capture path
   carries an AirPlay session end to end with no compositor changes. So the
   sender direction does not obviously need to be owned natively — the
   "integration" may amount to documentation plus a dock affordance that
   launches it. A receiver build and the audio path are still open.
2. **Phase 1 — `otto-airplay-rx`** if receiving is wanted: highest
   feasibility, pure-Rust path available, and it's a visible differentiator
   (iPhone screenshare lands as a normal Otto window; possibly Mac wireless
   display). Component-only, MIT-safe via LGPL crate.
3. **Phase 2 — native sender** only if Phase 0 shows demand and we accept
   the FairPlay strategy (external LGPL helper for fp-setup). Skeleton is
   80% otto-rdp minus input, plus an encoder.

## Sources

- [doubletake — AirPlay sender for Linux (X11 & Wayland)](https://github.com/omarroth/doubletake)
- [UxPlay — AirPlay mirroring receiver](https://github.com/FDH2/UxPlay), [RPiPlay](https://github.com/FD-/RPiPlay), [PhairPlay](https://github.com/mazer666/PhairPlay)
- [shairplay Rust crate](https://docs.rs/shairplay/latest/shairplay/)
- [Unofficial AirPlay protocol spec (nto)](https://nto.github.io/AirPlay.html), [openairplay spec — known implementations](https://openairplay.github.io/airplay-spec/known_implementations.html)
- [AirPlay internals: pairing & fp-setup](https://air-display.github.io/airplay-internal/pairing_process.html), [AirPlay 2 internals (Cozzi)](https://emanuelecozzi.net/docs/airplay2)
- [Slave-in-the-Magic-Mirror — original FairPlay code-execution approach](https://github.com/espes/Slave-in-the-Magic-Mirror)
- [AirplayMirroringGo — dormant Go sender](https://github.com/openairplay/AirplayMirroringGo)
- [pyatv — pairing/AirPlay-video prior art](https://pyatv.dev)
