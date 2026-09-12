# File Previews

How otto-files shows what is inside a file: grid and list thumbnails, the
preview column at the end of column view, the Quick View panel, and video
playback in the last two.

Behaviour is specified in [specs/quickview.md](../../specs/quickview.md) and
[specs/file-browser.md](../../specs/file-browser.md). This page describes the
structure as it is built today, and it disagrees with the Quick View spec in
several places; see [Where the spec has drifted](#where-the-spec-has-drifted).
The video worker has its own page, [otto-media-kit](otto-media-kit.md).

## The pieces

```
components/
├── otto-kit/src/preview/          Preview payload types, layout, draw, zoom maths (pure Skia)
├── otto-kit/src/filetype/         name → type of record, bytes → sniffed type
├── otto-quickview/                the decode worker and everything around it
│   ├── src/spawn.rs               open a file, run a worker on it, read the payload back
│   ├── src/sandbox.rs             rlimits, namespaces, fd hygiene, --sandbox-selftest
│   ├── src/payload.rs             the wire format ("OQV2") and its validation
│   ├── src/decode/                one decoder per content type, run in the worker
│   ├── src/opening.rs             entrance/exit geometry for the panel
│   ├── src/render.rs              PNG rendering for the standalone CLI only
│   └── src/main.rs                `otto-quickview` CLI: --describe, --render, --filmstrip
├── otto-media-kit/                video: Player (library) + otto-media-worker (GStreamer)
└── otto-files/src/
    ├── thumbnails.rs              in-memory thumbnail store (no threads, no I/O)
    ├── thumbcache.rs              read-only freedesktop thumbnail cache
    ├── quickview.rs               Session, zoom/pan, Video, decode request sizing
    ├── pane_surfaces.rs           the panel's and the preview video's subsurfaces
    ├── view.rs                    draw_quickview, preview column stage, draw_thumbnail
    └── app.rs                     wiring: sync_thumbnails, start_preview, start_quickview
```

Three paths use this code. **Every file byte is interpreted in the same
sandboxed decode worker**, which is the host binary run again with
`--decode-worker`:

```
                         ┌─────────────── otto-files process ───────────────┐
 grid/list/column rows ─▶│ thumbnails::Store ─▶ thumbcache::lookup ─(miss)─┐│
 preview column ────────▶│ start_preview ───────────────────────────────── ┤│
 Quick View (Space) ────▶│ start_quickview ─────────────────────────────── ┤│
                         │               spawn_blocking: decode_path ◀──────┘│
                         └──────────────────────────┬────────────────────────┘
                                   fd 3 = the file   │  stdout = OQV2 payload
                                                     ▼
                         otto-files --decode-worker  (sandboxed, one per decode)
                                                     │
                     video/* card + media worker available?
                                                     ▼
                         otto-media-worker  (GStreamer, one per playback)
```

Every host must call `otto_quickview::run_worker_if_requested()` as the first
thing in `main`, before any thread or Wayland connection exists (otto-files
does it in `main.rs`). If it doesn't, the worker is the host re-executed as an
ordinary host, which starts a second file browser instead of decoding
anything.

## The payload

Decoders don't draw. A decoder produces an `otto_kit::preview::Preview`, and
every host draws that with `otto_kit::preview::draw`. The set of variants is
closed:

| Variant | Carries |
|---------|---------|
| `Pixels { pixels, pages, page }` | Premultiplied RGBA8888 at a stated size, plus the intrinsic size and page information |
| `Text { lines, truncated, language }` | Validated UTF-8 lines. `language` is recorded, but there is no highlighting |
| `Rows { rows, truncated, summary }` | A directory or archive listing |
| `Card { title, subtitle, facts, hero, icon }` | Key/value facts, optional artwork, an icon-theme chain |
| `Unavailable { reason, icon }` | Why there is no preview, shown under the file's icon |

A new content type adds a decoder that returns one of these, not a new variant.

**The icon chain is filled in two places.** The worker fills it from the
sniffed MIME type (`decode::decode`). The parent fills in a chain derived from
the file name only when the chain is still empty (`payload::with_icon`), which
happens for payloads no decoder produced: a crash, a timeout, a file that
couldn't be opened. When the two disagree, the worker's chain is used, because
the name is only a guess.

**Wire format** (`payload.rs`). The format is written by hand, without serde:
the magic `OQV2`, a one-byte tag, then little-endian integers and strings
prefixed with their length. On the host side `payload::decode`:
- never panics on truncated input (`Cursor::take` uses checked arithmetic);
- rejects any length above 512 MiB, and any string that isn't valid UTF-8;
- caps preallocation, so a hostile length can't reserve memory up front;
- refuses a pixel buffer whose length isn't `w × h × 4`;
- turns a bad magic or an unknown tag into `None`, which `spawn::run` turns
  into `Unavailable`.

`Pixels::to_image` checks the length once more before handing the buffer to
Skia. Tests at the bottom of `payload.rs` reject every truncated prefix of a
valid payload.

## The decode worker

### Spawning (`spawn.rs`)

`open(path)` opens the file with `O_NONBLOCK`, calls `fstat`, and refuses FIFOs,
sockets and devices, which could block on open or on read. `decode_path` wraps
the whole sequence and always returns a `Preview`, even when the open fails.

`run` then:
1. Starts `current_exe() --decode-worker --width W --height H --page P --zoom Z
   --name NAME` with a cleared environment. Only `PATH`, `RUST_LOG` (default
   `error`) and `LANGUAGE` are passed, the last so that reasons and card facts
   come back in the user's language.
2. In `pre_exec`: `dup2`s the file to **fd 3**, closes every other inherited
   descriptor (`close_range`), and applies the sandbox.
3. Reads the worker's **stdout** to the end on a helper thread, waiting at most
   **8 s** (`DEADLINE`). A worker that overruns is killed and the result is
   `Unavailable`.
4. Ignores the exit status. Only the bytes on stdout count: a crashed worker
   produces a truncated payload, and truncated payloads fail validation.

The worker never learns a path. Its `name` argument is used for titles and to
narrow a sniffed type to a subtype, never to choose a decoder.

### Containment (`sandbox.rs`)

`sandbox::apply` runs twice: once in `pre_exec` and again at the top of the
worker. It does the following, in order:
1. `chdir("/")`;
2. `PR_SET_NO_NEW_PRIVS`;
3. `RLIMIT_AS` 1 GiB, `RLIMIT_CPU` 10 s, `RLIMIT_FSIZE` 0, `RLIMIT_NOFILE` 64,
   `RLIMIT_CORE` 0;
4. `unshare(CLONE_NEWUSER | CLONE_NEWNET)`, falling back to `CLONE_NEWNET`.
   This is best effort and failures are ignored.

If `apply` fails inside the worker, the worker answers
`Unavailable("quickview-error-sandbox")` rather than decoding without
containment. Decoders also read through `read_capped` (512 MiB by default) and
apply their own limits, listed below.

**This is not a filesystem jail.** There is no seccomp filter and no mount
namespace, so a compromised decoder can still open and read any file the user
can read. The code says so, and
`otto-quickview --sandbox-selftest` reports it (`can_still_open_other_files`)
together with the rlimits and a network probe. The self-test runs the real
sandbox in a real child with a piped stdout, because `RLIMIT_FSIZE = 0` would
raise SIGXFSZ on a file-backed stdout. Keep its output in step with any change
to `sandbox.rs`.

### Dispatch and decoders (`decode/`)

`decode::previewed`:
- sends a directory to `listing::directory`;
- answers a zero-length file with an "empty file" card;
- otherwise reads 4 KiB and calls
  `filetype::refine_opt(filetype::sniff(head), name)`. The content decides the
  type, and the name may only narrow it to a subtype (an SVG behind an XML
  declaration, for example).

`dispatch` then tries these in order:

| Sniffed type | Decoder | What it does | Limits |
|--------------|---------|--------------|--------|
| `image/svg+xml` | `image::svg` | Skia's SVG DOM with `SealedResources`: no external loads, system fonts only | 64 MiB read; surface up to 8192 px |
| `application/pdf` | `pdf::render` | Runs the first rasteriser found on `PATH` (`pdftoppm`, `pdftocairo`, `mutool`, `gs`), passes the document on stdin, decodes the PNG it writes | 2048 px maximum width; page clamped to the page count. With no rasteriser, a card naming the package to install |
| `image/*` | `image::raster` | Skia `Codec` with sampled decoding. Codecs that can't sample (PNG) decode in full and are then resampled down with `fit_within` | 64 MiB pixels (`MAX_DECODE_PIXELS`); above that, a "too large" card. Never upscales |
| zip, or a subclass of it | `listing::zip` | Walks the central directory; nothing is decompressed | 32 MiB directory, 2000 rows |
| `application/x-tar` | `listing::tar` | Reads the 512-byte headers of an uncompressed tar | 64 MiB read, 2000 rows |
| `audio/*` | `media::audio` | ID3v2.3/2.4 tags written by hand, with APIC cover art as the hero | 1 MiB of header |
| `video/*` | `media::video` | MP4/QuickTime `mvhd`/`tkhd`: duration and dimensions. **No poster frame** | 1 MiB of header |
| `text/plain`, or a subclass of it | `text::read` | Refuses a NUL in the first 4 KiB; reads UTF-8 (BOM removed) or falls back to Latin-1 | 8 MiB, 4000 lines, 2000 bytes per line |
| anything else | `media::generic` | Card with the kind and size | — |

The only dependencies are `skia-safe`, `libc` and `otto-kit`: there is no image
crate, PDF library, archive crate or syntax highlighter. That is a deliberate
choice. PDF rasterisers are run as programs rather than linked, so a new format
can often be supported by adding a command to a table.

`Request` sizes are in physical pixels. The host already asks for about twice
the panel size, so **decoders must not double it again**.

## Thumbnails

### The store (`thumbnails.rs`)

`thumbnails::Store` has no threads and does no work itself. The host asks it
which thumbnails are wanted, runs those jobs, and reports each result back:

- **Requests.** `Browser::sync_thumbnails` runs once per update and considers
  **only the active column**. Parent Miller columns draw thumbnails the store
  already holds but never ask for new ones. It works out the visible range for
  the current mode (grid, list or Miller pane; search results and Recent are
  ordinary columns) and skips directories.
- **Key.** Absolute path plus mtime. A different mtime makes the entry
  refetchable, and `image()` won't serve a stale one.
- **Throttling.** At most `MAX_IN_FLIGHT = 4` jobs at a time, taken top to
  bottom from the visible range, with no other prioritisation.
- **No cancellation.** Jobs for rows that scrolled away still run to
  completion, and changing folder doesn't reset the store (`Store::clear` has
  no callers). Entries live until evicted.
- **Eviction.** At most 512 entries or 96 MiB (`w × h × 4` for ready images),
  whichever is hit first. The oldest *insertion* goes first, and pending slots
  are never evicted.

### Fetching (`thumbnails::fetch`)

`App::start_thumbnail` runs each job with `tokio::task::spawn_blocking`:

1. **`thumbcache::lookup`** reads the freedesktop cache in-process.
   - **Root and size.** `$XDG_CACHE_HOME/thumbnails`. It picks the smallest
     size bucket at or above the request, otherwise the largest smaller one.
   - **Name.** `md5(file URI).png`, with MD5 and the URI escaping written by
     hand.
   - **Validation.** `Thumb::MTime` is compared in whole seconds, read by a
     small PNG text-chunk walker, and the image is loaded with
     `Image::from_encoded`.
   - **Failures.** `fail/otto-files/` markers are honoured on read.
2. **On a miss**, it generates a thumbnail only when `may_generate` is true.
   That is `Kind::Image` today, so PDF and video thumbnails come from the shared
   cache or not at all. Generating calls `otto_quickview::decode_path` with a
   thumbnail-sized `Request`: **one sandboxed worker per thumbnail**.
3. **Only `Preview::Pixels` becomes a thumbnail.** Cards, text and listings
   leave the type icon in place.

**The cache is read-only.** Nothing is written, neither thumbnails nor failure
markers. A miss is remembered only in memory, as `State::Absent`.

**Sizes.** `Size::for_box(edge, scale)` rounds up to a 128/256/512/1024
bucket. The box edge is 64 pt in grid and 18 pt in list and column views.
`AppContext::scale_factor()` is an integer, so a fractional scale is truncated.

**Back on the UI thread.** `Store::finish` bumps `epoch` when a picture lands,
and the worker calls `request_wakeup`. The Miller pane picture keys include
the epoch. That epoch is global, so every landing invalidates every pane, not
only the one showing that file.

### Drawing

All three views go through `view::draw_thumbnail`, which aspect-fits the image
(never enlarging it), aligns it to the bottom of the icon box, and adds a
hairline:
- list and grid call it from their immediate-mode row and cell drawing;
- column view uses `scene.rs` rows, which prefer `thumb` over `icon`.

A thumbnail replaces the type icon on the next repaint, with no placeholder or
fade. The draw uses `SamplingOptions::default()`, so a 128 px bucket drawn into
an 18 pt row is sampled nearest-neighbour.

## The preview column

The trailing pane in column view (`view::PREVIEW_W = 280`) is shown when
`Browser::preview_visible` holds: column view, exactly one selected entry, and
that entry is not a directory. **It does not use thumbnails**: it runs a full
Quick View decode at panel size.

- `sync_preview_target` replaces `PreviewPaneState` and bumps its generation on
  every selection change. There is no cache, and an in-flight decode is not
  cancelled.
- `App::start_preview` runs `quickview::decode(path, panel, scale)` on a
  blocking task. The width is `280 × scale × 2` and the height is the window
  height × scale × 2, both clamped to 64–4096. `finish_preview` drops results
  from an old generation.
- **Drawing.** The stage (`draw_preview_stage`) draws:
  - the video, if there is one;
  - otherwise a card's `hero`, or the large type icon while the decode is
    pending or unavailable;
  - otherwise `otto_kit::preview::draw` at `Zoom::FIT`.

  The caption (`preview_info`) shows kind, size and modified time.
- The whole pane is a `picture_cached` scene layer (`files-preview`). Its key
  covers the name, scroll, whether a result has arrived, the video, the theme
  and the size.
- A video opens **paused** on its first frame and plays on click, because the
  column follows the arrow keys; see [Video](#video).

## Quick View

### A session

`quickview::Session` is the whole state of one open panel: the current
`Preview`, `first_row`, `Zoom`, the two pan `ScrollView`s, an optional `Video`,
`expanded`, `opened_at`, and whether it is closing.

Opening and moving between files work like this:

1. Space, or the palette's **Quick Look**, calls `start_quickview`. The palette
   can't run it itself: `run_request` returns `Followup::QuickView`, and the
   host drains that in `follow_quickview`.
2. `begin_quickview` bumps `quickview_generation` and replaces the session with
   `Session::awaiting(name, is_dir, anchor)`. The old preview, zoom, pan and
   video are dropped, while `opened_at` and `expanded` are kept, so moving to
   another file doesn't replay the entrance. The panel shows "Opening preview…"
   immediately, **never the previous file's content**.
3. A blocking task runs `quickview::decode`, sized at panel × scale × 2 and
   clamped to 64–4096. `finish_quickview` discards a result whose generation is
   stale.
4. An arrow key, Home, End or Page Up/Down with the panel open moves the
   selection and calls `start_quickview` again.

Stale decodes are **ignored, not killed**. The superseded worker keeps running
until it finishes or reaches the 8 s deadline. `close_quickview` also bumps the
generation, so a slow decode can't reopen a panel that was dismissed.

### The surface

The panel is a **subsurface** of the browser toplevel, created and synced by
`PaneSurfaces::sync_quickview`:

- **Position.** By default it is centred on the display. `request_output_frame`
  is asked once when the panel opens and once when the exit starts, so a window
  moved in between carries the panel with it. `OTTO_FILES_QV_CENTER=0` centres
  it on the window instead. Combined with `OTTO_FILES_PANE_SUBS` unset, that
  paints the panel into the toplevel's own buffer. That legacy path is still
  maintained.
- **Material.** Set through `otto_surface_style_v1`: corner radius 12, a shadow,
  and `BackgroundBlur` when frosting is on (`OTTO_FROSTING`), `Normal`
  otherwise. The client draws no shadow of its own.
- **Stacking.** `restack` orders the subsurfaces as columns, preview video, pan
  bar, palette, catcher, Quick View, calling `place_above` again on each sync.
- **Repaints.** The panel is repainted only when `quickview_key` changes (rect,
  generation, `first_row`, zoom, scrollbars, loading, the video's frame
  sequence), and not while a frame is in flight. A resize is claimed only once
  the matching buffer has been painted (`PaneSurface::place` holds it as a
  pending claim for `draw`), because the style protocol applies a size
  immediately and would stretch the old buffer. A hidden panel drops its input
  region.

**Entrance and exit** use the geometry in `otto_quickview::opening`:
- **`entrance`.** A single uniform scale from the anchor (the cursor entry's icon
  rect, `view::quickview_anchor`) to the resting rect, clamped to 0.04–1. With
  no anchor it swells in place from 0.96.
- **Timing.** `sample` is a spring with a small overshoot and runs 300 ms in;
  `sample_out` is a smoothstep and runs 180 ms out.
- **Who animates.** The client does, resizing and moving the surface every
  frame (`Session::panel`), with the frame loop kept alive by
  `quickview_animating()`. Comments in `opening.rs` and `lib.rs` still say the
  compositor runs it through surface-style transactions; it doesn't.
- **Chrome.** The 30 pt title strip, close dot and expand dot fade in with the
  card's size (`quickview_chrome_opacity`), so they don't fill the first frames.
- **Closing.** `close_quickview` moves the session to `quickview_closing`,
  re-reads the anchor, and `tick_quickview_exit` retires it. A playing video
  keeps being drawn through the exit.

**Expand** (`Session::toggle_expanded`) swaps the resting rect: normally 72% of
the space with a 420×320 minimum, expanded it fills the space less a 12 pt
margin. The setting survives moving to another file.

### Drawing and input

`view::draw_quickview` draws the card background and hairline, the title strip,
and then either `otto_media_kit::view::draw` for a video or
`otto_kit::preview::draw(content, preview, first_row, zoom, icons)`, followed by
the pan scrollbars.

`otto_kit::preview` owns layout and the zoom geometry, so every host clamps the
same way:

- **Scale.** `Zoom { scale, offset, band }` runs from fit to `MAX = 8×`, and
  snaps back to fit below `SNAP = 1.02`.
- **Pinch.** `zoom_about` keeps the focal point fixed during a pinch. Hosts
  track the pointer position themselves, because the pinch protocol reports
  only the drift since the gesture began.
- **Limits.** `clamp_zoom` holds `offset` inside the picture's slack and passes
  `band` (the rubber-band stretch) through untouched. Only `zoomed` adds it at
  draw time.
- **Resampling.** Pictures are drawn with the Mitchell cubic resampler over a
  checkerboard.

Input in otto-files:

| Gesture | Path |
|---------|------|
| Wheel over text or a listing | `quickview_wheel` → `Session::scroll_by(rows)`; a plain row step, no momentum |
| Pinch over an image | `on_pointer_pinch_*` → `quickview_zoom_to` → `Session::zoom_to` |
| Two-finger scroll over a zoomed image | `Session::pan_wheel` feeds one `ScrollView` per axis (fling, spring-back, overlay bars); `pull_pan`/`push_pan` sync them with `Zoom` |
| Space / Escape | Space toggles. Escape unwinds one layer at a time: Get Info, search, Quick View, picker menu, picker, selection |
| Click outside the panel | Closes it, and the click goes no further |

The pointer reaches the panel two ways: on its own surface
(`install_quickview_pointer`, surface-local) and on the toplevel (a press
outside closes). The two share hit rects.

**Zoom does not decode again.** `Request.zoom` exists and `image::raster`
supports it, but `quickview::decode` always asks for zoom 1. An 8× zoom
enlarges the panel-sized decode, which is about 2× the panel's pixels.

**Accessibility.** An open session is published with `A11yTree::preview` and
takes focus.

## Video

Playback is not a decoder. The decode worker returns a `video/*` `Card` as
usual, and the host decides whether to play it:

1. `payload::is_video(preview)` must be true. That means a card whose first icon
   is a specific `video-*` name, so the choice follows the sniffed bytes and not
   the file extension.
2. `otto_media_kit::player::available()` must find `otto-media-worker`, looking
   at `OTTO_MEDIA_WORKER`, then next to the executable, then `PATH`.
3. `quickview::Video::open` calls `Player::open(path, options, wake)`, with the
   frame size limited to panel × scale (64–3840 × 64–2160). The card stays
   underneath as the fallback. Its `hero` would be the poster, but
   `media::video` never produces one, so a video shows its icon until the first
   frame arrives.

**In the Quick View panel** the video autoplays and is drawn into the panel's
own surface. `Session::video_key` is part of `quickview_key`, and the player's
`wake` requests a frame. `Video::pointer` handles input:
- a click on the picture or the play button toggles playback;
- mute switches the volume between 0 and 1;
- scrubbing pauses, runs fast keyframe seeks while dragging, then one accurate
  seek on release, and resumes playback if it was playing.

Moving to another file drops the session and with it the `Player`, which kills
the worker.

**In the preview column** the video opens paused. The worker delivers the
preroll frame so it isn't black. `PaneSurfaces::sync_preview_video` gives it
**its own subsurface**, so a 30 fps clip repaints only that surface and not the
toplevel or the cached `files-preview` picture, whose key leaves the video out
when it is on a surface. That surface:
- exists only in column view, and only while Quick View is closed;
- stays hidden until `VideoSnapshot::aspect()` is known (frame, then announced
  size, then poster);
- is sized by `view::preview_video_box` to `width / aspect + transport height`,
  and hidden if that would spill out of the viewport;
- has an empty input region. The toplevel hit-tests the same box through
  `Browser::preview_video_pointer`.

### otto-media-kit in brief

The full description is on [otto-media-kit](otto-media-kit.md). What a host
needs to know:

- **Two halves.** The library (`default-features = false` in hosts, no
  GStreamer) and `otto-media-worker` (feature `worker`, the default). The worker
  needs Rust 1.92 or later because of the gstreamer 0.25 crates; the library
  alone doesn't.
- **Descriptors.** The file on fd 3 and a memfd frame ring on fd 4: a 4 KiB
  header, then three RGBx slots. Commands go as lines on stdin, events come back
  as lines on stdout.
  - Commands: `play`, `pause`, `seek <ns> accurate|fast`, `volume <0..1>`,
    `quit`.
  - Events: `ready <w> <h> <duration_ns|-1>`, `frame <slot> <seq> <pts_ns>`,
    `position <ns>`, `playing`, `paused`, `ended`, `error <text>`.
- **Threads.** `Player` reads events on its own thread (`otto-media-events`),
  copies each announced slot out immediately, and calls `wake` after **every**
  event, so `wake` has to be cheap. Hosts draw from `Player::state()` and
  `frame()` snapshots, or pass them to `view::draw_frame` on another thread.
- **Lifecycle.** Dropping a `Player` sends `quit` and then kills the worker
  straight away. A worker that exits while loading, playing or paused turns the
  state into `Failed`. There is no `PR_SET_PDEATHSIG`: a worker whose host dies
  exits when stdin reaches EOF or a write to stdout fails.
- **Pipeline.** `playbin3` (or `playbin` as a fallback) with
  `videoconvert ! videoscale ! capsfilter(RGBx, [1,max], par 1/1) ! appsink
  sync=true max-buffers=2 drop=true`, playing
  `file:///proc/self/fd/3`. Audio goes through playbin's default sink inside
  the worker.

## Debugging

| Tool | Does |
|------|------|
| `otto-quickview --describe FILE` | Prints the payload the worker produces |
| `otto-quickview --render OUT.png FILE [--dark --page N --zoom Z --width W --height H]` | Draws the card through the same `otto_kit::preview::draw` |
| `otto-quickview --filmstrip OUT.png FILE` | Samples the entrance animation over a mock desktop |
| `otto-quickview --sandbox-selftest` | Reports which parts of the sandbox are in force |
| `OTTO_FILES_QV_TRACE=1` | Logs decode generations, restack order, and a line per painted panel frame (rect, resize, paint time, gap) |
| `OTTO_FILES_QV_AUTO=1` | Opens Quick View on the first entry, with no keypress |
| `OTTO_FILES_QV_CENTER=0` | Centres the panel on the window rather than the display |
| `OTTO_FILES_PANE_SUBS=1` | Puts each column on its own subsurface; also forces the panel onto a subsurface |
| `OTTO_QUICKVIEW_OPEN_MS`, `OTTO_QUICKVIEW_CLOSE_MS`, `OTTO_QUICKVIEW_BOUNCE` | Tunes the entrance and exit |
| `RUST_LOG=debug` | Forwarded to the worker, which logs the time and name of each decode |
| `OTTO_MEDIA_TRACE=1`, `GST_DEBUG=3` | Worker stderr and GStreamer debugging; see [otto-media-kit](otto-media-kit.md#debugging) |
| `cargo test -p otto-files --lib -- --ignored --nocapture thumbcache` | Runs the two ignored tests against your real thumbnail cache |

Unit tests cover:
- the payload round trip and rejection of bad input;
- decoder limits, text encodings and PDF page counts;
- `opening.rs` geometry;
- zoom and pan in both `otto_kit::preview` and `quickview.rs`;
- the thumbnail store's throttling and eviction;
- cache naming and mtime checks;
- the media-kit protocol and transport layout.

Nothing runs a real decode worker or media worker in `cargo test`. The CLI
above is the way to check them.

## Known gaps

**Thumbnails**

- **Cache writes.** Nothing writes to the shared thumbnail cache, including the
  failure markers it reads. The spec's "Quick View is a thumbnail producer" is
  not built (`Opened::mtime` and `len` are marked `dead_code`).
- **Cancellation.** Neither thumbnails nor previews are cancelled. Scrolling
  fast or arrow-keying quickly leaves workers running up to their deadline.
- **Invalidation.** The thumbnail epoch is global, so a thumbnail landing
  invalidates every Miller pane picture.
- **Loading cost.** Cached thumbnails load lazily with `Image::from_encoded`,
  so the PNG is decoded on first draw, on the render thread.
- **Scale and sampling.** Thumbnail sizing truncates fractional scales, and
  thumbnails are drawn with nearest sampling.
- **Failure markers.** `is_known_failure` reads its marker on the UI thread,
  once per file the store hasn't seen before.

**Decoding**

- **Sandbox.** The decode worker can still read the user's files: there is no
  seccomp filter or mount namespace.
- **Payload size.** The total payload isn't bounded, only each field of it.
- **Dispatch.** `may_generate` is decided by the entry's `Kind`, but the worker
  dispatches on the sniffed content. A file named like an image whose bytes are
  a PDF therefore reaches the PDF rasteriser while thumbnailing.
- **Formats.** Video has no poster frame, and only MP4/QuickTime headers are
  read; a `moov` atom at the end of a large file isn't found. Folders get no
  preview column. Text has no highlighting.

**Quick View and media kit**

- **Zoom.** Zooming never decodes again, so detail stops at about 2× the
  panel's pixels.
- **Frame ring.** otto-media-kit resizes the ring with `set_len` before the
  host has read the matching `ready`. If a stream's frame size **shrinks**
  mid-playback, the host can read a stale `frame` through its old mapping past
  the end of the memfd and get SIGBUS. The ring is also remapped on every
  `ready`, and `ready` follows every flushing seek, so scrubbing costs an
  mmap/munmap per seek.
- **Worker path.** `OTTO_MEDIA_WORKER` isn't checked for existence, so
  `available()` can report true and `Player::open` then fail.

## Where the spec has drifted

`specs/quickview.md` still carries parts of earlier designs:
- **Status.** It says "draft — nothing implemented".
- **Descriptors.** It describes three descriptors (file, memfd, status pipe).
  The code uses fd 3 and a stdout pipe.
- **Animation.** It says the compositor runs the animation through surface-style
  transactions. The client animates.
- **Dropped designs.** It keeps D-Bus-era text (`SetIndex`, `SetUris`, a
  five-second service grace, layer-shell overlays).
- **Zoom.** It promises that zooming decodes again.

Several code comments are stale for the same reasons: `opening.rs` and `lib.rs`
on the animation; `pane_surfaces.rs` claiming the panel's input region is empty;
comments in `view.rs` and `app.rs` about the panel being drawn into the window's
own surface; and `app.rs` mentioning D-Bus activation. If you change either
document, check this list first.
