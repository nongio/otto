# Search Plan

A plan to take file search out of otto-files and share it: one search core
and one query language, used by the Files strip, the launcher, the command
line, D-Bus and the agents.

The core is a library, not a daemon. Every caller queries LocalSearch over
D-Bus itself, as otto-files does today. The only new D-Bus method,
`org.otto.Files1.ShowSearch`, asks a Files window to show a search; it does
not answer one.

Today's structure is described in [File Search](file-search.md) and the
behaviour in [specs/file-browser.md](../../specs/file-browser.md#recent-and-find).

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
| `taken:2024`, `camera:pixel`, `place:rome` | photo metadata |
| `artist:…`, `album:…` | audio metadata |
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

**Plain words search names only**, even with full text available. Contents
are explicit (`text:` or *Contents*), which keeps results predictable and
fast.

## Phase 2: using the queries directly

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
| Command line | `otto-files --search 'invoice kind:pdf modified:<30d' [--in DIR]` |
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

- A Settings pane: indexed folders, status, progress, pause and re-index,
  through the `org.freedesktop.Tracker3.Miner.Files` settings.
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

- Full text on by default, or opt-in?
- The core in `otto-kit`, or its own crate?
- A `search:` URI scheme, or only the flag and D-Bus?
