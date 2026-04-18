## Screen Sharing in Otto

This document explains (at a high level) how screensharing is wired in Otto.
It focuses on the control flow and where to look in the code.

### Overview

Otto exposes a compositor-side ScreenCast service (`org.otto.ScreenCast`). A portal
backend (`components/xdg-desktop-portal-otto`) uses it to create a session, pick an
output, and publish a PipeWire node that apps consume via the standard
`org.freedesktop.portal.ScreenCast` API.

Conceptually there are two halves:

- **Control plane**: D-Bus calls create/stop sessions and streams.
- **Data plane**: after Otto renders a frame, it copies the rendered framebuffer into
  a PipeWire-provided DMA-BUF (GPU-only blit), then tells PipeWire a new frame is ready.

### Architecture

```
Application (OBS, Chrome, Firefox)
    |
    v org.freedesktop.portal.ScreenCast
    
xdg-desktop-portal
    |
    v org.freedesktop.impl.portal.ScreenCast
    
xdg-desktop-portal-otto (components/xdg-desktop-portal-otto/)
    |
    v org.otto.ScreenCast
    
Otto Compositor:
    
    D-Bus Service (tokio thread)          Render Loop (udev)
    - CreateSession                       - Direct GPU Blit
    - StartRecording          <-------->  - glBlitFramebuffer
    - StopRecording                       - Damage-aware regions
    - DestroySession                      - GPU-only (no CPU copy)
        |                                     |
        v calloop channel                     |
                                              v
    Session Management
    - Active screenshare sessions
    - PipeWire buffer pool management
    - Per-output stream tracking
        |
        v
    PipeWireStream (dedicated thread)
    - MainLoop, Context, Core, Stream
    - DMA-BUF buffer management
    - Video format negotiation (BGRA preferred)
    - VideoDamage metadata (SPA_META_VideoDamage)
        |
        v PipeWire video stream
        
Application receives video frames via PipeWire
```

If you only remember one thing: **apps talk to the portal**, and the portal talks to
**Otto’s `org.otto.ScreenCast` service**, while frames are produced from the compositor
render loop.

### Components

#### 1. D-Bus Service (`src/screenshare/dbus_service.rs`)

The compositor runs a zbus server on a dedicated tokio thread and registers:

- Service: `org.otto.ScreenCast`
- Root object: `/org/otto/ScreenCast`
- Per-session objects: `/org/otto/ScreenCast/session/<id>`
- Per-stream objects: `<session>/stream/<id>`

**Interfaces:**

| Interface | Path | Description |
|-----------|------|-------------|
| `org.otto.ScreenCast` | `/org/otto/ScreenCast` | Main service interface |
| `org.otto.ScreenCast.Session` | `/org/otto/ScreenCast/session/<id>` | Per-session control |
| `org.otto.ScreenCast.Stream` | `/org/otto/ScreenCast/stream/<id>` | Per-stream control |

**Methods (what they mean):**

```
org.otto.ScreenCast:
  CreateSession(properties: a{sv}) -> session_path: o
  ListOutputs() -> connectors: as

org.otto.ScreenCast.Session:
  RecordMonitor(connector: s, properties: a{sv}) -> stream_path: o
  RecordWindow(properties: a{sv}) -> stream_path: o
  Start()
  Stop()
  OpenPipeWireRemote(options: a{sv}) -> fd: h

org.otto.ScreenCast.Stream:
  Start()
  Stop()
  PipeWireNode() -> info: a{sv}
  Metadata() -> info: a{sv}

Notes:

- `RecordWindow` currently returns “not supported”.
- `Start()` is where the compositor actually creates a PipeWire stream and returns a node id
  through `PipeWireNode()`.
```

### 2. PipeWire Stream (`src/screenshare/pipewire_stream.rs`)

`PipeWireStream` owns the PipeWire stream and its buffer pool.

What matters for understanding the design:

- It runs a **PipeWire main loop on a dedicated thread**.
- It negotiates a video format and asks PipeWire for **DMA-BUF buffers**.
- It configures **single-buffer mode** (`min=1,max=1`) and advertises
  `SPA_META_VideoDamage` (so clients can take advantage of damage metadata).

**Configuration:**

```rust
pub struct StreamConfig {
    pub width: u32,              // Stream width
    pub height: u32,             // Stream height
    pub framerate_num: u32,      // Framerate numerator (e.g., 60)
    pub framerate_denom: u32,    // Framerate denominator (e.g., 1)
    pub gbm_device: Option<Arc<GbmDevice<DrmDeviceFd>>>,
    pub capabilities: BackendCapabilities,
}
```

### 3. Command Handler (`src/screenshare/mod.rs`)

The compositor is synchronous (calloop), but zbus is async. The bridge is:

- D-Bus thread sends a `CompositorCommand` through a `calloop::channel`.
- The compositor main loop handles the command and mutates `state.screenshare_sessions`.

The command enum is defined in `src/screenshare/mod.rs`.

```rust
pub enum CompositorCommand {
  CreateSession { session_id: String },
  ListOutputs { response_tx: tokio::sync::oneshot::Sender<Vec<OutputInfo>> },
  StartRecording {
    session_id: String,
    output_connector: String,
    response_tx: tokio::sync::oneshot::Sender<Result<u32, String>>,
  },
  StopRecording { session_id: String, output_connector: String },
  DestroySession { session_id: String },
  GetPipeWireFd {
    session_id: String,
    response_tx: tokio::sync::oneshot::Sender<Result<zbus::zvariant::OwnedFd, String>>,
  },
}
```

This is the core pattern to keep in mind when debugging: if a D-Bus call “hangs” or
does nothing, it’s usually because the calloop side didn’t receive/handle the command.

### Frame Delivery

Screen sharing uses a **direct GPU blit** approach:

1. Otto renders the output as usual (Skia → GL framebuffer).
2. If a screenshare stream exists for that output, Otto asks the `PipeWireStream` for
  an available buffer.
3. Otto blits the framebuffer into that DMA-BUF using `Blit<Dmabuf>` (GPU-only
  `glBlitFramebuffer`).
4. Otto signals PipeWire that a new frame can be queued.

#### Udev Backend (`src/udev.rs`)

This is the only backend that currently has the “after-render blit into PipeWire buffer”
integration.

**Key Features:**
- **GPU-only path**: No CPU memcpy, direct FBO→dmabuf blit via `glBlitFramebuffer`
- **Damage-aware**: Only blits changed regions when reusing the same PipeWire buffer
- **Zero-copy**: Compositor renders once, PipeWire consumes GPU buffer directly
- **Synchronous**: Blit happens on main thread immediately after render

### Winit Backend (`src/winit.rs`)

Winit starts the screenshare D-Bus service, but does not currently implement the
per-frame PipeWire delivery path described above.

### Usage

#### Session Setup and Prerequisites

Screen sharing requires proper D-Bus session setup and PipeWire services. The compositor must share the same D-Bus session with applications.

**Required Services:** PipeWire + a session manager (typically WirePlumber).

#### Starting the Compositor

**Production (TTY/bare metal):**

Use the provided `start_session.sh` script for proper environment setup:

```bash
./scripts/start_session.sh
```

This script automatically:
1. Creates or reuses a D-Bus session
2. Saves D-Bus info to `$XDG_RUNTIME_DIR/dbus-session` for other terminals
3. Starts/verifies PipeWire services via systemctl
4. Launches the xdg-desktop-portal-otto backend
5. Starts the compositor with correct environment variables

**Development (windowed mode):**

```bash
cargo run --release -- --winit
```

Note: Windowed mode may have different D-Bus session requirements.

### Running Applications with Screen Sharing

To use Chrome, Firefox, OBS, etc. with screen sharing support:

**From another terminal on the same TTY:**

```bash
# Load D-Bus session environment created by start_session.sh
source "$XDG_RUNTIME_DIR/dbus-session"

# Ensure you target the compositor Wayland socket
export WAYLAND_DISPLAY=wayland-0

# Now run applications
google-chrome        # Chrome/Chromium
firefox             # Firefox
obs                 # OBS Studio
```

That’s usually enough. If an app can’t see screensharing, it’s almost always a
**D-Bus session mismatch**.

### Testing with D-Bus

If you need to sanity-check the compositor service itself:

- `busctl --user list | grep org.otto.ScreenCast`
- `busctl --user introspect org.otto.ScreenCast /org/otto/ScreenCast`

### Verifying PipeWire Stream

Look for the node id returned by the portal / compositor, then inspect it with `pw-dump`.

### Testing with Applications

After setting up the portal backend (see `docs/xdg-desktop-portal.md`):

1. Start the compositor: `./scripts/start_session.sh`
2. In another terminal: `source "$XDG_RUNTIME_DIR/dbus-session"`
3. Launch an application and test screen sharing:
   - **Chrome/Chromium**: Visit a meeting site, click share screen
   - **OBS Studio**: Add a "Screen Capture (PipeWire)" source
   - **Firefox**: Start screen sharing in a web conference
   - **GNOME Screen Recorder**: Use built-in screen recorder

Expected performance: typically capped at 60 FPS for WebRTC compatibility.

### Troubleshooting

**Screen sharing dialog shows no outputs:**
- Ensure the app is running in the same D-Bus session as the compositor
- Check `$DBUS_SESSION_BUS_ADDRESS` is set: `echo $DBUS_SESSION_BUS_ADDRESS`
- Verify portal is registered: `busctl --user list | grep otto`
- Try: `source "$XDG_RUNTIME_DIR/dbus-session"` before launching the app

**Video freezes after a few seconds:**
- Verify PipeWire is running: `pgrep -x pipewire`
- Check compositor logs: `tail -f otto.log`
- Ensure systemd user services are enabled:
  ```bash
  systemctl --user enable --now pipewire.service pipewire-pulse.service wireplumber.service
  ```
- Restart PipeWire: `systemctl --user restart pipewire.service`

**D-Bus connection errors:**
- The D-Bus session file is created when compositor starts
- If running from a different TTY, source the session file first
- Ensure `$XDG_RUNTIME_DIR/dbus-session` exists and is readable
- Check permissions: `ls -la $XDG_RUNTIME_DIR/dbus-session`

**Portal backend not found:**
- Verify portal is built: `ls target/release/xdg-desktop-portal-otto`
- Check portal logs: `tail -f components/xdg-desktop-portal-otto/portal.log`
- Rebuild if needed: `cargo build -p xdg-desktop-portal-otto --release`

### File Overview

| File | Purpose |
|------|---------|
| `src/screenshare/mod.rs` | Module root, session state, command handlers, direct blit utility |
| `src/screenshare/dbus_service.rs` | D-Bus interface implementation |
| `src/screenshare/pipewire_stream.rs` | PipeWire stream management, buffer pool, format negotiation |
| `src/skia_renderer.rs` | Blit<Dmabuf> trait implementation for direct GPU blitting |
| `src/udev.rs` | Direct blit integration (udev backend) |
| `src/winit.rs` | Starts the screenshare D-Bus service (frame delivery currently udev-only) |

### Related: wlr-screencopy-v1

The portal/PipeWire path above is **not** the only way to get frames out of Otto.
External tools (`grim`, `wf-recorder`, `wl-mirror`, OBS via wlrobs) use the
`zwlr_screencopy_manager_v1` Wayland protocol directly, implemented in
`src/state/screencopy.rs`.

To avoid two parallel readback paths, wlr-screencopy reuses the same
`BlitCurrentFrame` infrastructure as PipeWire screenshare:

- The compositor advertises **`linux_dmabuf`** alongside the SHM
  `buffer` event (v3+ clients), so capable consumers can negotiate a GPU
  dmabuf and skip CPU readback.
- **Dmabuf clients** ride `BlitCurrentFrame::blit_current_frame` — the
  same `glBlitFramebuffer` call PipeWire screenshare uses. Zero CPU copy.
- **SHM clients** (legacy `grim`, default `wf-recorder`) fall back to
  `skia_surface.read_pixels` — synchronous, expensive, but only paid by
  the few tools that need it. This path could later be replaced with a
  GPU blit-into-temp-dmabuf + async PBO readback for parity.
- The post-render hook is **gated on `!pending_screencopy_frames.is_empty()`**;
  it does no work when no client is asking. It does **not** force renders —
  pending frames piggyback on the next render that happens for some other
  reason (scene damage, cursor, DND).

**Quick perf reference** (Iris Xe, 2880×1920 @ 120 Hz, idle desktop ~7%):
- `wf-recorder -c h264_vaapi` (dmabuf): ~9% Otto CPU during capture
- `wf-recorder` default (SHM, libx264): ~30% Otto CPU during capture
- No active capture: 7% (no overhead)

### Known Issues and Fixes

#### Framerate Compatibility (January 2026)

**Issue**: When the compositor runs on high-refresh-rate displays (e.g., 120Hz), screensharing 
would fail with Chrome/WebRTC clients showing "no more input formats" error.

**Root Cause**: Commit 5ea901 changed the code to use the actual display refresh rate instead 
of hardcoding 60Hz. Chrome and most WebRTC implementations don't support PipeWire streams 
above 60fps, causing format negotiation to fail.

**Fix**: The screenshare framerate is now capped at 60fps regardless of the display's actual 
refresh rate:

```rust
let framerate_num = (refresh_rate / 1000).min(60); // Cap at 60fps for compatibility
```

This maintains compatibility with Chrome, Firefox, OBS, and other screenshare clients while 
still allowing the display to run at higher refresh rates (120Hz, 144Hz, etc.) for normal 
compositor operation.

**Future**: The FPS cap should be made configurable (e.g., `config.screenshare.max_fps`) to 
allow power users to experiment with higher framerates for specific clients that support them.

#### Portal/compositor integration drift (January 2026)

If screensharing fails very early (no portal session / `OpenPipeWireRemote` failures), double-check:

- **Compositor object path**: the compositor registers `org.otto.ScreenCast` at `/org/otto/ScreenCast`.
- **Portal client default path**: `components/xdg-desktop-portal-otto` currently has a hard-coded
  default path of `/org/screencomposer/ScreenCast` in its D-Bus proxy; that must match the compositor.
- **PipeWire remote FD**: the compositor-side `OpenPipeWireRemote` currently forwards to a
  `GetPipeWireFd` command which is still marked TODO in `src/screenshare/mod.rs`.

If you are debugging this, start by looking at the logs from:

- `otto.log` (compositor)
- `components/xdg-desktop-portal-otto/portal.log` (portal backend)

<!-- Implementation details intentionally omitted here.
  If you need them, start from src/screenshare/pipewire_stream.rs and src/udev.rs. -->
