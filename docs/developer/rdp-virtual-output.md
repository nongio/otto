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

Three subsystems, one per module:

- `pipewire_capture.rs` — consumes the virtual output's existing PipeWire
  node (the same stream any screenshare consumer would use). Negotiates raw
  32-bit BGRx video **without modifiers**; handles pre-mapped `MemFd`/`MemPtr`
  buffers and mmap-able linear `DmaBuf`s. Frames are re-packed to tight
  stride and fanned out on a `tokio::sync::broadcast` channel (lagging RDP
  connections skip frames rather than backlogging).
- `wl_input.rs` — a Wayland client on Otto's own socket. Finds the target
  output by name via xdg-output, then creates a virtual pointer **with that
  output** (`create_virtual_pointer_with_output`) so `motion_absolute`
  coordinates map into the output's geometry server-side, plus a virtual
  keyboard with a default libxkbcommon keymap.
- `rdp.rs` / `main.rs` — `ironrdp-server` glue: full-frame
  `DisplayUpdate::Bitmap`s from the broadcast channel (ironrdp applies RDP
  bitmap compression), RDP mouse/keyboard events translated to the input
  thread. Mouse handles both absolute `Move` and relative `RelMove` /
  `Scroll{x,y}` (touchpad-mode mobile clients send the relative variants).
  Keyboard handles set-1 scancodes (→ evdev keycodes, with an extended-code
  table for arrows/nav/meta) and Unicode (mobile/on-screen keyboards send
  `UnicodePressed`): ASCII codepoints are injected by tapping the matching
  **US-QWERTY keycode + Shift** against the fixed startup keymap. This is
  deliberately *not* a per-keystroke keymap swap — swapping races the client
  applying the new keymap and yields the wrong character.

## Compositor-side support

Two compositor fixes were needed to make this work. Both have landed; they are
recorded here because they affect every synthesized-input consumer, not just
this bridge.

**Virtual-pointer output binding.** `create_virtual_pointer_with_output` stores
the bound output, and `motion_absolute` maps normalized coordinates into
**that** output's global geometry (`src/state/virtual_pointer.rs`).
Previously the output argument was ignored and absolute motion always mapped
to the first output, so the bridge could not aim input at the virtual screen.

**Virtual-keyboard delivery.** `on_keyboard_event`
(`src/state/virtual_keyboard_handler.rs`) was a no-op. In the pinned smithay
revision the virtual-keyboard dispatch sends the client the keymap but does
**not** deliver the key on the non-IME path — the compositor must forward it
(as smithay's anvil example does). Without this, *every* synthesized key —
this bridge, plus wlrctl / ydotool / KDE Connect — was silently dropped. The
handler now forwards to the focused surface (not through the shortcut filter,
so remote typing reaches apps and never triggers compositor shortcuts).

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

Otto logs the node at startup: `Virtual output 'virtual-1' started (PipeWire node N)`.

```sh
WAYLAND_DISPLAY=wayland-1 otto-rdp --node <N> --output virtual-1 --listen 0.0.0.0:3389
# from the remote machine:
xfreerdp /v:<host>:3389 /sec:rdp
```

## Current limitations

- **No auth**: trusted-network only. `--tls` enables TLS security with a
  self-signed certificate (generated once, persisted under
  `~/.local/state/otto-rdp`, key 0600) — required by `mstsc` and
  Microsoft's mobile clients, which refuse the plain-RDP layer. Without
  `--tls` the listener is `RdpServerSecurity::None` (FreeRDP `/sec:rdp`).
  CredSSP/NLA auth is the natural next step; ironrdp-server supports it.
- Full-frame updates every frame — no damage-based partial updates yet. The
  virtual output renders at its configured refresh (30 Hz default) and
  ironrdp's bitmap compression keeps this workable on a LAN; wiring Otto's
  damage tracking into `BitmapUpdate` rects is the obvious optimization.
- No clipboard or audio. The desktop size is negotiated once at connect:
  the client's reported box is served **verbatim** (letterboxed
  server-side — native picture aspect-fit and centered, black bars baked
  in), and mouse input (absolute and relative alike) is mapped from box
  space back to native pixels through the picture rect; bar positions
  clamp to the picture edge. No dynamic re-negotiation after connect —
  rotating a phone mid-session keeps the originally negotiated size.
- **Client scaling quirks** (no RDPGFX in ironrdp-server, legacy bitmap
  updates only; the client's `desktopScaleFactor` hints are dropped).
  Microsoft's iOS Windows App: a served desktop that *matches* its
  requested box renders 1:1 physical in a corner (no upscaling); a
  *mismatched* one is stretched non-uniformly to fill the view. The
  workaround is `--desktop WxH` set to the device's **physical screen
  resolution**: that box is served verbatim with the picture aspect-fit
  and centered inside, so the app's stretch is uniform (desktop aspect
  == view aspect) and the picture displays full-screen and undistorted.
  Input is normalized from the client's reported box through the
  picture rect. The proper fix is RDPGFX support upstream.
- Keyboard injection covers ASCII only. Non-ASCII Unicode (accents, emoji,
  non-Latin scripts) is dropped with a debug log — a compose/dead-key or
  dynamic-keymap path is the follow-up.
- Clients that only speak TLS/NLA (Windows `mstsc`, the Microsoft mobile
  apps) cannot connect to the `RdpServerSecurity::None` listener; use a
  FreeRDP-based client with `/sec:rdp`, or add the TLS acceptor.
