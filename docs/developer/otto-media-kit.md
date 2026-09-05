# otto-media-kit

Video playback for Otto applications, kept out of otto-kit on purpose: a
media stack is more code than the toolkit it would be bolted onto, and it
links GStreamer, which no application binary should.

The user-facing behaviour is specified in
[specs/quickview.md](../../specs/quickview.md#video-playback); this page is
about the pieces.

## Two halves

```
components/otto-media-kit/
├── src/lib.rs                     the library a host embeds (no GStreamer)
│   ├── player.rs                  Player: spawn the worker, commands, events, frames
│   ├── protocol.rs                the pipe/ring contract shared by both halves
│   ├── transport.rs               the control bar: layout, draw, hit-test
│   └── view.rs                    frame fitted above the transport
├── src/bin/otto-media-worker.rs   the worker (feature `worker`, default on)
└── examples/probe.rs              play a file for 4 s with no display
```

Hosts depend on the library with `default-features = false` so GStreamer
never enters their link. The worker binary is built from the same crate and
found at run time: `OTTO_MEDIA_WORKER`, then next to the host's executable,
then `PATH`. `otto_media_kit::player::available()` says whether one was found.

## The worker's contract

Descriptors are fixed, not negotiated: the media file on 3 (read-only), the
frame ring on 4 (read-write). Commands are lines on stdin, events lines on
stdout — `protocol.rs` has the grammar and both parsers, and its tests keep
them round-tripping.

The ring is a memfd the host creates and the worker sizes once it knows the
frame size: a 4 KiB header then three slots of tightly packed RGBx. A `frame`
event names the slot, the sequence number and the presentation time; the
host copies the slot out on its event thread before the worker is back to it
two frames later. `ready` carries the size and duration and may repeat — the
duration is often only known after preroll.

The pipeline is `playbin3` with `video-sink` set to
`videoconvert ! videoscale ! capsfilter ! appsink`. The caps filter asks for
RGBx within the host's limits as *ranges*, so `videoscale` keeps the aspect
ratio and never scales up. The file is opened as
`file:///proc/self/fd/3`: the worker never learns a path.

## Containment

Same shape as the decode worker in `otto-quickview`, minus what a media
stack cannot live under: no `RLIMIT_FSIZE` (the plugin registry cache) and
an 8 GiB address-space ceiling (hardware decoders map device memory freely).
The environment is a whitelist — `PATH`, `HOME`, `XDG_RUNTIME_DIR`, the
`GST_*` and `LIBVA_*` variables, the locale — because the audio server's
socket and the registry cache live there. No Wayland or bus address.

## Debugging

- `OTTO_MEDIA_TRACE=1` lets the worker's stderr through and adds GStreamer's
  debug string to error events.
- `cargo run -p otto-media-kit --example probe -- FILE` plays four seconds
  with no display, seeks once, and writes the last frame as a PNG
  (`PROBE_PNG` names it). Point `OTTO_MEDIA_WORKER` at the worker if it is
  not next to the example binary.
- `GST_DEBUG=3` works as usual since `GST_*` is passed through.

## Embedding in a preview surface

`otto-media-kit::view::draw_frame` draws from a `Frame` + `State` snapshot
rather than the live `Player`, so a host that records its drawing into a
picture on another thread (as otto-files' preview column does) can paint the
player without holding the player. `otto-files` uses it in two places behind
one `quickview::Video`: the Quick View panel (autoplays) and the docked
Miller preview column (opens paused on the first frame, plays on click).

A paused pipeline emits its first frame as a *preroll*, not a sample, so the
worker delivers both — otherwise a paused embed would show black. The docked
column always opens paused on that first frame and plays on click; only the
Quick View panel autoplays.

## Aspect and the preview-column subsurface

The player box takes the video's own shape, not the whole space it sits in:
`otto-files`' `view::preview_video_box` sizes it to `width / aspect +
transport height`, capped and centred, so a 16:9 clip in a narrow column is a
compact box on dark ground rather than a sliver in a field of black. Aspect
comes from the frame, then the announced size, then the poster
(`VideoSnapshot::aspect`); square pixels are already enforced worker-side, so
this never distorts anamorphic content.

In the preview column that box is its **own Wayland subsurface**
(`pane_surfaces::sync_preview_video`), so a 30 fps video repaints only that
surface — never the browser's toplevel, nor the scene's cached preview
picture, whose key drops the video term when the video is on a surface. Input
still belongs to the toplevel (empty input region), and the browser hit-tests
the same box, so play and scrub work through the existing routing. The Quick
View panel is unaffected: it is already its own surface.
