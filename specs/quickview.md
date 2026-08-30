# Quick View

**Status:** draft — nothing implemented
**Related specs:** [file-browser.md](./file-browser.md) — in particular its *Shared foundations*, which defines the thumbnail cache and file-type detection this spec consumes — [file-picker.md](./file-picker.md), [launcher.md](./launcher.md), [portal-access-dialog.md](./portal-access-dialog.md), [settings-app.md](./settings-app.md), [context-menus.md](./context-menus.md), [localisation.md](./localisation.md)

## Summary

Press space on a selected file and see it, instantly, without launching the
application that owns it. Quick View is a **component the file views embed** —
the file browser, the save/open dialog, and the desktop's file view — drawn as a
subsurface of the window showing the files, so it is parented, stacked and
dismissed by that window rather than managing any of it itself. Everything that
interprets file bytes runs in a short-lived, sandboxed worker process that has
one file descriptor and no network.

## Goals

- Pressing space with a file selected shows that file's content over the host's
  window. Pressing space again closes it. Nothing else about the host changes.
- While the preview is open, moving the selection swaps its content in place,
  without a new surface and without a new process.
- The preview grows out of the selected row and shrinks back into it, because
  the host knows where that row is.
- All three hosts embed one implementation. Adding a fourth means calling the
  same library, not re-implementing a previewer.
- A malformed, hostile, or absurdly large file can waste one worker process and
  produce a blank preview. It cannot reach the network, write anything, outlive
  the preview, or affect the compositor's frame loop. Reading other files is
  *not* yet contained — see Sandboxing for what that costs to close.
- The panel appears within 50 ms of the keypress and shows real content within
  100 ms for anything already thumbnailed. It never appears empty and never
  appears late.
- Images, PDFs, and text documents are previewed properly — not as a card
  describing the file. A photograph can be zoomed into and stays sharp; a PDF
  shows its pages and can be paged through; a text file shows its text. These
  three carry the feature, and no other type may be traded against them.
- Adding a content type means writing a decoder, not a new surface, a new IPC
  shape, or a new drawing path.
- Previews and every file view's thumbnails come from one cache in one format,
  written by whichever process got there first. Three consumers share it with no
  daemon and no coordination, because the filesystem is the coordination.

## Non-Goals

- Editing, annotating, cropping, rotating, converting, exporting, printing, or
  sharing. Quick View is read-only, and the escape hatch is "open in the real
  application".
- File management: rename, delete, move, copy, permissions. That is the
  browser's job.
- Playback in v1. Audio and video show a poster and metadata; a transport is a
  later stage, not a hidden v1 requirement.
- HTML, EPUB, and anything else whose faithful rendering implies a browser
  engine. This is a permanent exclusion, not a deferral.
- Out-of-tree previewer plug-ins. The seam for new content types is in-tree; a
  loadable third-party previewer reintroduces exactly the untrusted-code problem
  the sandbox exists to solve.
- Remote or virtual locations. `file://` only — no network fetch, ever, by any
  part of the system.
- A second thumbnail cache, a content index, or full-text search.
- A preview process, a preview service, or a preview daemon. There is no bus
  name and nothing to activate.

## Behavior

### What this is, and what it is not

Three things could have been built. Only one of them is Quick View.

**It is not a compositor feature.** A previewer's entire job is parsing
untrusted bytes, and the compositor is a single process whose death takes the
session with it. That argument survives everything below: decoding happens in a
separate sandboxed process no matter who draws the result.

**It is not a standalone application either, and an earlier draft of this spec
was wrong to say so.** That draft reasoned from the sandbox to a separate
*application*, which does not follow: parsing untrusted bytes requires a
separate *decoder*, and says nothing about where the UI lives. Building it that
way cost real things — the preview could not be stacked above the window that
opened it, could not know where the file was on screen, and had to hand-manage
focus and dismissal that a parent window gives for free.

**It is a component the file views embed.** The preview is a subsurface of
whichever window is showing files. That is forced by the protocol and is the
point: `wl_subcompositor.get_subsurface` takes two surfaces from the *same*
client, as do `xdg_popup` and `xdg_toplevel.set_parent`, so "parented to the
file view" and "separate process" are mutually exclusive. Choosing parented
dissolves the hardest open problem in this spec — the host already knows where
the row is in its own surface, so the entrance can grow out of the file with no
protocol, no compositor change, and no `anchor` in screen coordinates that
nobody could compute.

Three hosts embed it: the file browser, the save/open file dialog, and the
desktop's file view. The `otto-quickview` binary remains for previewing a path
named on a command line, and for being the sandboxed decode worker.

The layers, and the split is load-bearing:

1. `otto_kit::preview` — draw functions over an already-decoded payload, plus
   hit-test helpers. Pure Skia: a canvas, a rect, a payload, a theme. No
   `AppContext`, no `wayland-client`. A host draws it into its own surface, and
   the compositor can draw the same thing server-side for a dock thumbnail.
2. `otto_kit::filetype` and `otto_kit::thumbnails` — type detection and the
   shared on-disk thumbnail cache, used identically by all three hosts.
3. `otto-quickview` — the library: the sandboxed decode worker, the payload
   wire format, and the entrance geometry. Hosts call it; they do not
   re-implement it.

**Embedding obligation.** The worker is this binary re-executed, and the binary
*is* the host once the library is embedded — so every host must call
`run_worker_if_requested()` first thing in `main`, before any thread or Wayland
connection exists. Without it a preview would re-exec the host as a host, which
starts a second file browser instead of decoding anything.

### The embedding contract

This is the contract; everything else in this spec is downstream of it. It is a
Rust API rather than a wire protocol, because the preview runs inside its host.

```rust
// Once, first thing in main — before any thread or Wayland connection.
otto_quickview::run_worker_if_requested();

// Per preview: open the file, decode it in a contained worker.
let opened = otto_quickview::open(path)?;                  // refuses FIFOs, devices
let preview = otto_quickview::decode_path(path, &request); // always returns something

// Draw it into the host's own surface, at whatever rect the host chose.
// `zoom` magnifies an image and drags it about; a host with no zoom gesture
// passes `Zoom::FIT` and gets what it got before there was one.
otto_kit::preview::draw(canvas, rect, &preview, &theme, first_row, zoom, &resolve_icon);
let layout = otto_kit::preview::layout(rect, &preview, first_row, zoom);
layout.row_at(x, y);                                       // hit-testing

// Gesture geometry, so every host clamps the same way.
let zoom = otto_kit::preview::zoom_about(rect, &preview, zoom, scale, focus);
let zoom = otto_kit::preview::clamp_zoom(rect, &preview, zoom);

// The entrance, from the row the user pressed space on.
let entrance = otto_quickview::opening::entrance(row_rect, panel_rect);
```

Rules the host must honour:

- **Call `run_worker_if_requested()` first.** The worker is the host binary
  re-executed; without this the preview re-execs the host as a host.
- **Decode off the UI thread.** `decode_path` blocks until the worker answers or
  its deadline expires. A host that calls it inline stalls its own frame loop
  for as long as the file takes.
- **Tag decodes with a generation and drop stale ones.** Arrow-keying is faster
  than decoding; a result that arrives after the selection moved must be
  discarded, not painted.
- **Never show one file's preview under another file's name.** The moment the
  selection moves, the panel drops what it was showing — content, scroll, zoom
  and pan — and says it is working until the new decode lands. Decoding costs a
  process spawn, so the old content would otherwise sit there for long enough to
  be read, with nothing on screen marking it as the wrong file. A waiting line
  is the honest state; a stale picture is a lie the user cannot see through.
- **Never interpret file bytes in the host.** Everything the host receives is a
  `Preview` — validated text, bounded rows, or a pixel buffer whose dimensions
  have already been checked against its length.

What the host owns, because it is better placed to: the panel's rect, the
keyboard (it never loses focus, so there is nothing to hand back), dismissal,
and the selection. There is no `Navigate`, no `Activated` and no `Closed` — the
host already has the keypress, already knows the selection, and already knows
when it closed the panel.

### The keyboard, and the panel

The host never loses focus, which removes the hardest part of the old design.
There is no hand-back: the host already has the keypress.

- **Space, Escape** — close the preview.
- **Arrows, Home, End, Page Up/Down** — the host moves its own selection and
  tells the preview the new path. When the content has pages, Page Up/Down
  paginate instead.
- **Enter** — the host opens the file in its default application and closes the
  preview.
- **`+` / `-` / `0`** — zoom, which is the preview's own business; past fit,
  the arrows pan rather than moving the selection. *Not implemented yet — the
  pointer gets there first, see below.*

### The pointer, and zooming an image

Zoom applies to **images only**. Text, listings and cards are laid out to fit
by construction and go on scrolling exactly as they did; a pinch over one of
them does nothing rather than something surprising.

- **Pinch zooms**, centred on the focal point between the fingers, so the
  gesture grabs the picture rather than the panel. The protocol
  (`zwp_pointer_gesture_pinch_v1`) reports only how far that point has drifted
  since the gesture began, so the host carries the pointer's last position over
  the panel and adds the drift to it.
- **The range is fit → 8×**, measured against the *fitted* size rather than the
  source's own, so a pinch means the same thing on a thumbnail and on a
  photograph. Past the top it stops; near the bottom it snaps back to fit
  exactly, because there is no useful state between "the whole picture" and
  "visibly zoomed in".
- **A two-finger scroll pans a zoomed image**, and scrolls everything else —
  including an image at fit, which has nothing to pan to. Panning stops with
  the picture still covering the box it is looked at through: it can never be
  dragged off and leave the panel empty.
- **The pan is a scroll view**, one per axis, sharing the picture's box as
  their viewport. Panning a picture *is* scrolling — the box shows part of
  something bigger — so it gets what every other scroll in the toolkit gets: a
  gesture that goes on gliding after the fingers lift, the same distance
  covered per point of finger travel, ends that stretch and spring back, an
  overlay bar per axis that can be grabbed to move the picture, and a notched
  wheel that steps without throwing.
- **The bars come up with the zoom**, not only with a pan. Zooming in is the
  moment the picture stops fitting, and how much of it is now off the sides is
  what a bar is there to say; they fade out again on their own. Only an axis
  with slack raises one.
- **The stretch travels in `Zoom::band`**, apart from `offset`, because
  nothing may clamp it: `clamp_zoom` holds the offset to the picture's own
  slack and carries the band through untouched, and only the drawing adds it
  in. So a picture pulled past its stop is drawn past it, while everything
  that asks how far there is left to pan still gets an answer inside the
  picture's limits. A zoom snapped back to fit drops the band with it.
- **Zoom belongs to the picture, not to the panel.** Changing file or closing
  the preview puts it back to fit.

The geometry — the clamp, the snap, the pan limits and the focal-point
transform — lives in `otto_kit::preview` beside the layout, not in the host, so
the browser and the file picker cannot end up with different ideas of how far a
picture zooms. The host owns only the state: one `Zoom` per open session, and
the two scroll views that move it. The views own the pan while a gesture or a
fling is running and the `Zoom` is read back from them after every step; a
pinch, which places the picture without scrolling it, writes the other way.

### The panel

A subsurface of the host's window, positioned by the host: centred over the
file view, sized to the content and clamped to 80% of the host's window with a
floor of 400×300. Content smaller than the floor is centred in it rather than
upscaled.

It carries the file's name, its size and modification date, a close affordance
on the trailing edge, and "Open with <app>". The close is a single control, not
the toolkit's traffic lights: those belong to a window — something you minimise,
zoom and arrange — and this has exactly one exit.

The material is the compositor's, declared through surface-style: the popup
token, background-blurred, with the desktop's corner radius and shadow. It reads
as the same kind of surface as the bar's menus and the launcher's card, because
it is.

Dismissal: space, Escape, the close control, a click outside the panel, or the
host closing it for any reason of its own. The embedded panel has no focus-loss
rule of its own — the host's window losing focus is the host's business, and a
preview inside it simply goes when the host says so. The file browser makes
exactly that call: it closes on `wl_keyboard.leave` for its toplevel, which is
also the only signal a client gets that expose opened, since a subsurface is out
of reach of the compositor's popup dismissal.

### Opening and dismissing

**The opening is animated, and it opens out of the file.** The card grows and
fades from `anchor` — the rect of the item the user pressed space on — to its
resting rect. That motion is the thing that makes the preview feel attached to
the file rather than summoned on top of it, and it is why `anchor` is the item's
rectangle and not the caller's window.

- **Geometry** — anchor rect → resting rect, ~260 ms, spring with a slight
  overshoot. Deliberately less bounce than the launcher's card: this surface is
  much larger, and on a large surface the same overshoot stops reading as life
  and starts reading as wobble.
- **Opacity** — 0 → 1 in ~90 ms, ease-out, and therefore finished long before
  the geometry settles. The card should be legible almost at once and still be
  arriving; the two are out of step on purpose.
- **No anchor** — with an empty `anchor` there is nothing to grow out of, so the
  card scales in place from ~0.96, which is the launcher's entrance. Same code
  path, different starting rect.
- **Chrome fades in with the card** — the title strip and the close dot are
  absolute (30 pt tall, a 17 pt dot) while the card grows from the file's own
  row, so on the first frames they would *be* the card. They stay away below
  two strips' height (or 140 pt of width) and reach full strength by four
  strips (240 pt), both ends well under the smallest panel that ever rests, so
  a panel at rest always has its titlebar. The content's box is unchanged
  either way — the strip's room is reserved whether or not the strip is drawn
  — so nothing under it moves when it appears.
- **Dismissal reverses it**, and faster: ~160 ms back toward the anchor,
  ease-in, with the opacity going in the same 90 ms. Leaving does not bounce —
  there is nothing to settle into. If the anchor is no longer valid (the caller
  scrolled, moved, or closed), the card fades in place rather than flying to a
  stale rectangle.

Two things the opening must **not** do:

- It must not wait for content. The window is mapped and animating inside 50 ms
  with the file's name and icon on it; the decoded content arrives during or
  after the motion and does not restart it. An animation that waits for a
  decoder is an animation whose duration is set by the file.
- It must not re-run on `SetIndex`. Arrow-keying through a directory swaps
  content inside a card that is already open and already still — the card keeps
  its place and its entrance, and only what is inside it changes. Replaying the
  entrance per keystroke would be unusable. What changes immediately is the
  title and the content: the name is the file now selected, and the content is
  the waiting line until that file's decode lands.

The animation is declared through the compositor's surface-style transactions —
the same mechanism otto-launcher's card uses — so it runs compositor-side and
costs the previewer no frames. That matters here more than it does for the
launcher: this process is also supervising a decode, and the entrance must not
share a thread with it.

Dismissal, for ephemeral sessions: space, Escape, the close affordance, a click
outside the card, focus loss (when `close_on_focus_loss`), `Close` from the
caller, or the caller vanishing. Every path emits `Closed` with its reason.

Modality: Quick View is **not** modal. It takes the keyboard while it is up
because it is an overlay, but it does not block the caller, does not prevent
the caller from repainting, and imposes no ordering on anything. The
anti-spoofing requirements that apply to a permission grant
([portal-access-dialog.md](./portal-access-dialog.md)) do not apply here, because
nothing is being consented to.

The rule that keeps this from eroding, on **both** surface paths: nothing in
this spec may be phrased as "the user cannot do X while the preview is open".
Quick View holds the keyboard and it may be buried by the window that opened it;
those are the only two facts about its relationship to other windows. Any
requirement that needs more than that is a requirement Otto cannot currently
enforce, and writing it down as though it could is how a spec acquires a
dependency nobody costed.

### Content types

Ranked by what v1 ships, and honest about the ones needing a decoder Otto does
not have.

| Type | v1 | Needs |
|---|---|---|
The three that matter most are images, PDF, and text documents. Those three are
**fully previewed in v1** — real pixels, real pages, real text — and the rest of
the table is arranged around not compromising them.

| Type | v1 | Needs |
|---|---|---|
| **Image** — PNG, JPEG, WEBP, GIF (first frame), BMP, ICO | Full: scaled decode, zoom to 1:1 and beyond, pan | Skia's own codecs, already linked |
| **Image** — SVG | Full, re-rendered at each zoom level, so it stays sharp | Skia's own SVG module — `skia-safe` is already built with `features = ["svg"]` |
| **PDF** | Full: rendered pages, page navigation, zoom | An external rasteriser, exec'd — see below |
| **Text and source code** | Full: monospace layout, line numbers, encoding sniff, wrap toggle. No syntax highlighting in v1 | Nothing |
| Directory | Full: entry count, total size, a grid of child icons and image thumbnails | Nothing |
| Lottie animation | Full, played | Skottie — already enabled and already used by otto-kit |
| Archive — zip, uncompressed tar | Listing only: names, sizes, dates, entry count | Nothing (see below) |
| Audio | Metadata card: title/artist/album/duration, plus embedded cover art | Tag parsing by hand; PCM decode deferred |
| Video | Metadata card: dimensions, duration, codec, plus the container's embedded poster when present | Frame decode deferred |
| HEIC, AVIF, camera RAW | Not previewed — shown as an unsupported card naming the type | Deferred |
| HTML, EPUB, Office documents | Never | — |

Notes on the ones that look like they need a crate and do not:

- **Archive listing needs no decompression.** A zip's central directory carries
  every entry's name, size and date in plain form at the end of the file; a tar
  is 512-byte headers. Listing is a parse, not an extraction. A *compressed* tar
  (`.tar.gz`, `.tar.zst`) cannot be listed without inflating and is therefore
  shown as a plain file card in v1.
- **Scaled image decode needs Skia's `Codec`, not `Image::from_encoded`.**
  Sample-size decoding is the difference between previewing a 200 MP image and
  refusing to.
- **SVG and Lottie come from Skia, not from `resvg`.** Both modules are already
  compiled into `skia-safe` here, so the vector types cost nothing. Quick View
  deliberately does not become a new consumer of `resvg`/`usvg`, since whether
  Skia's SVG can replace them outright is an open question elsewhere in the
  project and a new caller would only make removing them harder.
- **Skia does not help with PDF.** Its PDF support is a document *writer*; there
  is no reader and no rasteriser. This is worth stating because "Skia already
  does PDF" is the natural wrong assumption.

### PDF, and the external rasteriser seam

PDF is rendered in v1 by **exec'ing an existing rasteriser inside the decode
worker** and reading a PNG or PPM back from its standard output, which Skia then
decodes like any other image. No PDF library is linked into anything Otto
builds.

The worker tries, in order, and uses the first one present:

| Command | From | Notes |
|---|---|---|
| `pdftoppm -png -r <dpi> -f N -l N` | poppler-utils | The expected path. Poppler is a de-facto universal desktop dependency — it is already installed on this machine, pulled in as a plain `poppler` package. |
| `pdftocairo -png -r <dpi> -f N -l N` | poppler-utils | Same package; better output on some documents. |
| `mutool draw -F png -r <dpi>` | mupdf-tools | AGPL, which constrains *linking* — exec'ing a separate program does not create a derived work, so it is available here in a way `libmupdf` is not. |
| `gs -sDEVICE=png16m` | ghostscript | Last resort; slowest. |

If none is present, PDF falls back to the metadata card, and the card says which
package would enable page rendering rather than silently looking broken.

The file is handed to the rasteriser as an already-open descriptor via
`/dev/fd`, so the child never resolves a path of its own and cannot be
redirected to a different file between the worker's `fstat` and the child's
open. Page navigation re-execs for the requested page; at roughly 50–100 ms per
page this stays inside the interaction budget, and the current page is kept
while the next renders so paging never blanks the view.

**This generalises, and is the reason to prefer it over a linked library.** The
same seam — a table of external renderer commands, tried in order, run inside
the existing sandbox — is how video poster frames arrive later
(`ffmpegthumbnailer`, `gst-launch-1.0`) without GStreamer entering the default
build, and how any future format can be supported by a distribution that ships
the right tool. A new format becomes a table row.

Rejected alternatives for PDF: `pdfium-render` bundles a large C++ blob into
Otto's build for a preview feature; `dlopen`ing a system libpdfium avoids the
build cost but depends on a library most distributions do not package, so it
would degrade far more often than poppler does; pure-Rust `lopdf`/`pdf` parse
structure but do not rasterise, so they cannot produce a page image at all.

Decoders Otto genuinely lacks, with candidates and their cost:

- **Video poster frames.** GStreamer is already in the workspace, but only in
  `otto-rdp`, which pins Rust 1.96 and drags the whole GStreamer tree. Taking it
  into the default build would raise the workspace MSRV for a preview feature.
  **Recommendation:** an optional `preview-video` feature; without it, the
  metadata card. FFmpeg bindings are a larger version of the same trade and are
  not recommended.
- **Audio playback and waveforms.** Needs a PCM decoder (`symphonia` is the
  pure-Rust candidate, one crate per codec family) plus an output path. Deferred
  wholesale; v1 deliberately gets most of the value from tags and cover art,
  which need neither.
- **Syntax highlighting.** `syntect` brings a regex engine and megabytes of
  syntax definitions; `tree-sitter` brings a crate per language. **Recommendation
  when this is built:** a hand-written token scanner covering strings, comments,
  numbers, and a keyword set per language — a few hundred lines, correct enough
  for a preview, and consistent with the project's dependency posture.

### The previewer seam

A previewer is three pieces, and only the first two are ever written for a new
type:

1. **A claim** — the set of MIME types it handles, with a priority. Detection is
   `otto_kit::filetype`, defined in
   [file-browser.md](./file-browser.md#2-file-type-detection): two calls
   answering two questions. `mime_for_name(name)` is the type of *record*, and
   drives the icon, the browser's Kind column, and default-application
   association. `sniff(bytes)` is a bounded magic check over the first 4 KB.

   **A previewer dispatches off `sniff`, never off the name.** A file named
   `.png` that is not one must not reach the PNG decoder; the sandbox would
   contain the consequences, but the correct preview is the one matching the
   bytes. Display, conversely, follows the name — so the two calls cannot
   disagree, because they are not answering the same question. Where the answers
   differ and it is useful, Quick View says so on the card ("named `.png`, is
   not one").

   The type hierarchy from `subclasses` is what lets the text previewer claim
   everything that is a `text/plain` subtype without enumerating languages.
2. **A decoder**, running worker-side: given one read-only file descriptor and a
   budget, produce a `PreviewPayload` or fail.
3. Nothing else. Drawing comes from the payload.

`PreviewPayload` is a **closed** set in v1, and this is the point of the design:

- `Pixels` — premultiplied RGBA at a stated size, with an optional intrinsic
  size for zoom, and an optional page count. Zoom is measured against the
  fitted rect rather than this intrinsic size; the intrinsic size says how far
  a decode can be zoomed before it starts inventing detail.
- `Text` — bounded, validated UTF-8 with optional style spans.
- `Rows` — a table (archive entries, directory listing): name, size, date, an
  icon key.
- `Card` — key/value metadata plus an optional `Pixels` hero image.
- `Unavailable` — a reason to display.

A new content type adds a decoder that produces one of these. It does not add a
payload variant, an IPC message, a window mode, or a draw path. When a variant
genuinely must be added, that is a deliberate change to `otto-kit::preview` and
to this spec — not a routine extension.

### Sandboxing

Nothing in the Quick View application process ever interprets file content. For
each file to be previewed, the application:

1. Opens the path itself with `O_NONBLOCK`, `fstat`s it, and refuses anything
   that is not a regular file or a directory — no FIFOs, devices, or sockets,
   which can block on open or on read.
2. Creates a `memfd` for the result and a pipe for status.
3. Spawns `otto-quickview --decode-worker`, passing exactly three descriptors:
   the file (read-only), the memfd (write), the status pipe (write). No other
   descriptor, no Wayland socket, no D-Bus socket, no environment beyond a
   minimal set — `PATH`, `RUST_LOG`, and `LANGUAGE`. The locale is in that set
   because the worker writes the strings a person reads (why a preview is
   unavailable, a card's facts) and holds no bus connection to ask the
   compositor which locale is in use; a locale tag is not a capability.

The worker, before touching a byte of the file: drops to `chdir("/")`, sets
`PR_SET_NO_NEW_PRIVS`, unshares the network namespace (and a user namespace
where the kernel allows it), and applies `RLIMIT_AS`, `RLIMIT_CPU`,
`RLIMIT_NOFILE`, and `RLIMIT_FSIZE = 0`. It has no path to open anything: it
never received one.

The parent enforces a wall-clock deadline independently and kills a worker that
overruns. A worker that crashes, overruns, or reports failure produces
`Unavailable` with a reason, and nothing else happens.

**What a malformed file cannot do:** write or grow any file anywhere; make any
network connection; talk to the compositor, the display server, or the session
bus; escalate privileges; consume more than its address space or CPU budget;
survive its own preview; or take down anything other than one worker process,
which the parent expects to lose.

**What it can do:** produce a wrong or blank preview, occupy one core for the
length of the deadline, and — this is a real gap, not a rounding error — **read
other files the user can read**. The rlimits and the network namespace are not a
filesystem jail, and `chdir("/")` only removes relative-path surprises. Closing
it needs one of two things, and neither is free:

- a **seccomp filter** denying `openat`, which is cheap for the in-process
  decoders but breaks the PDF path, since an exec'd rasteriser must open its own
  libraries;
- a **mount namespace with `pivot_root`**, which is the correct answer and must
  then bind in enough of `/usr` for those rasterisers to run.

Until one lands, the honest containment story is: one process, one descriptor,
hard budgets, no network, no writes — and reading is not contained. This is
stated rather than glossed because a security property nobody measures is a
security property nobody has.

**The claim is testable.** `otto-quickview --sandbox-selftest` applies the real
sandbox in a real child and reports what is in force, so this section can be
checked rather than believed. Any change to the sandbox must keep that output
matching this text.

### Performance

- **Panel up within 50 ms** of the keypress, unconditionally — it is drawn into
  the browser's own surface, so there is no window to map and nothing to wait
  for. The first frame carries the file's name and says the preview is opening;
  the decode fills the card in when it answers. It is never a delayed panel, and
  a keystroke never appears to have done nothing.
- **Real content within 100 ms** for anything already in the thumbnail cache,
  which is the common case when browsing a directory the browser has scrolled
  through. Cold decode fills in when it lands, replacing the placeholder without
  a resize jump.
- **Cold process start ≤ 150 ms** to a mapped window. Repeat presses in one
  browsing session pay it once: the service stays alive for a few seconds after
  its last session closes, then exits. It is activatable and not a daemon, per
  the launcher's precedent — but a five-second grace is the difference between
  arrow-keying feeling instant and feeling like process spawns.
- **Precomputed:** thumbnails only. There is no index, no background scan, and
  no speculative decode of files that are merely adjacent to the selection.
- **A 200 MP image** is never decoded at full resolution. The worker decodes at
  the smallest sample size that still exceeds twice the window's pixel size, and
  refuses above a fixed pixel budget when the codec cannot decode scaled, in
  which case the file gets a metadata card. **Zooming re-decodes** rather than
  upscaling the fit-sized decode: past roughly 1:1 the worker decodes the
  visible region at a finer sample size, so zooming into a large photograph
  shows detail instead of blur. The previous frame stays on screen while that
  runs. This matters because images are a primary type — a previewer that goes
  soft the moment you look closely has not previewed the image.
- **The doubling is the host's, and happens once.** The size in a request is
  already about twice the panel's pixels; a decoder renders at the size it was
  asked for and does not double it again. Applied at both ends the factors
  compound to four, which for a PDF meant rasterising a page at four times the
  panel and waiting seconds for detail no panel can show.
- **A PDF page is rasterised no wider than 2048 px**, whatever the panel asks
  for. A page is re-rendered from vectors at whatever size is requested and the
  cost climbs faster than the area, so this is a ceiling in its own right
  rather than a safety valve — comfortably above a full-screen panel's own
  pixels on a HiDPI display, and the difference between a preview that opens
  and one that is waited for.
- **A 2 GB video** is never read whole. The worker reads container headers and
  the index only, under a hard cap on total bytes read, and produces a poster or
  a card. The same cap applies to every metadata-only path.
- **The compositor is never in the path.** Quick View is a separate process and
  decoding is in a grandchild of it; the compositor's only involvement is
  mapping and compositing a surface like any other client's. No decode, no file
  I/O, and no parse ever runs on a compositor thread. Memory pressure is the one
  channel by which a preview could hurt the session, which is what the worker's
  `RLIMIT_AS` is for.

### The shared foundations, and what Quick View does with them

The thumbnail cache and file-type detection are defined once, in
[file-browser.md](./file-browser.md#shared-foundations). They are not restated
here. What matters on this side:

**Quick View is a thumbnail producer, not only a consumer.** Its worker already
performs a scaled decode, in a sandbox, on the file the user is most interested
in — so it writes that result into the cache under the shared rules, and the
browser is faster afterwards for a preview having been opened. Discarding it
would mean the browser decoding the same file again, less safely. The one
constraint the cache imposes on a producer: write **only** the four standard
size buckets. An off-size image in that tree is invisible to every other
implementation and is not a thumbnail.

**The cache serves the first frame only.** It is the sub-100 ms placeholder. The
real preview is always a fresh scaled decode at roughly twice the window's pixel
size, in the worker. The cache therefore never holds full-resolution content and
never needs a "decode this at an arbitrary size on someone's behalf" call —
neither now nor later.

**Every file view embeds the previewer directly.** It keeps its own selection
and its own keyboard, calls `decode_path` off its UI thread, and draws the
result with `otto_kit::preview::draw`. There is no interface between them beyond
the Rust API.

**The worker protocol stays private.** Hosts receive a `Preview`, never bytes
from the file. The wire format between parent and worker is an implementation
detail of the library and may change without any host noticing.

### Where the panel sits

By default the panel is centred on the **window** it was opened from, and grows
out of the file's icon. Centring it on the **display** instead is opt-in
(`OTTO_FILES_QV_CENTER=1`), and is a genuine trade, not a strict improvement:

- The panel is a subsurface of the browser, so its position is relative to that
  window, and a client is never told where its own window sits. It asks:
  `request_output_frame` answers with the output's *usable* rect — the display
  minus the dock and any exclusive zones — expressed in the coordinates the
  client already positions in. See
  [surface-output-placement.md](./surface-output-placement.md).
- Because the answer comes back in window coordinates, the entrance still runs
  from the file's icon. Anchor and resting place are in the same space, so
  nothing has to be translated and the opening reads the same as the
  window-centred one; only where it lands differs.
- **The resting rect is worked out once per opening and once per closing**, not
  every frame. A window that moves while the panel is up therefore carries the
  panel with it, which is what a subsurface does anyway; recomputing per frame
  instead let a stale window-relative answer walk the panel off the display.
- **The panel is stacked above every column** with `wl_subsurface.place_above`
  against each sibling in turn, restated while the panel is up rather than
  assumed from creation order — a pooled column that is shown again still holds
  its old place in the stack.

## Constraints & Edge Cases

- A file that changes or is deleted while previewed: the preview is re-decoded
  on modification-time change if the window is still open, and shows a "no
  longer available" card on deletion. It never shows stale content silently.
- A directory passed to `Open` previews as a directory, it does not descend.
- A symlink is followed and the target is previewed; a broken symlink and a
  symlink loop both produce `Unavailable` rather than an error dialog.
- A zero-byte file, and a file the user cannot read, both produce a card stating
  which — these are the two most common "nothing appears" cases and must be
  distinguishable.
- A file whose name and content disagree previews as its content and is labelled
  as its name, with the mismatch noted. A file the sniff cannot identify at all
  previews as a plain file card, never by falling back to its extension.
- `SetUris` may arrive purely to extend the window around a moving selection,
  with the currently shown file unchanged. That must not re-decode anything.
- `SetIndex` arriving faster than decodes complete must not queue decodes. The
  session keeps at most one in-flight worker; a superseded decode is killed, not
  awaited.
- The preview must survive the invoking window being minimised, moved to another
  workspace, or closed. A closed caller ends the session; the other two do not.
- Multi-output: the preview appears on the output implied by `anchor` and stays
  there. It does not follow the pointer.
- Under the windowed development backend there is no layer-shell overlay
  guarantee different from production, but the dimming behind an overlay covers
  only the compositor's own output — acceptable, and worth stating so it is not
  reported as a bug.
- Two callers racing `Open`: last wins, first gets `Closed(replaced)`. There is
  no queue, because two simultaneous previews are never what anyone wanted.
- The service must exit cleanly if it is activated but never receives an `Open`
  — a caller that crashes between activation and its first call must not leave a
  process behind.
- **`xdg_toplevel.set_parent` buys nothing here.** Otto does not read toplevel
  parentage today — window order comes from workspace stacking alone — so a
  toplevel-based preview cannot be kept above the application that opened it,
  and there is no protocol-level modality to fall back on. This is why the
  ephemeral path is a layer-shell overlay and not a parented toplevel: it needs
  to be on top, and the overlay layer is the only way to actually be on top. The
  non-ephemeral path is an ordinary window and wants no special stacking, which
  is the whole reason the two paths differ. Neither path may be "improved" by
  adding `set_parent` unless Otto first grows toplevel parent stacking, which is
  compositor work this spec does not assume and does not require.
- Fullscreen from an overlay surface and fullscreen from a toplevel are
  different mechanisms; `f` must work identically in both, or be absent from the
  overlay variant rather than working differently.

## Rationale

**The invocation surface was designed first, and the rest fell out of it.**
An earlier draft made that surface a typed D-Bus interface, and got a working,
verified implementation out of it — and it was still the wrong shape, because a
separate process cannot be parented, cannot learn where the file is on screen,
and has to hand-manage focus and dismissal. Choosing the embedding API instead
deleted three problems rather than solving them: the `Navigate` hand-back, the
surface-position prerequisite, and the overlay's lifetime.

**The keyboard is handed back rather than shared.** The alternative — Quick View
declining focus so the browser keeps its arrows — leaves nobody able to close
the window with a key, and makes the second space press unroutable. Taking focus
and forwarding navigation puts one component in charge of the keyboard at a
time, which is the only arrangement that stays comprehensible when a third
caller appears.

**Decoding is in a grandchild process, not a thread.** A thread shares the
address space, so a heap corruption in a JPEG decoder is a compromise of the
process that holds the user's file descriptors and their Wayland connection.
Process isolation is also what makes the timeout enforceable: a runaway thread
cannot be killed, a runaway process can. The cost is one `fork`+`exec` and a
memfd copy per preview, which fits inside the 100 ms budget comfortably.

**Two surface types, not one.** An overlay that cannot be left open is wrong for
`otto-quickview report.pdf`; an ordinary window that must be dismissed by hand is
wrong for space-on-selection. Both behaviours already exist in Otto's toolkit —
layer-shell for the launcher, xdg-toplevel for the settings app — so supporting
both costs one branch at surface creation and nothing at draw time.

**v1 ships images, PDF and text properly, and everything else honestly.** Those
three are what people press space on; a previewer that shows a photograph but
describes a PDF has failed at a third of its job. Everything else — audio,
video, archives — gets a metadata card, which is better than nothing and better
than distorting the build for a convenience feature.

**PDF is exec'd, not linked, and that turned out to be the better design
anyway.** The obvious route was a PDF library in the build, which is a large C++
blob for one file type. But the worker is *already* a separate, sandboxed,
rlimited process spawned per preview — so running a rasteriser that already
exists on the system costs one more `exec` in a place that was doing an `exec`
regardless. The dependency is a package a distribution almost certainly already
installed, not a crate in Otto's tree, and it degrades to a card with a useful
message rather than to a build-time choice nobody can change afterwards. It also
sidesteps MuPDF's AGPL, which restricts linking and says nothing about running a
program. Generalising it to a table of renderer commands is what lets video
poster frames arrive later without GStreamer.

**The payload set is closed on purpose.** An open plug-in interface would let a
new content type invent a new drawing path, and the previews would stop looking
like each other within a release. Forcing every type through five payload shapes
is what keeps a PDF card and an audio card visually the same object.

**The thumbnail cache is a filesystem layout, not a service.** A service means a
process to start, a lifetime to manage, and a second failure mode for something
whose whole job is to make things faster. The freedesktop layout already
specifies naming and invalidation, is already populated by other applications,
and lets Quick View and the browser cooperate without either knowing the other
exists. Both sessions designing this reached that conclusion independently,
which is some evidence it is the obvious one.

**Display follows the name; decoding follows the content.** These look like the
same question and are not, which is why detection is two calls rather than one
with a precedence rule. A single answer forces a choice between an icon that
flickers when an empty file's sniff comes back inconclusive, and a decoder fed
by an attacker-chosen extension. Two answers make both correct, and make the
disagreement itself displayable.

**Selections are sent, directories are not.** Once navigation round-trips
through `Navigate` → `SetIndex`, Quick View never needs the directory — and it
must not have it, because the caller's sort order, filter and hidden-file state
are the caller's alone. The 256-URI window exists so that selecting a very large
number of files does not put a very large array on the bus for no benefit.

## Resolved decisions

Settled with the file browser/picker session; recorded so neither side reopens
them.

- Quick View is a separate binary, not a compositor feature and not a library
  the browser embeds. The cache contract is therefore cross-process — and is a
  filesystem layout, so that costs nothing.
- The browser has **no inline preview pane in v1**, so no decoded payload ever
  leaves this application and the worker protocol stays private. A pane would
  need either a second sandboxed decoder inside the file manager — which is
  precisely what a separate previewer process exists to avoid — or a public
  decode call. Neither is v1.
- Quick View writes into the shared thumbnail cache, in the four standard size
  buckets only.
- The cache never holds full-resolution content, and gains no arbitrary-size
  decode call.
- `otto_kit::filetype` is two calls: name → type of record, bytes → sniffed
  type. Previewer dispatch uses the sniff. Display uses the name.
- Shared MIME-info **is** consulted, contrary to this spec's first draft:
  `globs2` and `subclasses` are line-oriented plain text, so the full database
  costs a file read and a hash map — no XML parser, no `mime.cache` binary
  format, no new crate. A hand-rolled table of ~30 types was the wrong call; it
  cannot name what a portal filter or a Kind column will legitimately ask for.
- The invocation contract in this spec is adopted verbatim by the browser,
  including the focus model.
- The preview is embedded by its hosts rather than run as a process of its own.
  A subsurface's parent must belong to the same client, so parented and separate
  are mutually exclusive, and parented wins.
- Hosts decode off their UI thread and drop stale results by generation.

## Open Questions

- **Is the five-second exit grace right?** It is the whole difference between
  arrow-key navigation feeling instant and feeling spawned, but it contradicts
  the launcher's "no daemon, nothing to keep warm" principle. If that principle
  is absolute here too, `SetIndex` latency needs a different answer.
- **What is a "text document"?** Plain text and source code are fully previewed
  in v1. If the intent includes `.odt` and `.docx`, that is a different and much
  larger feature: both are zip containers whose text lives in compressed XML, so
  it needs an inflate implementation and an XML parser before a single word can
  be shown, and a faithful rendering needs layout on top of that. Extracting the
  embedded preview image both formats already carry is a far cheaper middle
  option and would show *something* for both. Not started either way.
- **Where does "Open with…" get its application list?** The browser has since
  settled that it writes `mimeapps.list` directly rather than routing
  associations through the settings service, which makes `mimeapps.list` plus
  `freedesktop-desktop-entry` the authoritative pair and leaves the compositor's
  `default_apps.rs` as something else — a fallback, or a thing to fold in.
  Quick View only needs this on the non-ephemeral path, since an ephemeral
  session emits `Activated` and lets its caller launch. Narrower than it was,
  but not closed.
- **Does the compositor need any new surface role for this at all?** The design
  assumes layer-shell overlay with exclusive keyboard is sufficient. If dimming
  the desktop behind the card while leaving the caller repainting turns out to
  need compositor cooperation, that is a compositor-side change this spec has
  not budgeted for.
