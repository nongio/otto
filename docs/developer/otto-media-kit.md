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

The pipeline is `playbin3` (or `playbin` where `playbin3` is missing) with
`video-sink` set to `videoconvert ! videoscale ! capsfilter ! appsink`, the
sink running `sync=true max-buffers=2 drop=true`. The caps filter asks for
RGBx within the host's limits as *ranges*, so `videoscale` keeps the aspect
ratio and never scales up. The file is opened as
`file:///proc/self/fd/3`: the worker never learns a path.

## Containment

The worker contains itself after exec (`contain()` in the worker binary):
`chdir("/")`, `PR_SET_NO_NEW_PRIVS`, `RLIMIT_AS` 8 GiB (hardware decoders map
device memory freely), `RLIMIT_NOFILE` 512, `RLIMIT_CORE` 0, then a
best-effort `unshare(CLONE_NEWUSER | CLONE_NEWNET)`. Compared with the decode
worker in `otto-quickview` it has no `RLIMIT_FSIZE` (the plugin registry
cache), no `RLIMIT_CPU`, a higher descriptor ceiling, and no pre-exec half.

The host clears the environment and passes a whitelist: `PATH`, `HOME`,
`XDG_RUNTIME_DIR`, `XDG_CACHE_HOME`, `RUST_LOG`, `LANG`, `LANGUAGE`,
`PULSE_SERVER`, `PIPEWIRE_REMOTE`, `LD_LIBRARY_PATH`, `OTTO_MEDIA_TRACE`, and
the `GST_*` and `LIBVA_*` variables, because the audio server's socket and
the registry cache live there. Wayland and bus *addresses* are not passed,
but this is not a filesystem jail: with `XDG_RUNTIME_DIR` known, those
sockets are still reachable by path.

There is no `PR_SET_PDEATHSIG`. A worker whose host dies exits when stdin
reaches EOF or a write to stdout fails. Dropping a `Player` sends `quit` and
then kills the worker at once, so the worker's own teardown rarely runs.

## Debugging

- `OTTO_MEDIA_TRACE=1` lets the worker's stderr through and adds GStreamer's
  debug string to error events.
- `cargo run -p otto-media-kit --example probe -- FILE` plays four seconds
  with no display, makes an accurate seek to 1 s around the two-second mark,
  and writes the last frame as a PNG (`PROBE_PNG` names it; the default is
  `/tmp/otto-media-probe.png`). Cargo puts examples in
  `target/<profile>/examples/`, away from the worker, so build the worker
  (`cargo build -p otto-media-kit --bin otto-media-worker`) and point
  `OTTO_MEDIA_WORKER` at it.
- `GST_DEBUG=3` works as usual since `GST_*` is passed through.

## Embedding in a preview surface

`otto_media_kit::view::draw_frame` draws from a `Frame` + `State` snapshot
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
