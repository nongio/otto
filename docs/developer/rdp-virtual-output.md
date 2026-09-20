# RDP bridge for virtual outputs (`otto-rdp`)

`components/otto-rdp` serves an Otto **virtual output** over RDP: remote
clients see the frames Otto renders for that output and their mouse/keyboard
input is injected back into the compositor, targeted at that output. With an
`interactive = true` virtual output this behaves like a remote-accessible
extra screen.

The design principle worth noting: **the compositor knows nothing about RDP.**
A virtual output is just an output that has no physical connector; Otto renders
it and publishes it as a PipeWire node exactly like any other. `otto-rdp` is an
ordinary client that consumes that node and injects input back through standard
virtual-pointer and virtual-keyboard protocols. Any other remote-display
protocol could be added the same way, without touching `src/`.

## Architecture

```
Otto ──renders──▶ virtual output PipeWire node ──▶ otto-rdp ──RDP──▶ client
  ▲                                                   │
  └──── zwlr_virtual_pointer (bound to the output) ◀──┤
  └──── zwp_virtual_keyboard (default xkb keymap) ◀───┘
```

Three subsystems:

- `pipewire_capture.rs` consumes the virtual output's existing PipeWire
  node (the same stream any screenshare consumer would use). Negotiates raw
  32-bit BGRx video **without modifiers**; handles pre-mapped `MemFd`/`MemPtr`
  buffers and mmap-able linear `DmaBuf`s. Frames are re-packed to tight
  stride and fanned out on a `tokio::sync::broadcast` channel (lagging RDP
  connections skip frames rather than backlogging).
- `wl_input.rs` is a Wayland client on Otto's own socket. Finds the target
  output by name via xdg-output, then creates a virtual pointer **with that
  output** (`create_virtual_pointer_with_output`) so `motion_absolute`
  coordinates map into the output's geometry server-side, plus a virtual
  keyboard with a default libxkbcommon keymap.
- `rdp.rs` / `main.rs` / `egfx.rs` / `h264.rs` are the `ironrdp-server` glue.
  Video goes out over EGFX/AVC420 by default: `h264.rs` encodes captured
  frames with VA-API through GStreamer, `egfx.rs` pushes the access units down
  the graphics-pipeline channel. A client that advertises AVC as disabled, or
  `--bitmap`, falls back to full-frame `DisplayUpdate::Bitmap`s from the
  broadcast channel, which ironrdp compresses. RDP mouse/keyboard events are
  translated to the input thread. Mouse handles both absolute `Move` and
  relative `RelMove` / `Scroll{x,y}` (touchpad-mode mobile clients send the
  relative variants).
  Keyboard handles set-1 scancodes (→ evdev keycodes, with an extended-code
  table for arrows/nav/meta) and Unicode (mobile/on-screen keyboards send
  `UnicodePressed`): ASCII codepoints are injected by tapping the matching
  **US-QWERTY keycode + Shift** against the fixed startup keymap. This is
  deliberately *not* a per-keystroke keymap swap: swapping races the client
  applying the new keymap and yields the wrong character.

## Compositor-side support

Two pieces of compositor behaviour carry this bridge. They matter to every
synthesized-input consumer, not just to RDP.

**Virtual-pointer output binding.** `create_virtual_pointer_with_output` stores
the bound output, and `motion_absolute` maps normalized coordinates into
**that** output's global geometry (`src/state/virtual_pointer.rs`). Without the
binding, absolute motion would land on the first output and the bridge could
not aim input at the virtual screen.

**Virtual-keyboard delivery.** In the pinned smithay revision the
virtual-keyboard dispatch sends the client the keymap but does **not** deliver
the key on the non-IME path, so the compositor has to forward it, as smithay's
anvil example does. `on_keyboard_event`
(`src/state/virtual_keyboard_handler.rs`) forwards to the focused surface, and
not through the shortcut filter, so remote typing reaches apps and never fires
compositor shortcuts. Without it every synthesized key would be dropped: this
bridge's, and wlrctl's, ydotool's and KDE Connect's alike.

## Running

```sh
# otto_config.toml
[[virtual_outputs]]
name = "virtual-1"
resolution = { width = 1920, height = 1080 }
refresh_hz = 30.0
position = { x = 1440, y = 0 }
interactive = true        # pointer/focus can reach it → remote control works
```

```sh
WAYLAND_DISPLAY=wayland-1 otto-rdp --output virtual-1 --listen 0.0.0.0:3389
# from the remote machine (TLS is on by default):
xfreerdp3 /v:<host>:3389 /cert:ignore
```

The bridge resolves the PipeWire node itself — no need to read the numeric
id out of Otto's log. Otto tags each virtual output's node with a custom
`otto.output.name` property (`src/screenshare/pipewire_stream.rs`), and
`otto-rdp`'s `discover` module (`components/otto-rdp/src/discover.rs`) walks
the PipeWire registry for a node matching `--output` (default `virtual-1`).
Pass `--node <id>` directly to skip discovery if you already have the id.

The whole command line:

| Flag | |
|---|---|
| `--list` | print the nodes it can see and exit |
| `--output <name>` | the virtual output to serve, by `otto.output.name` (default `virtual-1`) |
| `--node <id>` | a PipeWire node id, skipping discovery; not with `--connector` |
| `--connector <name>` | serve a **physical** output instead, through `org.otto.ScreenCast` |
| `--listen <addr:port>` | the bind address |
| `--port <n>` | the port alone (default 3389) |
| `--desktop <WxH>` | announce a fixed desktop size; see the iOS note below |
| `--tls` / `--no-tls` | TLS is on by default |
| `--bitmap` | full-frame bitmaps instead of EGFX/H.264 |

**Rendering is on-demand.** `render_virtual_outputs()`
(`src/udev/render.rs`) skips a virtual output's composite/render work
entirely while its PipeWire stream has no linked consumer, checked via
`PipeWireStream::is_streaming()`, which mirrors PipeWire's own
`StreamState::Streaming` (true only once a consumer has connected and
negotiated format). A configured-but-unwatched virtual output costs nothing
until `otto-rdp` (or any other PipeWire consumer) connects.

## Sharing indicator

While a client is being served, the bridge publishes a red dot in the top bar's
tray with a `Stop Sharing` menu. It uses StatusNotifierItem + dbusmenu, so no
bar-side code is involved, and it cannot go stale; see
[Remote-Desktop Indicator](remote-desktop-indicator.md).

## Testing

The RDP wire protocol itself needs a real client (and a VA-API encoder), so it
stays a manual check. What is covered automatically:

```sh
cargo test -p otto-rdp                            # the bridge's own logic
cargo test --features headless --test rdp_bridge  # what it asks of Otto
```

`cargo test -p otto-rdp` covers the decisions a client's connection turns on:
the served layout (client box verbatim, aspect-fit picture, even rounding on
the EGFX path), the mouse mapping back through that letterbox, AVC-vs-bitmap
transport selection from the advertised capability sets, and the RDP→evdev
scancode table.

`tests/rdp_bridge.rs` drives a headless compositor as the bridge does: it binds
the same globals in the same versions, reads the output's logical geometry from
`xdg-output`, and injects pointer motion, a click and keystrokes through
`zwlr_virtual_pointer_v1` / `zwp_virtual_keyboard_v1`, asserting they land on
the right window and reach the application.

## Current limitations

- **No auth**: trusted-network only. TLS security with a self-signed
  certificate (generated once, persisted under `~/.local/state/otto-rdp`,
  key 0600) is on **by default**, required by `mstsc` and Microsoft's
  mobile clients, which refuse the plain-RDP layer. Pass `--no-tls` for the
  `RdpServerSecurity::None` listener instead (FreeRDP `/sec:rdp`).
  CredSSP/NLA auth is the natural next step; ironrdp-server supports it.
- Full-frame updates every frame — no damage-based partial updates. The
  virtual output renders at its configured refresh (`refresh_hz`, 60 Hz by
  default); the H.264 encoder's inter-frame coding, or ironrdp's bitmap
  compression on the fallback path, keeps this workable on a LAN. Feeding
  Otto's damage tracking into the update rects is the obvious optimization.
- No clipboard or audio. The desktop size is negotiated once at connect:
  the client's reported box is served **verbatim** (letterboxed
  server-side: native picture aspect-fit and centered, black bars baked
  in), and mouse input (absolute and relative alike) is mapped from box
  space back to native pixels through the picture rect; bar positions
  clamp to the picture edge. There is no dynamic re-negotiation after
  connect, so rotating a phone mid-session keeps the originally
  negotiated size.
- **Client scaling quirks** (the client's `desktopScaleFactor` hints are
  dropped).
  Microsoft's iOS Windows App: a served desktop that *matches* its
  requested box renders 1:1 physical in a corner (no upscaling); a
  *mismatched* one is stretched non-uniformly to fill the view. The
  workaround is `--desktop WxH` set to the device's **physical screen
  resolution**: that box is served verbatim with the picture aspect-fit
  and centered inside, so the app's stretch is uniform (desktop aspect
  == view aspect) and the picture displays full-screen and undistorted.
  Input is normalized from the client's reported box through the
  picture rect.
- Keyboard injection covers ASCII only. Non-ASCII Unicode (accents, emoji,
  non-Latin scripts) is dropped with a debug log; a compose/dead-key or
  dynamic-keymap path is the follow-up.
- Conversely, a client that only speaks plain-RDP (`xfreerdp /sec:rdp`)
  can connect **only** with `--no-tls`; TLS is on by default.
