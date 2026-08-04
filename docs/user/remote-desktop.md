# Remote Desktop

`otto-rdp` serves an Otto output over RDP, so you can view **and control** your
desktop from Microsoft Remote Desktop, FreeRDP, or the Windows / iOS / Android
Remote Desktop apps.

## How it works

It is a standalone bridge: it captures an output's frames, encodes them, and
forwards the remote client's keyboard and pointer back into the compositor. The
compositor itself knows nothing about RDP.

## Two ways to use it

**Serve a physical screen** — mirror what is on your monitor:

```sh
otto-rdp --connector eDP-1 --listen 0.0.0.0:3389 --tls
```

**Serve a virtual output** — a headless screen that exists only for the remote
user, with its own workspaces and windows, independent of your physical
displays. This is usually what you want.

First declare it in your Otto config:

```toml
[[virtual_outputs]]
name = "virtual-1"
resolution = { width = 1920, height = 1080 }
refresh_hz = 60.0
position = { x = 3840, y = 0 }
interactive = true          # required for remote input
```

`interactive = true` is what allows remote pointer and keyboard events to be
aimed at that output. Without it the feed is view-only.

Otto logs the PipeWire node id at startup:

```
Virtual output 'virtual-1' started (PipeWire node 42)
```

Then:

```sh
otto-rdp --node 42 --output virtual-1 --listen 0.0.0.0:3389 --tls
```

## Connecting

```sh
# FreeRDP, hardware H.264
xfreerdp3 /v:192.168.1.10:3389 /gfx:AVC420 /cert:ignore

# mstsc, or the mobile / Windows App clients
# just point them at 192.168.1.10:3389 — TLS must be on
```

## Command-line reference

```
otto-rdp (--node <id> | --connector <name>) [options]
```

| Flag | Meaning |
|------|---------|
| `--node <id>` | PipeWire node id of a virtual output's stream (from Otto's log) |
| `--connector <name>` | Capture a physical output instead, e.g. `eDP-1`. Mutually exclusive with `--node`; also becomes the default `--output`. |
| `--output <name>` | Wayland output to aim input at. Defaults to `virtual-1`, or the `--connector` value. |
| `--listen <addr:port>` | Listen address. Default `0.0.0.0:3389`. |
| `--desktop <WxH>` | Serve this desktop size instead of the client's reported box |
| `--tls` | Accept TLS-security connections with a self-signed certificate |
| `--bitmap` | Force the legacy raw-bitmap path instead of hardware H.264 |

### Environment

| Variable | Default | Meaning |
|----------|---------|---------|
| `OTTO_RDP_FPS` | 30 (H.264), 12 (bitmap) | Frame rate |
| `OTTO_RDP_BITRATE` | — | Target bitrate in kbps, H.264 only |
| `OTTO_RDP_H264_ENCODER` | `vah264enc` | GStreamer encoder element; try `vah264lpenc` on some Intel GPUs |

## TLS

Use `--tls` unless you have a reason not to. `mstsc` and Microsoft's mobile,
iOS and Windows App clients **require** it — they refuse to run graphics over
plain RDP security.

The certificate is self-signed and persisted in `~/.local/state/otto-rdp`, so it
stays stable between runs and your client stops warning about a changed
certificate. Pass `/cert:ignore` to FreeRDP the first time.

With `--tls` on, plain-RDP clients (`xfreerdp /sec:rdp`) can no longer connect.

## Video transport

By default the bridge encodes with **hardware H.264** (VA-API) and ships it over
RDP's Graphics Pipeline Extension as AVC420. It falls back to raw bitmaps
automatically, per connection, for clients that cannot do H.264.

The choice is made once per client, from the capabilities that client advertises
when it connects, and does not change for the life of the connection.

- A client advertising AVC support gets the H.264 path.
- A client that sets `AVC_DISABLED` — which Microsoft's **mobile, iOS, Android
  and Windows App clients all do**, since they do not implement H.264 — falls
  back to bitmaps automatically.
- A client that never opens the graphics channel at all also falls back, after a
  short grace period.

`--bitmap` forces the legacy path for every client. Use it to isolate a
suspected H.264 problem.

Hardware encoding needs `gst-plugin-pipewire` and `gst-plugins-bad` (for VA-API)
installed.

## Input

Remote keyboard and pointer events are forwarded into the compositor as virtual
input devices, aimed at the output named by `--output`. Absolute pointer
coordinates are mapped into that output's own geometry, so the remote pointer
lands where the remote user clicked even when the served output is not the first
one.

Remote input drives the compositor's UI exactly like physical input: dock hover,
magnification and tooltips all respond.

## The `--tty-udev` caveat

Otto only renders while its virtual terminal is **active**. If you switch away
from Otto's VT, the DRM session is paused by libseat and the remote feed
freezes. That is expected, not a bug.

Run Otto on the VT you intend to leave it on.

## A helper script

The repo ships `run-rdp.sh`, which starts Otto and the bridge together with
logging, waits for the compositor to be ready, extracts the Wayland socket and
PipeWire node from the log automatically, and reports why Otto exited if it
does:

```sh
./run-rdp.sh            # serve the physical screen (eDP-1)
./run-rdp.sh virtual    # serve virtual-1
```

> **Careful:** `Logo+Q` and `Ctrl+Alt+Backspace` quit Otto instantly and cannot
> be rebound. Avoid pressing them from the remote client.

## Remote sessions and power

A lid close does **not** suspend the machine while an RDP client is actively
pulling frames, so the remote user is not cut off. A stream that exists but has
no consumer does not block suspend. See
[Power Management](power-management.md).

## Security

The bridge has **no authentication of its own** — anyone who can reach the port
gets your desktop. `--tls` encrypts the transport; it does not gate access.

Do not expose port 3389 to an untrusted network. Bind it to localhost and tunnel
over SSH instead:

```sh
otto-rdp --node 42 --output virtual-1 --listen 127.0.0.1:3389 --tls
# from the client machine:
ssh -L 3389:localhost:3389 you@your-host
```

## Troubleshooting

**The client connects but the screen is black.** Otto's VT is not active, or the
virtual output has no PipeWire node. Check Otto's log for the
`Virtual output ... started` line.

**"No graphics" or the client disconnects immediately.** The client cannot do
AVC420 and the fallback did not kick in. Retry with `--bitmap`.

**Input does nothing.** The virtual output needs `interactive = true`, and
`--output` must name the output you are actually serving.

**mstsc or the mobile app refuses to connect.** You need `--tls`.

**The H.264 encoder fails to start.** Install `gst-plugins-bad` for VA-API and
try `OTTO_RDP_H264_ENCODER=vah264lpenc`. Failing that, `--bitmap`.

**Clicks land in the wrong place from a mobile client.** Some clients render 1:1
in physical pixels but report their box in points. Pass the device's physical
resolution with `--desktop 2532x1170`.

## Not yet supported

- Audio, clipboard, and drive or device redirection
- Multi-monitor RDP sessions — one output per connection
- Switching transport mid-connection
- Any built-in access control
