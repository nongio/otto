# Screenshot Portal — implementation plan

> **Status: not implemented.** No `org.freedesktop.impl.portal.Screenshot`
> exists in `xdg-desktop-portal-otto`, and `otto.portal` declares only
> `ScreenCast` and `Settings`.
>
> **This is not the same thing as taking a screenshot on Otto today.** The
> `zwlr_screencopy_v1` Wayland protocol is already in production
> (`src/state/screencopy.rs`) and is what `grim`, `wf-recorder`, `wl-mirror`
> and OBS-via-wlrobs use. See
> [screenshare.md](screenshare.md#wlr-screencopy-v1). This document covers the
> *D-Bus portal* interface that GTK/Qt screenshot apps and sandboxed apps use
> instead.

## What it would add

`org.freedesktop.impl.portal.Screenshot`, so third-party screenshot tools
(GNOME Screenshot, Spectacle, Flameshot) can capture through the standard
portal.

The key difference from ScreenCast: a screenshot is **one file, once**, not a
stream. No PipeWire, no session, no format negotiation. The portal returns a
`file://` URI and the app takes it from there.

## The chain

```
Screenshot app
  → org.freedesktop.portal.Screenshot          (xdg-desktop-portal)
  → org.freedesktop.impl.portal.Screenshot     (xdg-desktop-portal-otto)
  → org.otto.Screenshot                        (the compositor)
  → capture → PNG → temp file → file:// URI
```

## Data flow

1. App calls `org.freedesktop.portal.Screenshot.Screenshot()`.
2. xdg-desktop-portal forwards to the otto backend.
3. The backend sends a D-Bus request to the compositor.
4. The compositor captures the current frame for the target output.
5. Convert to CPU memory if needed (dmabuf → RGBA).
6. Encode to PNG with the `image` crate.
7. Write to `$XDG_RUNTIME_DIR` or `/tmp`.
8. Return `file:///…/screenshot-XXXXXX.png`.
9. The app displays, saves, or copies it.

## Phase 1: basic screenshot

**Reuse the existing capture path.** The SHM branch of
`zwlr_screencopy_v1` already does exactly steps 4–5: `BlitCurrentFrame`
(`src/renderer/mod.rs`) for the GPU side, `skia_surface.read_pixels` for the CPU
readback. A screenshot is a one-shot version of that, and should call the same
code rather than growing a parallel path.

**Compositor side** — a new `src/screenshare/screenshot.rs` plus one command:

```rust
pub enum CompositorCommand {
    // … existing commands
    Screenshot {
        output_name: String,
        response_tx: oneshot::Sender<Result<String, String>>, // the URI
    },
}
```

The handler captures one frame, gets RGBA out of it, encodes PNG, writes a
temp file, and returns the URI. It runs on the main loop like every other
`CompositorCommand` — see the sync/async bridge in
[screenshare.md](screenshare.md#2-the-syncasync-bridge--srcscreensharemodrs).

**Portal side** — a new
`components/xdg-desktop-portal-otto/src/portal/screenshot.rs`:

```rust
impl Screenshot for PortalBackend {
    async fn screenshot(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        options: HashMap<String, Value<'_>>,
    ) -> Result<(u32, HashMap<String, Value>)> {
        // forward over the existing org.otto.* connection
        // return (response_code, {"uri": "file:///…"})
    }
}
```

**PNG encoding:**

```rust
use image::{ImageBuffer, Rgba};

fn encode_png(rgba: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>> {
    let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba)
        .ok_or("bad buffer size")?;
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}
```

Do not forget to register the interface in the backend's `main.rs` **and add
it to `otto.portal`** — an interface that is implemented but not declared is
never routed to.

## Phase 2: colour picker (optional)

`PickColor` returns `(response_code, {"color": (r, g, b)})`. It needs pixel
readback at a point and a BGRA → RGB conversion; the same capture path applies.

## D-Bus interface

```xml
<method name="Screenshot">
  <arg type="o"    name="handle"        direction="in"/>
  <arg type="s"    name="app_id"        direction="in"/>
  <arg type="s"    name="parent_window" direction="in"/>
  <arg type="a{sv}" name="options"      direction="in"/>
  <arg type="u"    name="response"      direction="out"/>
  <arg type="a{sv}" name="results"      direction="out"/>
</method>
```

**Options** — `modal` (b) and `interactive` (b). Both can be ignored: the
requesting app provides its own selection and annotation UI.

**Results** — `uri` (s), a `file://` URI to the PNG.

## Dependencies

```toml
image = { version = "0.25", default-features = false, features = ["png"] }
tempfile = "3.0"
```

Otto already depends on `image` behind the `udev` and `debug` features; check
whether the existing dependency is enough before adding another.

## Checklist

**Phase 1**
- [ ] Add `image` (png) and `tempfile` dependencies where needed
- [ ] `src/screenshare/screenshot.rs`: one-shot capture reusing `BlitCurrentFrame` / `read_pixels`
- [ ] `Screenshot` variant in `CompositorCommand` and its handler
- [ ] PNG encoding and temp-file creation with a correct `file://` URI
- [ ] `components/xdg-desktop-portal-otto/src/portal/screenshot.rs`
- [ ] Register the interface in the backend's `main.rs`
- [ ] Add `org.freedesktop.impl.portal.Screenshot` to `otto.portal`
- [ ] Test with `gnome-screenshot`, `spectacle`, `flameshot gui`

**Phase 2**
- [ ] `PickColor` command and handler
- [ ] Pixel readback and BGRA → RGB conversion
- [ ] Test with a portal-aware colour picker

## Design notes

- **No UI in the compositor.** Third-party apps provide their own selection and
  annotation.
- **Temporary files.** `/tmp` or `$XDG_RUNTIME_DIR`; the app is responsible for
  deleting them.
- **PNG only** initially — most compatible and lossless.
- **Full primary output** initially.

## Later

Specific output selection; window-specific screenshots by window id (the
`window_to_dmabuf` path from screenshare already does this); JPEG with a
quality parameter; app-provided save location; delay/timer; area capture from
coordinates.
