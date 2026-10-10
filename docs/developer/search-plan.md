# Search Plan

A plan to take file search out of otto-files and share it: one search core
and one query language, used by the Files strip, the launcher, the command
line, D-Bus and the agents.

The core is a library, not a daemon. Every caller queries LocalSearch over
D-Bus itself, as otto-files does today. The only new D-Bus method,
`org.otto.Files1.ShowSearch`, asks a Files window to show a search; it does
not answer one.

Today's structure is described in [File Search](file-search.md), the
behaviour in [specs/file-browser.md](../../specs/file-browser.md#recent-and-find)
and the query syntax in [specs/search-language.md](../../specs/search-language.md).

## Status

Phase 1's core has landed on `feat/search`: the `otto-search` crate (parser,
plan, SPARQL with bound parameters, the wire), subsequence names through the
index, `text:` with snippets, and otto-files moved onto it. Still open in
Phase 1: `GraphUpdated`, and a fake endpoint for headless tests. Snippets are
returned but not yet shown in Files (Phase 1b).

Phase 1 also took in the parts of Phases 2 and 3 that let an agent search in
plain language and show the answer in Files, and they have landed:

- [x] One driver in the crate, `otto_search::find`: paging past ghost rows,
  statting, `recheck`, ranking, the cap. Files and the command both use it;
  Files keeps only the worker, `Entry`s and the pictures' words.
- [x] The `otto-search` command (`--in`, `--limit`, `--json`), packaged
  beside the other binaries.
- [x] `otto-files --search QUERY [--in DIR]`, `--select PATH...`, and a file
  as a plain argument opening its folder with it selected.
- [x] The `find.md` page of the `otto-help` skill: natural language to the
  query language, `otto-search --json`, then Files.

Not yet: words read from pictures in the command (the OCR cache is Files'),
the `search:` URI, `org.otto.Files1.ShowSearch` (each `--search` starts a new
window), and snippets in the command's plain output.

## Review notes

Checked against LocalSearch 3.11 on a home of 130,000 files before building:

- **Bound parameters work** through `Query`'s `args` (`~name`), for
  `STRSTARTS`, `CONTAINS`, `REGEX` and `fts:match` alike.
- **Subsequence by `REGEX`** costs the same as the old `CONTAINS`, so the
  `otfl` gap closes in the index, not after it.
- **Shape matters more than content.** A triple pattern written after a
  `FILTER` took 60 s where the same query with triples first under
  `SELECT DISTINCT` took 0.07 s. `NOT EXISTS` on a folder type took 23 s.
- **The index has no type for un-extracted files** (archives, scripts, most
  source): no `nie:interpretedAs` at all. `kind:` must treat the type as
  optional, and `kind:archive` goes by extension. Today's Recent required a
  type, so it silently left out every such file; it no longer does.
- **The index remembers deleted files.** Here, a deleted browser profile left
  900+ of Recent's 1000 newest rows pointing at nothing; the old query hid them
  only because those files had no type. Files now pages past ghost rows. Phase
  5 should surface this ("the index is out of date") and offer a re-index.
- **`place:` is weak:** 27 of 1,432 photos carry a place name; the indexer
  does not turn GPS coordinates into places. Left out of the language for now.
- **Core in its own crate**, not `otto-kit`: otto-agentsd does not link the
  toolkit (Skia, Wayland), and Phase 4 needs the core there.
  Name matching later moved to `otto-foundations` (#326), which otto-kit
  re-exports as `otto_kit::matching`.
- **Words are independent.** The plan's "name contains both words" disagreed
  with `matching::score`, which reads a space as "in order". Each word is now
  scored on its own.
- **Plain words and pictures.** [peek-ocr.md](../../specs/peek-ocr.md) promises
  that typing a word seen in a screenshot finds it, which "plain words search
  names only" would have broken. Kept: plain words also match words Otto has
  read in pictures; document contents still need `text:`.
- **Full text on by default** is not Otto's to decide: LocalSearch already
  extracts text. The real question was whether plain words search contents,
  answered above.

## What the index already holds

LocalSearch 3.11 indexes `$HOME` recursively with live monitoring. It skips
folders holding `.git`, `.hg`, `.nomedia` or `.trackerignore`; removable
drives and optical discs are off by default.

| Kind | Extracted |
|---|---|
| Every file | name, path, size, modified and accessed times, MIME type |
| Documents (PDF, MS Office, OpenDocument, EPUB, comics, PS, XPS, HTML, AbiWord, text) | title, author, page count, plain text up to a byte cap |
| Images (JPEG, PNG, TIFF, WebP, GIF, BMP, RAW, SVG) | EXIF/XMP: camera, lens, exposure, date taken, dimensions, GPS, keywords |
| Audio and video (libav, MP3) | artist, album, track, genre, duration, codec, resolution |
| Other | `.desktop` apps, games, ISO images, executables, playlists |

Query features Otto does not use yet:

- full text: `fts:match`, ranked by `fts:rank`, excerpts from `fts:snippet`
- content graphs (`tracker:Documents`, `Pictures`, `Audio`, `Video`,
  `Software`), which give kind filters almost for free
- `GraphUpdated` change notifications
- the writeback service, which writes edited metadata back into files
  (support level unchecked)

Otto asks only for file name, modification time and MIME type. Photo
libraries, kind and date filters and full-text search need no new indexing:
they are queries over data already there.

## Phase 1: one core, one language

**Shared core.** Move `components/otto-files/src/search.rs` into a shared
module (`otto-kit::search`, or a small `otto-search` crate) next to
`otto_kit::matching`. Files, the launcher, the CLI and the agents call the
same code.

**Grammar.** One parser turns text into a structured query; everything that
accepts a query accepts the same syntax. It is specified in
`specs/search-language.md` (to be written).

| Form | Meaning |
|---|---|
| `invoice tax` | name contains both words; subsequence ranking as today |
| `"tax return"` | exact phrase |
| `text:ricevuta` | file contents, full text, with a snippet |
| `kind:pdf`, `kind:image,video` | type; a comma is OR |
| `-kind:image` | exclude |
| `in:~/Documents` | scope, recursive |
| `modified:today`, `modified:<7d`, `modified:2025-03` | relative or absolute dates |
| `size:>100M` | size |
| `taken:2024`, `camera:pixel` | photo metadata (not yet) |
| `artist:…`, `album:…` | audio metadata (not yet) |
| `sort:size`, `sort:modified` | ordering |

- `kind:` maps to the content graphs: image to Pictures, audio, video,
  document, app to Software.
- Canonical keys are English. Localised aliases are accepted and chips show
  the localised label; URIs and D-Bus always carry the canonical key, so a
  saved query works in every locale.
- An unknown `foo:bar` stays as plain text with a hint. Nothing is dropped
  silently.
- The parser keeps each token's source span, so the strip's chips round-trip
  to text exactly.

**SPARQL.** The structured query becomes SPARQL with bound parameters
(`~name` through the `args: a{sv}` of `Query`) instead of `sparql_string`
escaping, which removes the injection boundary.

**Fuzzy names.** Fix the `otfl` gap: ask the index for names containing all
of the query's characters, then rank by subsequence with
`matching::score` as today.

**Text in pictures.** The OCR cache stays Otto's own, and the core merges
it, as `ask_pictures` does in Files today. Moving that into the core gives
the launcher and the agents the same answers, and `text:` covers both
document text from the index and words recognised in pictures.

It does not go into LocalSearch itself:

- An extractor module of our own is loaded through a private API with no
  installed headers, from folders fixed at build time (the only override is a
  pair of test variables that replace the whole folder). One module serves
  each MIME type, so taking over `image/png` means redoing its EXIF
  extraction, and OCR would then run over every picture on the indexer's
  schedule rather than on the ones Otto chose to read.
- The index belongs to its miner; rows we inserted would be dropped on
  re-extraction.
- Writing the words into the picture as XMP would change the user's files.

A later option, if agents query raw SPARQL a lot: keep the OCR words in a
TinySPARQL store of Otto's own, with the same ontology
(`nie:plainTextContent` on the same `file://` URIs), published as an
endpoint. One query can then join both through
`SERVICE <dbus:org.freedesktop.LocalSearch3>` and find documents or pictures
that mention a word. It costs a small endpoint process, so it waits until
something needs it.

**Live results.** Subscribe to `GraphUpdated` so Recent and open results can
refresh. Results still do not reshuffle while being read: updates land on
focus or behind a "new results" affordance.

**Tests.**

- Unit tests from text to structured query to SPARQL, with golden files.
- A fake `org.freedesktop.Tracker3.Endpoint` on D-Bus serving canned cursor
  rows, so headless tests run the full path without LocalSearch installed.

## Phase 1b: the language in the Files strip

**Chips.** A finished token such as `kind:pdf` becomes a chip on space.
Backspace at a chip's edge turns it back into text. Copying the query gives
text that works anywhere else.

**Autocomplete.**

- `ki` suggests `kind:`; after `kind:` the known kinds are offered.
- `in:` completes paths.
- `camera:` and `artist:` suggest distinct values read from the index.

**+ Filter.** A menu (Kind, Modified, Size, Contains text, Location) adds
chips without typing syntax. It is how people discover the language.

**Chips edit in place.** Clicking one opens a popover: a kind picker, date
presets, a size field.

**Scope.** *This folder* is `in:<cwd>`, *Everywhere* is no `in:`. The pills
stay as the quick switch and reflect a typed `in:`. A third pill, *Contents*,
is the same as a `text:` chip.

**Running.** Return still runs the search; chips parse live but do not query
per keystroke. Escape closes an open popover first, then the strip, putting
the original folder back.

**Results.**

- Content matches show a snippet line with the term highlighted, in list and
  grid.
- Name matches highlight the matched characters from `matching` spans.
- The status line says what ran ("12 PDFs modified this week in Documents"),
  and carries the indexing-off and still-indexing states.

**Saved searches.** Pin a query to the sidebar; the entry is a `search:`
URI, so smart folders come for free. Recent becomes a built-in saved search
(`sort:modified` over its roots), editable like any other.

**Plain words search names only**, even with full text available, except for
words Otto has read in pictures (see Review notes). Document contents are
explicit (`text:` or *Contents*), which keeps results predictable and fast.

## Phase 2: using the queries directly

The everyday half has landed early, as `otto-help`'s `find.md` page over the
`otto-search` command (see Status). What is left is a raw-SPARQL skill for
questions the language cannot ask.

**`otto-search` skill** in `resources/plugins/otto/skills/`, for agents
(Ask, Sessions) and power users:

- how to query: `tinysparql query` against `org.freedesktop.LocalSearch3`
  and `localsearch search` (verify the exact CLI invocation first; a
  count-per-graph probe returned nothing)
- a prefix and property catalogue: `nfo:`, `nie:`, `nmm:`, `nmm:Photo`,
  EXIF, where full text lives
- recipes: recent documents, photos by date or place, music by artist, full
  text with snippets, large files untouched for a year
- rules: read-only queries, always a `LIMIT`, filter by path prefix, stat a
  path before trusting it since the index lags the disk
- handing off: to show results, open Files on the search (Phase 3) rather
  than pasting paths

**Docs.**

- `docs/user/files.md`: the query syntax for people.
- `docs/developer/file-search.md`: the shared core and raw SPARQL recipes.
- A new docs page must be added to `website/build-docs.sh`.

## Phase 3: opening Files on a search

| Door | Form |
|---|---|
| Command line (done) | `otto-files --search 'invoice kind:pdf modified:<30d' [--in DIR] [--select PATH]` |
| URI | `otto-files 'search:///?q=invoice%20kind%3Apdf'`, for `xdg-open`, the launcher and agents |
| D-Bus | `org.otto.Files1.ShowSearch(query: s, options: a{sv})`, options `in`, `select_first`, `new_window` |

- Results open in the synthetic results pane (`/dev/null/otto-search`) with
  the strip filled in, so the query can be refined.
- `ShowSearch` goes to the Files window that last had the keyboard, or starts
  one.
- A "Search Files…" action in the desktop entry.
- Alongside: `org.freedesktop.FileManager1` (`ShowItems`, `ShowFolders`),
  which browsers and apps call for "Show in folder" and Otto lacks.

## Phase 4: consumers

- **Launcher.** A Files source through the shared core: top hits inline and
  a last row, "Show all in Files", calling `ShowSearch`. A prefix such as
  `kind:pdf` switches it to files only.
- **Agents.** A typed `search_files` tool in otto-agentsd taking the query
  language and returning paths with snippets, so common questions need no
  SPARQL.
- **Portal file chooser.** The same Find and Recent.
- **otto-album.** The Pictures graph as its library.

## Phase 5: index health

- A **Search** pane in Settings, specified in
  [specs/settings-app.md](../../specs/settings-app.md#search): status (live
  through `org.freedesktop.Tracker3.Miner` on D-Bus), folders searched, the
  code-repository and removable-drive rules, and Rebuild index. When
  LocalSearch is missing the pane says so and how to install it. No GNOME
  libraries: D-Bus through zbus, the indexer's settings through the
  `gsettings` command that ships with it.
- Surface extraction failures, and keep "still indexing" distinct from
  "nothing found".
- Warn when the index partition runs low on space; full text makes the
  index grow.

## Spec changes

`specs/file-browser.md`:

- the Non-goals line that rules out content search
- the Find section: chips alongside the pills, Escape order
- Recent as a saved search

## Order

1 and 1b (core, language, strip, tests), then 3 (open on a search), then 2
(skill and docs), then the launcher part of 4, then 5. Phases 2 and 3 need
only the grammar settled and can run alongside 1b.

## Open questions

- A `search:` URI scheme, or only the flag and D-Bus? `search` is a generic
  scheme name to claim desktop-wide; `otto-search:` would not collide.
