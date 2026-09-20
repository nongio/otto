# Text in Pictures

How Otto reads the words in a photograph, a screenshot or a scanned page, so
they can be selected, copied and found.

Behaviour is specified in [specs/peek-ocr.md](../../specs/peek-ocr.md). This
page describes the structure. The panel the words are drawn in, and the worker
that decodes the picture, are on the [File Previews](file-previews.md) page.

## Otto does no recognition

`components/otto-peek/src/ocr.rs` links no OCR library. The recogniser is an
external command, run inside the decode worker's containment — the same door
the PDF rasterisers already go through.

```
tesseract stdin stdout -l {languages} --psm 3 hocr
```

That is `ocr::DEFAULT_COMMAND`. The contract is narrow enough that anything
else can take its place: **PNG in on stdin, hOCR out on stdout**, stderr to
`/dev/null`. `{languages}` is substituted into the command line and also
exported as `OCR_LANGUAGES`.

The pixels are already decoded, in the same buffer the panel is about to
draw, so a picture is decoded once and recognised from that decode. The writer
and the reader each get a thread, because a PNG larger than a pipe buffer
would otherwise deadlock against a recogniser waiting to be read.

`ocr::available()` walks `PATH` for the command's first token, or checks an
absolute path is a file. It never execs anything to find out.

| Limit | Value |
|---|---|
| `MIN_CONFIDENCE` | 60; a recogniser that reports none is trusted at 100 |
| `DEADLINE` | 20 s, and the recogniser is **killed**, not merely abandoned |
| `MAX_HOCR` | 16 MiB |
| `MAX_PIXELS` | 16 000 000 |
| `MAX_WORDS` | `payload::MAX_WORDS`; over it, the most confident are kept and reading order restored |

The worker's own budget is raised for a recognition: `OCR_DEADLINE` 25 s
against the usual 8, and `OCR_CPU_SECONDS` 30 against 10.

### Reading hOCR

`parse_hocr` is a hand-rolled tag scan, not an HTML parser. It counts
`ocr_carea` as a block, `ocr_par` as a paragraph, `ocr_line` (and
`ocr_textfloat`, `ocr_header`, `ocr_caption`) as a line, and reads each
`ocrx_word`'s `title` attribute for its `bbox` and `x_wconf`. Inner tags are
stripped and entities decoded.

A recogniser that emits a bare list of words, with no structure at all, still
yields words.

### Languages

`ocr::languages()` is the locale's language plus `eng`, filtered to the packs
actually installed, joined with `+`: `ita+eng` on an Italian desktop. With
nothing installed it is `eng` alone.

`tesseract_code()` maps BCP-47 to tesseract's own codes for the two dozen
languages Otto is translated into and a few more; Chinese picks `chi_tra` for
TW and HK, `chi_sim` otherwise. Packs are looked for as
`<dir>/<code>.traineddata` under `TESSDATA_PREFIX`, `/usr/share/tessdata`,
`/usr/share/tesseract-ocr/5/tessdata`, the 4.00 path, and
`/usr/local/share/tessdata`.

`TESSDATA_PREFIX` is the one extra variable the otherwise cleared worker
environment carries through.

## Where the words live

A `Word` (`otto_kit::preview`) is its text, a rectangle in **decoded-picture
pixels**, a confidence 0–100, and its block, paragraph and line numbers. Words
ride on `Preview::Pixels`, so a picture that has them draws exactly as one that
has not until a selection is made.

`otto_kit::preview::selection` turns them into a selection:

- `word_at` hits a word with 2 px of slop; `word_near` falls to the last word
  on the line when the pointer is past the end of it.
- `selection_text` joins with a space between words, a newline between lines
  and a blank line between paragraphs and blocks.
- `selection_rects` is one union rectangle per line, padded and clipped to the
  layout's inner box, drawn at the accent colour with alpha 90.

Recognition happens on raster images and on rasterised PDF pages. A PDF page
is recognised from its pixels even when the document carries a text layer, so
one path serves both. SVG is never recognised.

## Selecting and copying

`peek::Session` holds the selection, whether a drag is in progress, a
`words_epoch` and `recognising_since`. `app/peek_session.rs` wires it to the
window: `Ctrl+C` copies (and falls through to the listing's own copy when
nothing is selected), `Ctrl+A` selects everything, `Escape` clears the
selection before it closes the panel. The pointer becomes a beam over a word.

A badge sits bottom-right of the panel, in the close button's idiom: it
breathes while a recogniser is reading, becomes a serif **T** once there are
words, and is absent when there are none.

## When recognition runs

Four ways in, all through `peek::recognise`:

1. **On preview** (`app/present.rs`). Once the picture is on screen, the cache
   is asked first, with the words scaled to this decode. On a miss, and with a
   recogniser installed, the panel starts breathing and a second worker runs at
   `Priority::Interactive`. Results carry the panel's generation and a stale one
   is dropped.
2. **A background pass over the folder on screen** (`app/ocr.rs`).
   `sync_recognition` hands back at most one job: the interactive queue first,
   otherwise the first visible picture in the active column that nothing has
   read yet, but only when no recogniser is running, no preview is pending and
   the thumbnailer is idle. It runs at `Priority::Background`.
3. **`otto-files --recognise <picture or folder>…`**, one level deep, printing
   `path: N words in X.Ys` and exiting non-zero if any failed.
4. **The command palette**, *Run text recognition*, offered only when the
   selection is a picture. It pushes onto the interactive queue, jumping the
   background pass.

`Priority` is a niceness: 0 for interactive, 10 for background, set through
`sandbox::Budget.nice`. Niceness is absolute, only ever raised, and inherited
across exec, so the recogniser runs at the worker's.

Get Info **reports and never triggers**: `text_status` says *Reading…*, a word
count, *No text found*, or that the picture has not been read, and nothing at
all for a non-picture or when no recogniser is installed.

## The cache

`components/otto-files/src/ocrcache.rs`, one TSV file per picture under
`$XDG_CACHE_HOME/otto/ocr/` (`OTTO_OCR_CACHE` moves it).

The file is named by `thumbcache::key_for`, the MD5 of the percent-encoded
`file://` URI, the same key the thumbnail cache uses, with `.p<N>` before the
extension for a PDF page past the first.

```
otto-ocr 1
uri	file:///home/me/receipt.jpg
mtime	1758300000
languages	ita+eng
recogniser	tesseract stdin stdout -l {languages} --psm 3 hocr
size	1600	1200

120	64	58	21	96	0	0	0	Totale
```

An entry is valid only while the recorded mtime matches the file's. An entry
written by another engine or another language still draws and still answers a
search, but `is_current` says no, so the background pass reads it again.

`store` writes a temporary file in the same directory at mode 0600 and renames
it into place, so the last writer wins and a half-written entry is never read.
The directory is created 0700. Nothing is remembered on a failure, so a
transient one is retried rather than cached as an empty answer.

`prune` drops entries older than `KEEP_FOR`, 90 days, and leaves other
processes' temporary files alone. It is fired once per run, on its own thread,
the first time the background pass finds nothing left to read.

## Finding words

`ocrcache::matches` scans every `.tsv` in the cache, matching the query as a
lowercase substring of the joined words and skipping entries whose picture has
changed. [File Search](file-search.md) merges those hits with the index's,
deduplicated by path, ranked as high as a name match can reach.

They answer even when the file indexer is off. The status line still says
*File indexing is off*, because names are still not being searched.

Reading every entry on every Find is the known cost. An in-memory index built
on the first Find is the next step; a database is not.

## Configuration

`~/.config/otto/files.toml`, read once per process:

```toml
[peek]
recognise_text = true    # off: nothing new is read; cached words still show and search
recogniser = ""          # empty: the tesseract line above
```

There is deliberately no row for this in Settings.

With no recogniser installed there is no on-preview recognition, no background
pass, no palette command, no *not read yet* status and no badge. Pictures
preview exactly as they otherwise would, and nothing on screen mentions a
missing dependency. Installing one is picked up at the next preview.

## Testing

```sh
cargo test -p otto-peek  --lib ocr          # hOCR parsing, language mapping
cargo test -p otto-kit   --lib selection    # hit-testing, text, rectangles
cargo test -p otto-files --lib ocrcache     # round trip, currency, pruning, scaling
cargo test -p otto-files --lib ocr          # status, queueing, the panel's own file
cargo run  -p otto-peek --example ocr_probe -- PICTURE [recogniser command]
```

No test runs a real recogniser, because CI installs none. The parser is
exercised against fixed hOCR, and the probe is the manual check.

Strings are `files-info-text-*` and `files-command-recognise-text` in
`resources/locales/en-GB.ftl`.
