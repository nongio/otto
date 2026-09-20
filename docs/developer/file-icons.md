# File Icons

What goes in the icon box: the type icon every entry gets, the thumbnail that
replaces it, and the clock behind an animated one.

The decode worker and the thumbnail store are on the
[File Previews](file-previews.md) page; this one is about how a file becomes a
picture in a listing. Behaviour is specified in
[specs/file-browser.md](../../specs/file-browser.md) under *Shared
foundations*.

## Display follows the name, decoding follows the content

`otto_kit::filetype` answers two questions and keeps them apart.

- `mime_for_name(name)` decides **what is drawn**: the icon, the Kind column,
  portal filters, which application opens it. It does no I/O.
- `sniff(bytes)` decides **what is parsed**. Any decoder dispatches on this and
  never on the name.

Content must not override the name for display. If it did, an empty `.rs`
file's icon would flip the moment a sniff came back inconclusive — a bug you
can see.

`refine(sniffed, name)` is the one bridge, and it only narrows: the name may
pick a subtype within the hierarchy the content has already proved. An `.svg`
that sniffs as `application/xml` becomes `image/svg+xml`; a `liar.png` full of
XML stays XML. Container formats need this, and the observation behind it is
that Adwaita's SVGs sniff as XML while WhiteSur's sniff as SVG.

### The MIME database

There is no `xdg-mime` and no `mime.cache`. `filetype/db.rs` reads the
plain-text halves of shared-mime-info (`mime/globs2` and `mime/subclasses`)
from every XDG data directory, lowest priority first, so a user database wins.
It is loaded once per process, and a missing database is not an error: every
lookup returns `None` and the listing falls back to generic icons.

Rules are split into literals, extensions, a linear glob scan and the handful
of case-sensitive patterns, ranked by the spec's own order: literal first,
then weight, then pattern length. The glob matcher takes `*`, `?` and
`[...]`, backtracking iteratively so `*a*a*a*` stays linear.

`sniff` is a table of fixed signatures at offset zero, then RIFF and `ftyp`
brands at their own offsets, zip, `ustar` at 257, an `<svg` scan of the first
kilobyte, and finally UTF-8 with no NUL byte as `text/plain`, an explicit
last resort and never a guess dressed up as one.

## The chain

`Entry::icon_chain()` is most specific first:

| Entry | Chain |
|---|---|
| a directory | `folder`, `inode-directory` |
| `report.pdf` | `application-pdf`, `application-x-generic`, `x-office-document` |
| `shot.png` | `image-png`, `image-x-generic` |
| an unknown name | the kind's generic icon alone |

`filetype::icon_names` builds it: the MIME type with `/` turned into `-`, then
`<top>-x-generic`, then the `Kind`'s own fallback when it is not already there.
`Kind` is the lossy grouping (Folder, Image, Video, Audio, Text, Document,
Archive, Application, Other) and it also decides what may be thumbnailed.

A `.desktop` file draws `application-x-desktop`. The listing does not read its
`Icon=` key.

## Finding the theme

`otto_kit::icon_theme` settles on a name, in order:

1. the portal's `org.freedesktop.appearance` / `icon-theme`, Otto's own key,
   which neither the KDE nor the GNOME portal sets;
2. the portal's `org.gnome.desktop.interface` or `org.kde.kdeglobals.Icons`;
3. the desktop's own files, read synchronously at startup so the first frame
   has icons: `~/.config/kdeglobals` under KDE, otherwise `gtk-4.0` then
   `gtk-3.0` `settings.ini`;
4. the first of `breeze` or `Adwaita` that is installed.

A name is only accepted when `<root>/<name>/index.theme` exists under
`~/.icons`, `$XDG_DATA_HOME/icons` or an `$XDG_DATA_DIRS` icons directory. A
`SettingChanged` signal is watched, and a value arriving under a less specific
key never displaces a more specific one. Changing the theme clears the whole
icon cache.

With no theme at all, lookups fall to `hicolor`, which ships no `folder` and no
mimetype icons, so the listing would draw without any. That is what the file
and default fallbacks are for.

The compositor sets `icon_theme` in `otto_config.toml` and serves it on the
portal, but does not consume its own portal, so `src/utils` does that lookup
separately.

## Lookup, and the cache in front of it

`otto_kit::icons` wraps the `freedesktop-icons` crate. Entry icons go through
**`cached_icon_chain_at(names, size, lookup_size)`**, which has two properties
worth knowing:

- It uses `exact_icon_in_theme` per name, not `find_icon_in_theme`. The latter
  substitutes a generic icon on a miss, so the first name in a chain would
  always "succeed" and the rest would never be consulted.
- It never touches `AppContext`, so it is callable from a bare draw closure.
  That is what lets the compositor draw the same icons server-side.

The theme is searched at `FULL_COLOUR_SIZE`, 64, and the file that comes back
is rasterised at the size actually asked for. Themes commonly ship monochrome
outline art in their small fixed directories (Fluent's `16/places/folder.svg`
is a grey glyph) and the real icon only in `scalable`.

Results live in one process-global map, shared by every view and the file
picker. **Misses are cached too**, which is what keeps a folder full of
unknown types cheap.

SVG goes through **resvg/usvg**, not Skia: parsed with the requested size as
the default, rendered into a `tiny_skia` pixmap and wrapped as a Skia raster
image. (Peek's own SVG *preview* decoder is a different thing: it uses Skia's
SVG DOM with sealed resources.)

### Surviving a broken theme

A theme whose `index.theme` has a directory section with no `Size=` makes
`freedesktop-icons` panic, 37 of them in the first 15 seconds when it was
found. `guarded_lookup` catches the unwind, sets a process-wide flag, logs once,
and from then on resolves by hand: it reads `index.theme` itself, treats every
section as a directory, takes `Size=` as a hint and otherwise reads the size out
of the directory name, then scores candidates on
`(min(size, wanted), size == wanted, is_svg)` across `svg`, `png` and `xpm`.
Only a name the theme genuinely lacks falls through to the default theme. The
flag clears when the theme changes.

`components/otto-kit/tests/broken_icon_theme.rs` covers this end to end, in a
test binary of its own because the lookup crate reads the XDG directories once.

## Sizes

| Place | Box |
|---|---|
| Grid cell | `GRID_ICON`, 64 |
| List row, Miller row | `ICON_SIZE`, 18 |
| Sidebar place | 16 |
| Get Info | 64 |

Entry icons are rasterised at the logical size, with no output scale applied:
a grid icon is 64 px in a 64 pt box on a 2× output. Only `named_icon_sized`
reads the scale factor, and only to pick a theme directory. Thumbnail buckets
do take the scale, as an integer, so a fractional scale truncates.

## The thumbnail that replaces it

A thumbnail is drawn in the same box as the icon it stands in for, so the whole
pipeline is elsewhere; see [File Previews](file-previews.md). Two rules
belong here because they decide what you see:

- **Lookup for anything, generate only for images.** Every non-directory entry
  is looked up in the shared freedesktop cache, because other applications
  write there too, so a PDF or a video shows a picture when another file
  manager has already been through the folder. Otto generates only for
  `Kind::Image`.
- **Only a picture stands in for an icon.** A decoder that came back with a
  card, text or a listing yields nothing, and the type icon stays. Standing a
  card in would put a wall of identical grey tiles where the type icons say
  something useful.

All three views share `view::draw_thumbnail`: aspect-fit, never cropped, never
enlarged past its own pixels, bottom-aligned so mixed shapes share a baseline,
with a low-contrast hairline around it. A photograph with a white sky does
not end anywhere on its own.

## Animated previews

`Preview::Pixels` carries its frames stacked in one buffer with a delay each,
so a still picture is an animation of one frame and nothing that draws a
picture has to know the difference. `frame_image(i)` wraps modulo the frame
count and validates the buffer's length before handing anything to Skia. A
delay under `MIN_DELAY_MS` (20) is read as the author asking for the default,
100 ms, which is how every browser has read a zero since the format's first
decade.

The decoder only builds a strip when `animate` is set. Frames are decoded at
the source's own size into one reused buffer, since GIF frames are patches
composited over their predecessors, and each kept frame is then resampled to
the carried size. The budget is `MAX_ANIMATION_BYTES`, 192 MiB for the whole
strip, and `fit_budget` spends it in a fixed order: give up size first, in
fractions, down to half; only then thin the frames with a stride, and only
while the gap between kept frames stays under 100 ms. A dropped frame's
duration is added to the frame before it, so a thinned animation still runs for
the length the author wrote. Below 64 px it gives up and the still first frame
is used.

**Only the Peek panel runs the clock.** `Session::first_row` doubles as the
frame index: the scroll offset into a listing or a text preview, and the
frame of an animation, which the toolkit reads the same way. `tick_animation`
advances it when the current frame's delay has elapsed. It
loops for as long as the panel is open; stopping partway would leave the panel
on whatever frame the author happened to end on. `first_row` is part of
`peek_key`, so a new frame repaints exactly that subsurface, and
`peek_frames_running()` keeps the idle timeout short enough to be a clock.

The preview column passes frame 0 and gains only a caption:
`files-preview-animation`, the picture's size and the loop's length.
Thumbnails ask for `animate: false`: a tile shows one frame, and asking for an
animation would buy a strip of hundreds and keep the first of them.

## When something is missing

| | |
|---|---|
| No shared-mime-info | every name lookup misses; entries draw their kind's generic icon |
| Name not in the theme | the chain is tried in order; if all miss, `None` is cached and the icon box is left empty |
| Malformed `index.theme` | caught, and resolved by hand from then on (above) |
| Unsupported for generation | the shared-cache miss is final; the type icon stays |
| Decode failure or the 8 s deadline | `Unavailable`, which is not a picture, so the icon stays; Peek draws the file's own icon large instead, since a decoder that gave up is not a blank panel and the file's icon is still true |
| A corrupt or stale cache PNG | rejected on a missing or mismatched `Thumb::MTime`, or a failed decode |
| Another program's fail marker | never read. Only `fail/otto-files/` is consulted: another program's inability to read a format says nothing about ours |

## Testing

```sh
cargo test -p otto-kit   --lib filetype     # globs, sniffing, refine, the chain
cargo test -p otto-kit   --lib icons        # exact lookup, chain fall-through
cargo test -p otto-kit   --test broken_icon_theme
cargo test -p otto-files --lib thumbcache   # MD5, URI escaping, PNG text chunks
cargo test -p otto-files --lib thumbnails   # the store, the ceiling, eviction
cargo test -p otto-peek  --lib decode::image  # the three-frame GIF fixture
```

Tests that need a real icon theme or a real MIME database return early rather
than fail, so a bare CI container does not go red over a missing package. The
`#[ignore]`d `thumbcache` tests check Otto's keys against the machine's own
shared cache:

```sh
cargo test -p otto-files --lib -- --ignored --nocapture thumbcache
```

Strings are `files-kind-*` and `files-preview-*` in
`resources/locales/en-GB.ftl`.
