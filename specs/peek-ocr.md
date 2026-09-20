# Peek text recognition

**Status:** stable
**Related specs:** [peek.md](./peek.md) — the preview panel, the sandboxed decode worker and the payload this spec extends — [file-browser.md](./file-browser.md) — the thumbnail cache whose keying this reuses, and Find, which this feeds — [settings-app.md](./settings-app.md)

## Summary

Text in a picture becomes text you can select, copy and search for. When Peek
shows a photograph of a sign, a screenshot, or a scanned page, the words
in it can be dragged over and copied like the words of a text file, and Find
in the file browser matches them. Recognition runs in the same sandboxed
worker that decodes the image, using the system's `tesseract` if it is
installed, and remembers what it found so nothing is recognised twice.

## Goals

- In an image preview, the words on the picture can be selected by dragging
  and copied with the usual shortcut. The selection is drawn over the words,
  not in a separate text box, and it follows the picture through zoom and pan.
- Recognised text is found by the file browser's Find. Typing a word that
  appears in a screenshot lists that screenshot.
- Recognition happens once per file. Opening the same picture again shows its
  words immediately, and a file that changed is recognised afresh.
- The recogniser runs inside the decode worker's containment. A hostile image
  can waste one worker process and produce a preview without text; it cannot
  reach the network or write anything except the cache entry the worker was
  handed.
- Nothing in the feature is required. With no recogniser installed, previews,
  Find and the cache behave exactly as they did before this spec existed.
- The preview never waits for recognition. The picture appears first; the
  words become selectable when they are ready.

## Non-Goals

- Recognising text in video frames, or in any type other than raster images
  and rasterised PDF pages.
- Reading a PDF's own text layer instead of recognising the page it was
  rasterised from. The words would be exact and free, but they come out of a
  second document parser; the page already goes through the recogniser like
  any other picture, and until the text layer is read the recogniser's answer
  is the answer. Later work on the same word boxes.
- Recognising an SVG. A drawing is written, not photographed: its words are
  in the file, and rendering it only to read them back answers a question
  nobody asked.
- Copying the whole text of a picture with one action, selecting a line
  with a triple click, and detectors for addresses, phone numbers or links
  in the recognised text. All are later work on top of the same word boxes.
- A row in the Settings app. The switch lives in the browser's own config
  file until Settings has a Files pane.
- Writing recognised text into the image file, as XMP or anything else. Otto
  never modifies the user's files to remember something about them.
- Translating, spell-correcting or reflowing the recognised text. What was
  read is what is shown.
- A language picker. The recogniser's languages follow the system locale.
- Crawling the disk. Only pictures the user has looked at, or that sit in a
  folder the browser is showing, are ever recognised.
- Bundling a recogniser or its language data. Both come from the distribution.

## Behavior

### What is recognised

Recognition applies to files Peek previews as a picture: raster images
Skia decodes natively, and rasterised PDF pages. A PDF page is recognised
from its pixels like any other picture, whether or not the document carries a
text layer of its own — the rasteriser hands back pixels, and reading the text
layer instead is later work. SVG, video posters and audio cover art are not
recognised.

A picture is recognised at the resolution the worker decoded it, capped so
recognition costs a bounded amount of time. A picture the worker downscaled
for the preview is recognised at that downscaled size; the word boxes are
expressed in the coordinates of the decoded pixels, so they line up with what
is drawn regardless of the source's true size.

### The recogniser

The recogniser is an external command, exec'd from inside the decode
worker's containment exactly as a PDF rasteriser is. Otto links no OCR
library. The worker writes the decoded pixels to the recogniser's standard
input as a PNG and reads an **hOCR** document from its standard output: the
one format the OCR world shares, carrying every word's bounding box,
confidence and text in reading order, grouped into lines, paragraphs and
blocks. The default command is `tesseract` with its hOCR output; the
`[peek] recogniser` setting names another, with `{languages}` standing
for the language list, which the command also finds in `OCR_LANGUAGES`. A
recogniser that emits words without confidences is trusted; one that emits
no lines puts everything on one.

Languages are the system locale's language plus English, in that order; when
no language pack for the locale is installed, English alone; when English is
not installed either, whatever the recogniser defaults to. There is no
setting. A missing pack is not an error the user sees.

Recognition is bounded in time. A recogniser that has not answered within its
deadline is killed by the worker that started it — not merely abandoned when
the host gives up on the worker — and the picture is treated as having no
text. Its output is bounded too: an engine writing without end is cut off
rather than read to the end of the worker's memory.

### What the worker produces

An image payload may carry a list of **words**. Each word is its text, its
bounding box in decoded-pixel coordinates, its confidence, and which line and
paragraph it belongs to, so that dragging across words produces text in
reading order with line breaks where the recogniser saw them. Words below a
confidence floor are dropped before they leave the worker; a preview that
shows nonsense selection boxes over noise is worse than one that shows none.

The list is bounded. A picture producing more words than the bound keeps the
most confident ones; the wire format's size limits stay in force.

A payload with no words draws exactly as an image payload draws today.

### When recognition runs

**On preview.** Opening a picture in Peek decodes it and shows it as
soon as the decode lands. Recognition follows as a second decode of the same
file at the same size, with the recogniser run inside that worker, and the
words land as a second result on the same generation. Moving the selection
before the words arrive drops them with the picture, as any stale result is
dropped, and a recognition not yet started for a file the user has left is
never started. A cached result is applied in the first result, so a picture
seen before has its words from the first frame.

**In the background, for the folder on screen.** The file browser recognises
pictures in the folder it is showing when it is otherwise idle, the way it
makes thumbnails for them: visible rows first, one worker at a time, never
while a preview decode is in flight, and abandoned the moment the folder
changes. This is what lets Find answer for a picture nobody has opened yet. It
is bounded to the folders the person visits; there is no sweep of home.

**Asked for directly.** `otto-files --recognise <picture or folder>…` does
the pass's work from a command line and exits: it reads the pictures named,
or the pictures directly inside a folder named, and remembers them in the same
cache. It is how a folder is made searchable without opening a window on it
and waiting for the pass to get there, and how the recogniser is exercised
without a desktop session.

**Asked for by name.** The command palette offers *Run text recognition* when
the selection — or the entry under the cursor — includes a picture and a
recogniser is installed. It reads those pictures again whatever is already
remembered about them, which is the point: the remembered answer is what the
command is for when it is wrong, after a language pack is installed or a
picture is half-read. The pictures go into a queue the background pass drains
first, ahead of its own scan and without waiting for the window to be idle,
one at a time like everything else, and at the priority of work somebody is
waiting for. Both panels say *Reading…* while it runs. Aimed at something
with no picture in it, the palette refuses in place rather than appearing to
do nothing.

Background recognition runs only when nothing else is in flight: no
preview decode, no recognition for the panel, no thumbnails outstanding, and
one picture at a time. Each picture on screen is considered once per window.
There is no battery throttle yet, for this or for thumbnails.

**The panel comes first.** A recognition somebody is waiting for runs at the
browser's own priority; the pass over the folder runs niced, and so does the
recogniser it starts, since the value is inherited. The two gates work at
different moments and both are needed: refusing to start a background picture
while the panel is busy keeps them from overlapping in the first place, and
the niceness settles who gets the cores when they overlap anyway — a
background worker already inside the recogniser when a picture is opened is
not interrupted, because the work is nearly always seconds from done and
throwing it away would cost more than letting it finish behind the panel.

### The cache

Recognised words are remembered in Otto's own cache under
`$XDG_CACHE_HOME/otto/ocr/`, one entry per file, keyed exactly as the
freedesktop thumbnail cache keys its entries: the lowercase hex MD5 of the
file's absolute percent-encoded `file://` URI. An entry records the source
URI, the source's modification time, the recogniser command line and
languages it was produced with, the decoded size the boxes are expressed in,
and the words.

An entry is valid when its recorded modification time equals the file's
current one. A stale entry is replaced; it is never trusted. An entry whose
recogniser or languages differ from the current ones is still valid for
selection and for Find, but the background pass reads its picture again, so
installing a language pack or naming a better engine improves results over
time without invalidating everything at once. An entry written before the
engine was recorded counts as having come from a different one, and is
replaced once.

A picture in which nothing was recognised gets an entry with no words, so it
is not recognised again at every visit.

Entries are written to a temporary file in the same directory and renamed
into place, mode `0600`, directory `0700`. Two processes recognising the same
file concurrently is safe; last writer wins. Otto prunes entries whose picture
has not been modified in 90 days, once per run, the first time the background
pass finds nothing left to read — the moment nothing else wants the disk. A
temporary file another process is mid-rename is left alone. The cache is Otto's and may be deleted at any time; the only
cost is recognising again.

The worker cannot write — its file-size limit is zero — so the entry is
written by the host from the words the worker sent back, alongside the
modification time the host read when it opened the file.

### Selection in the panel

Words are hit regions over the picture. The pointer shows an I-beam over a
word and an arrow elsewhere. Pressing over a word selects it; dragging
selects every word from the press to the pointer in reading order — the
recogniser's line and paragraph order, not the geometric sweep of the drag —
and each selected word is drawn with a translucent highlight in the theme's
accent colour, merged into one shape per line. Pressing elsewhere clears the
selection and otherwise does what it did before: a drag on a scrollbar pans
a zoomed picture.

The copy shortcut copies the selected words as text, one space between
words, a line break between lines and a blank line between paragraphs.
Select-all selects every word. Escape clears a selection before it closes
the panel, so a stray drag does not cost the preview.

The selection is in decoded-pixel coordinates and is transformed with the
picture. Zooming in on a selected word keeps it selected and keeps the
highlight on it. Changing file or closing the panel drops the selection.

The panel carries one badge in its bottom-right corner, in the close button's
idiom, and it says which of three things is true. While the recogniser is
reading the picture it shows a working glyph that breathes, so a wait of
several seconds on a large screenshot reads as a wait rather than as an answer.
When words land it becomes a text glyph and stays: a picture with selectable
text can be told from one without before the pointer touches it. When the
recogniser finishes with nothing to attach the badge goes away, because there
is nothing on that picture to select and a badge that stayed would say
otherwise.

A picture whose words are already cached shows the text badge as it opens,
with no working state in between — there is nothing to wait for.

The selection is drawn only in the panel. A dock thumbnail or any other
consumer of the same draw function passes no selection and draws no
highlight.

### In the preview column

The preview column's caption carries the same answer as its last line, under
the kind, the size and the date: *Reading…*, the number of words, *No text*.
A picture always has the line once there is a recogniser to read it, so the
caption keeps its height as the answer changes and the picture above it does
not move.

### In Get Info

Get Info on a picture carries a **Text** row, under *Kind*, saying what has
become of its words: *Reading…* while a recogniser is on it, the number of
words once they are remembered, *No text* when it has been read and there was
nothing in it, and *Not read yet* when nothing has looked. The row is live —
the panel is a window of its own and is told to redraw when a recogniser
starts or finishes on the file it is showing — so a picture opened mid-pass
moves from one to the next in place.

Only pictures have the row. A picture no recogniser will ever read has no row
either: with nothing installed, *not read yet* is a promise that is not
coming.

The row reports; it does not ask. Opening Get Info does not start a
recognition, because the background pass is already working through the
folder on screen and a second trigger would only take the worker away from
it.

### Find

A Find query that matches recognised words lists the picture. The match is
case-insensitive and by substring, as the desktop's index matches names.
Results found this way are ordinary rows: statted from the disk, thumbnailed,
previewable, and dropped if the file is gone. They merge with the index's
results and are deduplicated by path.

The scope pills apply: *This folder* answers only from entries whose source
sits under the current folder, *Everywhere* from the whole cache. Since only
visited folders and previewed files have entries, Find can answer only for
those, and the browser does not pretend otherwise: there is no "indexing" state
for this, and no message. A picture that was never looked at is simply not
found.

Find answers from the cache even when the desktop's index is off. The status
line still says *File indexing is off*, because names are still not searched.

### Without a recogniser

When the recogniser is not installed, or exits without writing any words,
the worker reports none and nothing is recorded, so a later install is
picked up at the next preview. The preview shows the picture. Get Info and the panel
say nothing about it; the user guide says what to install.

Recognition is on by default. `[peek] recognise_text = false` in
`~/.config/otto/files.toml` turns it off: no background recognition and none
on preview. Cached words are still shown and still searched, since they cost
nothing. There is no row for it in the Settings app.

## Constraints & Edge Cases

- **The containment must allow the exec.** The worker already execs a PDF
  rasteriser inside its sandbox; the recogniser goes through the same door,
  with the same restrictions: no network, no file writes, stdin and stdout
  only. The worker's sandbox is resource limits and namespaces, not a
  syscall filter, so the exec and the recogniser's reads of its language
  packs are admitted as the rasteriser's are. Recognition is given a longer
  deadline and CPU budget than a plain decode.
- **Huge pictures.** Recognition is bounded by the decoded size, which the
  worker already caps. A 100-megapixel scan is recognised at preview size,
  which loses small print. That is the trade the deadline demands.
- **Rotated text.** The recogniser detects page orientation; text at 90° in a
  photograph comes back with boxes that are correct and a reading order that
  is not. The selection follows the recogniser's order. Fixing this is the
  recogniser's business.
- **Word boxes and zoom.** Boxes are transformed with the picture, not
  re-laid; a highlight scales with the word. The hit test runs in
  decoded-pixel space after inverting the picture's transform.
- **PDF pages.** Each page is recognised separately and cached under the
  same key with the page number, since the worker rasterises one page per
  decode. A page whose document has a text layer is recognised from its
  pixels like any other: the rasteriser's output is what the worker holds.
- **The cache and the thumbnail cache are separate stores** with one keying
  rule. The OCR cache holds text and boxes, not pixels, and other file
  managers do not read it, so it lives under Otto's own directory rather than
  the freedesktop one.
- **Find over the cache reads every entry.** The cache is bounded by what the
  person has looked at, and an entry is small, so a linear scan on a search
  thread is acceptable. If it stops being acceptable, an in-memory index built
  on first Find is the next step, not a database.
- **Localisation.** The setting label and the install hint are Fluent strings.
  Recognised text is not localised; the languages used are.

## Rationale

- **Recognise in the worker, draw in the host.** The worker already owns
  every operation on file bytes and already execs a rasteriser. Running the
  recogniser in the host would bypass the sandbox for the one step most
  likely to hit a parser bug, and would mean two places that spawn processes.
  Boxes are data; drawing them is the host's job like drawing the pixels is.
- **Exec, not link.** Linking `tesseract` and `leptonica` into every host that
  embeds Peek adds a large C++ dependency to the file browser, the
  picker and the desktop for a feature many installs will never use. The
  command is the same seam the PDF rasteriser uses, and it degrades the same
  way when absent.
- **hOCR, not the engine's own table.** Tesseract's TSV is only
  tesseract's. hOCR costs a few dozen lines of scanning and buys a
  recogniser that is a setting: anyone can put a different engine, or a
  script around one, in tesseract's place without touching Otto.
- **Otto's cache, not the file.** Writing XMP sidecars or embedded metadata
  would let other applications read the text, but it mutates files the user
  did not ask to have changed, breaks hashes and sync, and works only for
  formats that carry XMP. The cache costs one recognition per machine and
  touches nothing.
- **Folders you visit, not the disk.** [file-browser.md](./file-browser.md)
  refuses to build an index, and [peek.md](./peek.md) refuses a
  content index or full-text search. This spec bends both: an OCR cache is a
  content index of the pictures a person has looked at, and Find reads it.
  What is kept from those refusals is the boundary: nothing crawls home,
  nothing runs when no window is showing files, and the cache is keyed and
  bounded exactly as thumbnails are. The thumbnail cache is already
  per-folder-on-screen background work over the same files; recognition rides
  on it. Both related specs' non-goals point here for the exception.
- **On by default.** The feature has no visible surface until it works, and a
  setting nobody knows to turn on is a feature nobody has. The cost is CPU
  time in the background at thumbnail priority, which is already spent.
- **Locale plus English, no picker.** Most text people screenshot is in their
  own language or in English. A list setting would be correct for the few and
  a chore for everyone.
- **No words for low confidence.** The recogniser is confident about real
  text and unsure about texture. Dropping the unsure words removes almost all
  the false boxes and almost none of the real ones.

## Open Questions

- Whether Find over the cache should be gated behind a separate scope pill
  (*In pictures*) rather than merged into the two existing ones. Merged is
  specified; a pill is the fallback if merged results confuse.
