# Screenshot Portal — implementation plan

The D-Bus screenshot portal: what is implemented, and the plan for capturing
in the compositor instead of shelling out.

> **Status: partly built.** `org.freedesktop.impl.portal.Screenshot` exists
> (`components/xdg-desktop-portal-otto/src/portal/screenshot.rs`, interface
> version 2) and `otto.portal` declares it alongside `ScreenCast`, `Settings`,
> `Access` and `FileChooser`. It captures by running `grim` over the whole
> output set into `~/Pictures/Screenshots/screenshot-<ms>.png` and returning
> that path as a `file://` URI. An `interactive` request is gated behind the
> Access dialog first, through the same `org.otto.Dialog1` renderer
> `AccessPortal` uses. The phases below (capture without an external binary,
> single-output and window selection, the colour picker) are still the plan.
>
> **This is not the same thing as taking a screenshot on Otto today.** The
> `zwlr_screencopy_v1` Wayland protocol is in production
> (`src/state/screencopy.rs`) and is what `grim`, `wf-recorder`, `wl-mirror`
> and OBS-via-wlrobs use — including `grim` as called from the portal backend.
> See [screenshare.md](screenshare.md#wlr-screencopy-v1). This document covers
> the *D-Bus portal* interface that GTK/Qt screenshot apps and sandboxed apps
> use instead.

## What it adds

`org.freedesktop.impl.portal.Screenshot`, so third-party screenshot tools
(GNOME Screenshot, Spectacle, Flameshot) can capture through the standard
portal.

The key difference from ScreenCast: a screenshot is **one file, once**, not a
stream. No PipeWire, no session, no format negotiation. The portal returns a
`file://` URI and the app takes it from there.

## The chain

Today:

```
Screenshot app
  → org.freedesktop.portal.Screenshot          (xdg-desktop-portal)
  → org.freedesktop.impl.portal.Screenshot     (xdg-desktop-portal-otto)
  → grim → zwlr_screencopy_v1                  (the compositor)
  → PNG in ~/Pictures/Screenshots → file:// URI
```

Planned, with the capture inside the compositor:

```
Screenshot app
  → org.freedesktop.portal.Screenshot          (xdg-desktop-portal)
  → org.freedesktop.impl.portal.Screenshot     (xdg-desktop-portal-otto)
  → org.otto.Screenshot                        (the compositor)
  → capture → PNG → temp file → file:// URI
```

## Data flow, once capture moves in-process

1. App calls `org.freedesktop.portal.Screenshot.Screenshot()`.
2. xdg-desktop-portal forwards to the otto backend.
3. The backend sends a D-Bus request to the compositor.
4. The compositor captures the current frame for the target output.
5. Convert to CPU memory if needed (dmabuf → RGBA).
6. Encode to PNG with the `image` crate.
7. Write to `$XDG_RUNTIME_DIR` or `/tmp`.
8. Return `file:///…/screenshot-XXXXXX.png`.
9. The app displays, saves, or copies it.

## Phase 1: capture in the compositor

**Not built.** The portal backend runs `grim`, so there is no compositor-side
screenshot command yet.

**Reuse the existing capture path.** The SHM branch of `zwlr_screencopy_v1`
already does steps 4–5: `BlitCurrentFrame` (`src/renderer/mod.rs:41`) for the
GPU side, `skia_surface.read_pixels` for the CPU readback. A screenshot is a
one-shot version of that, and should call the same code rather than growing a
parallel path.

**Compositor side.** A new `src/screenshare/screenshot.rs`, plus one command:

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
`CompositorCommand`; see the sync/async bridge in
[screenshare.md](screenshare.md#2-the-syncasync-bridge--srcscreensharemodrs).

**Portal side.** `ScreenshotPortal::screenshot` already returns
`(response, {"uri": …})` with `0` for success, `1` for cancelled and `2` for
failed. Only `capture_to_file` changes: forward over the existing
`org.otto.*` connection instead of spawning `grim`.

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

## Phase 2: colour picker

**Not built.** `PickColor` returns `(response_code, {"color": (r, g, b)})`.
It needs pixel readback at a point and a BGRA → RGB conversion; the same
capture path applies.

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

**Options.** `modal` (b) and `interactive` (b). `modal` is ignored.
`interactive` means the app wants the desktop to run the selection UI; since
Otto has none, the backend asks the user to confirm through the Access dialog
and then captures everything.

**Results.** `uri` (s), a `file://` URI to the PNG.

## Dependencies

Otto already depends on `image` 0.24 (optional, pulled in by the `udev` and
`debug` features; see `Cargo.toml`), so check whether that is enough before
adding another. A temp-file crate is only needed once capture moves into the
compositor; the portal backend writes to `~/Pictures/Screenshots` with
`std::fs`.

## Checklist

**Phase 1: done**
- [x] `components/xdg-desktop-portal-otto/src/portal/screenshot.rs`
- [x] Register the interface in the backend's `main.rs`
- [x] Add `org.freedesktop.impl.portal.Screenshot` to `otto.portal`: an
      interface that is implemented but not declared is never routed to
- [x] Gate `interactive` behind the Access dialog
- [x] A correct `file://` URI back to the app

**Phase 1: remaining**
- [ ] `src/screenshare/screenshot.rs`: one-shot capture reusing
      `BlitCurrentFrame` / `read_pixels`
- [ ] `Screenshot` variant in `CompositorCommand` and its handler
- [ ] PNG encoding and temp-file creation in the compositor
- [ ] Point `capture_to_file` at it instead of `grim`
- [ ] Test with `gnome-screenshot`, `spectacle`, `flameshot gui`

**Phase 2**
- [ ] `PickColor` command and handler
- [ ] Pixel readback and BGRA → RGB conversion
- [ ] Test with a portal-aware colour picker

## Design notes

- **No selection UI in the compositor.** Third-party apps provide their own
  selection and annotation. The only compositor-drawn surface in the path is
  the Access dialog that confirms an `interactive` request.
- **PNG only**, most compatible and lossless.
- **Every output at once**, written to `~/Pictures/Screenshots`.

## Later

Specific output selection; window-specific screenshots by window id (the
`window_to_dmabuf` path from screenshare already does this); JPEG with a
quality parameter; app-provided save location; delay/timer; area capture from
coordinates.
