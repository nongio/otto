# wlr-screencopy-v1

Otto implements `zwlr_screencopy_manager_v1` (version 2), allowing screen capture tools to grab output frames via shared-memory buffers.

## Usage

```bash
# Full-screen screenshot (saves to file)
grim screenshot.png

# Region capture
grim -g "100,100 800x600" region.png

# Live mirroring
wl-mirror eDP-1
```

Any client speaking wlr-screencopy-v1 works — `grim`, `wl-mirror`, `wlr-randr` (for output info), `wf-recorder`, etc.

## Protocol flow

```
Client                          Otto
  │                               │
  ├─ capture_output(output) ─────►│
  │                               ├─ create frame object
  │◄── buffer(ARGB8888, w, h, s) ─┤  (advertise SHM format)
  │◄── buffer_done ───────────────┤  (v3+)
  │                               │
  ├─ copy(wl_buffer) ────────────►│
  │                               ├─ queue as PendingScreencopy
  │                               ├─ force render (even if no damage)
  │                               ├─ read_pixels from Skia surface
  │◄── flags(empty) ──────────────┤
  │◄── ready(tv_sec, tv_nsec) ────┤
  │                               │
  ├─ destroy ────────────────────►│
```

`capture_output_region` follows the same flow but scales the requested logical region to physical pixels using the output's fractional scale.

## Architecture

### Key types (`src/state/screencopy.rs`)

| Type | Role |
|------|------|
| `ScreencopyManagerState` | Holds the Wayland global; created in `Otto::init` |
| `ScreencopyFrameData` | Per-frame metadata: output, region, dimensions, state machine |
| `PendingScreencopy` | Queued frame + client buffer, waiting to be filled during render |

### Render integration (`src/udev/render.rs`)

1. When `pending_screencopy_frames` is non-empty, `should_draw` is forced true — the compositor renders even if the scene has no damage.
2. After `render_frame` completes and the output is scanned out, `complete_screencopy_for_output` is called.
3. It uses `skia_surface.read_pixels()` to copy the rendered frame into each pending SHM buffer, then sends `ready` or `failed`.

### Buffer format

Only `ARGB8888` (4 bytes/pixel) is advertised. Stride is `width * 4`. The Skia read uses `BGRA8888` color type which matches ARGB8888 on little-endian (the Wayland convention).

## Limitations

- **SHM only** — no DMA-BUF export. Every frame does a GPU-to-CPU readback via `read_pixels`. Fine for screenshots; a streaming client like `wl-mirror` will consume more bandwidth than a zero-copy DMA-BUF path would.
- **No damage reporting** — `copy_with_damage` is accepted but damage regions are not reported to the client.
- **Cursor overlay** — the `overlay_cursor` flag is stored but the cursor is always composited into the frame (same as overlay_cursor=1).
- **Udev backend only** — the `complete_screencopy_for_output` call site is in `src/udev/render.rs`. Winit/X11 backends don't call it yet.
