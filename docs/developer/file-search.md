# File Search

How Otto finds a file: the Files search strip, the Recent listing, the
`otto-search` command, and the one place all of them ask.

Behaviour is specified in
[specs/file-browser.md](../../specs/file-browser.md#recent-and-find), and the
query syntax in [specs/search-language.md](../../specs/search-language.md). This
page describes the structure as it is built today. Text recognised in pictures is a
second source of matches and has its own page,
[Text in Pictures](peek-ocr.md).

## Two layers

| Crate | Owns |
|---|---|
| `components/otto-search` | The query language, SPARQL, the wire to LocalSearch, name scoring, and `find`: paging past rows that are gone, statting, rechecking, ranking and capping. Also the `otto-search` command. No toolkit, no Wayland: the agents daemon and command-line tools can link it. |
| `components/otto-files/src/search.rs` | The worker thread and cancellation, turning results into `Entry`s, merging in words read from pictures. |

`otto_kit::matching` is a re-export of `otto_search::matching`, so the launcher
and the command palette score with the same code.

## One source, and why

Every query goes to LocalSearch (TinySPARQL) over D-Bus, and there is no second
implementation behind it. A search of our own that reads every directory under
home takes seconds where the index takes a fraction of one. One source is
slower to be unavailable and never quietly disagrees with itself.

Otto builds no index of its own. Consulting an index the desktop already keeps,
its full-text store included, is a different thing, and is what this is.

## From text to rows

```rust
let query = otto_search::parse("invoice kind:pdf modified:<30d");   // never fails
let plan = Plan::new(&query, &scope, Clock::now())?;                // None: nowhere to look
let best = otto_search::find(&plan, 20, &mut ())?.best();          // Err: index unavailable
```

`find` is the whole policy for turning the index's rows into an answer; a
caller that wants the raw rows can still call `index::search(&plan.sparql(n))`.

- **`query::parse`** splits on whitespace (quotes group), reads `key:value`
  filters and keeps each term's byte span. A token that does not parse stays a
  plain word with an `Issue` attached.
- **`Plan::new`** resolves what the text leaves open: `in:` paths against home
  and the starting folder, `modified:` against the clock and local midnight
  (`dates::local_offset`, via `localtime_r`).
- **`Plan::sparql`** renders the SPARQL and its bindings. **`Plan::recheck`**
  applies, from a stat, what the disk answers better than the index (time,
  size, and `-kind:folder`). **`Plan::admits`** checks a file found some other
  way (a picture's words) against everything but `text:`. **`Plan::rank`**
  scores a name.
- **`find(plan, limit, progress)`** pages the index (below), stats every row
  into a `Found` (path, name, folder or not, symlink, size, time, snippet;
  `Found::stat` is symlink-aware, and Files builds its `Entry`s from it),
  rechecks and ranks it, and returns the `Results`. `progress` is a `Progress`
  implementation: `alive()` is asked between rows, so a superseded search stops
  within one stat, and `page()` receives the results after every page but the
  last. `&mut ()` is the no-op.

## The wire

| | |
|---|---|
| Bus name | `org.freedesktop.LocalSearch3` |
| Object | `/org/freedesktop/Tracker3/Endpoint` |
| Interface | `org.freedesktop.Tracker3.Endpoint` |
| Method | `Query(sparql: s, fd: h, args: a{sv})` |

Every value that came from the person is passed in `args` and referenced as
`~name` in the query text. That is the whole injection boundary: there is no
escaping to get wrong. Values Otto computes (dates, byte counts, MIME types)
are written inline.

Rows come back down a **pipe**, not in the reply. `index::query` makes an
`O_CLOEXEC` pipe, hands the write end to the daemon and drains the read end on
a thread of its own, since a result set larger than the pipe buffer would
otherwise deadlock both ends. Callers are plain threads, so it builds a
current-thread `tokio` runtime inline to drive zbus.

The cursor's wire format is undocumented upstream, so `cursor_rows` parses it
by hand: a `u32` column count, one `u32` value type per column, one `u32` end
offset per column, then a blob of NUL-terminated strings. A truncated stream
yields the rows parsed so far rather than an error.

## The SPARQL

`otfl kind:pdf modified:<7d` becomes, roughly:

```sparql
SELECT DISTINCT ?f WHERE {
  ?f a nfo:FileDataObject ; nfo:fileName ?n ; nfo:fileLastModified ?m .
  OPTIONAL { ?f nie:interpretedAs ?c . ?c nie:mimeType ?mt }
  FILTER(STRSTARTS(STR(?f), ~root0))
  FILTER(REGEX(?n, ~word1, "i"))                       # "o.*t.*f.*l"
  FILTER(((BOUND(?mt) && ?mt = "application/pdf")))
  FILTER((?m >= "2026-09-19T10:00:00Z"^^xsd:dateTime))
} ORDER BY DESC(CONTAINS(fn:lower-case(?n), ~whole2)) DESC(?m) LIMIT 1000
```

What each choice is for, all measured against a home directory of 130,000
files:

- **Triple patterns first, filters after, `SELECT DISTINCT`.** The same query
  with a size pattern written after a filter took a minute; this order takes a
  tenth of a second.
- **`REGEX` for subsequence.** It costs the same as the `CONTAINS` it
  replaced, and closes the old gap where the index matched `otfl` by substring
  and missed `otto-files.rs`.
- **Order by "contains each word outright", then newest.** Subsequence
  matches are broad; ordering like this keeps the cap from cutting a real
  match in favour of a newer scattered one.
- **The MIME type is `OPTIONAL`.** LocalSearch records `nie:interpretedAs`
  only for files it has an extractor for. Archives, scripts and most source
  files have none, so a required type silently drops them. Tests on `?mt` and
  `?c` are guarded with `BOUND`, so a negated kind keeps them.
- **`kind:archive` is by extension**, since no extractor gives archives a type.
  `kind:document` and `kind:app` are membership of the index's `Documents` and
  `Software` graphs.
- **`-kind:folder` is not sent at all.** Folders always carry
  `inode/directory`, but joining the type onto every row doubled Recent's cost,
  and `NOT EXISTS` on a folder type took over twenty seconds. The stat that
  every row gets anyway answers it (`Plan::recheck`).
- **`text:`** is `?c fts:match ~text`, each term an FTS5 quoted string so
  typed operators are matched as words, with `fts:snippet` marking the hit
  between `U+0002` and `U+0003`. `index::Snippet` turns that into text and
  byte ranges. A negated `text:` is a `NOT EXISTS` with its own match; it is
  the slowest shape (a few seconds over all of home).

The live test runs every shape against the real daemon and fails any that
takes over five seconds.

## A request

`Request` carries the query text, the default roots, the folder a relative
`in:` is relative to, and a debounce.

| Constructor | Query | Roots | `cwd` |
|---|---|---|---|
| `Request::recent()` | `-kind:folder sort:modified` | `recent_roots()` | none |
| `Request::folder(q, dir)` | `q` | `[dir]` | `dir` |
| `Request::everywhere(q)` | `q` | `[home]` | none |

`recent_roots()` is the XDG user directories that exist, less Recent itself
and less `$HOME`, falling back to `$HOME` when there are none. Home's most
recently written files are caches, dot directories and build output.

The `debounce` is `Duration::ZERO` at every call site. A search runs when
Return is pressed, not on each keystroke, so there is no burst to wait out;
the field is kept as a seam for a caller that needs one.

## Ranking

Scoring is `otto_search::matching::score`, shared with the launcher. It matches
by **subsequence**, not edit distance: +8 for a matched character, +14 at a
word boundary, +20 at the start, +12 for staying adjacent, −1 per skipped
character up to ten, and a length penalty of −len/6 at the end.

`Plan::rank` scores each word of the query on its own and adds the scores, so
`tax invoice` finds `invoice-tax.pdf`; a phrase must appear as written, and an
excluded word must not appear.

What decides which results survive the cap depends on `sort:`: the name score
for relevance and for A to Z (the pane then sorts by name), the modification
time for `sort:modified` and for a query with no words (Recent), the size for
`sort:size`. A file whose time or size cannot be read sorts to the bottom
rather than disappearing.

`Results` holds the winners: it collects to twice the limit, then sorts and
truncates, so a thousand results cost one sort rather than a thousand
insertions. Files' `LIMIT` is 500; the command's default is 20.
`Results::best` deduplicates by path, so a picture found by both its name and
its recognised words is one row, and puts `sort:name` results A to Z. Files
pushes its pictures into the same `Results` before taking the best.

## Ghost rows

The index can remember files the disk no longer has. On the machine this was
written on, a deleted browser profile under `~/Desktop` left over 900 of the
1000 newest rows under Recent's roots pointing at nothing. Those rows are
dropped when the stat fails, and `find` goes back for more. The first page is
twice the limit, and at least `MIN_PAGE` (200) rows so a small limit still
ranks from enough candidates; each next page is four times larger (a round
trip costs the index's sort of every match, not the rows returned), up to
`MAX_PAGES` asks, stopping once the limit's worth of real files are in hand or
the index runs out. Files sends the results so far, not `done`, between pages.
A later page that fails ends the search with what the earlier ones found.

## Threading and cancellation

`Search` mirrors the contract `model::Directory` already has: `start` never
blocks and `poll` never blocks.

`start` makes a fresh channel, bumps a shared generation counter and spawns the
worker thread. The worker's `Sink` holds the counter and the generation it was
born with; `alive()` compares them, and a superseded request's batches are
never sent. Nothing is interrupted — a stale worker finishes and its answer
is dropped. Dropping the `Search` bumps the counter too, so closing a pane stops
its worker.

After a successful send the sink calls `AppContext::request_wakeup()`. An idle
window commits no frames, so without it the batch would sit in the channel
until the next key or click.

`poll` drains the queue and returns the **last** batch only: a batch replaces
the results, it never appends, because the worker has already ranked and
capped. A request sends one batch per page read, the last with `done` set.

## When the index cannot answer

Every failure path fails before the first send, so a failure can never land on
half an answer. The reason is logged, and the batch goes out with `available`
false and whatever the OCR cache found.

That flag reaches the window. The status line reads *File indexing is off*
(`files-search-unavailable`) rather than *Nothing found*, and the empty pane
says the same. Nothing found and nothing able to look must not look alike.

LocalSearch is an optional dependency (`optdepends` in the `PKGBUILD`), so a
fresh install may have no indexer at all.

Its systemd unit carries `ConditionEnvironment=XDG_SESSION_CLASS=user` and
silently declines to start without it. When Otto owns the session it exports
that variable with `systemctl --user set-environment` and
`dbus-update-activation-environment`, alongside `WAYLAND_DISPLAY`
(`src/state/mod.rs`, udev backend only). A missing assignment takes file search
down with it.

## Text in pictures

`ocrcache::matches` is the second source. It scans every `.tsv` in
`$XDG_CACHE_HOME/otto/ocr/`, matching a needle as a lowercase substring of the
words, and skips an entry whose picture has been modified since it was read.

`ask_pictures` asks it for every `text:` term, or for every plain word when
the query has no `text:`, and keeps the pictures that match them all. It stats
them into entries, checks them with `Plan::admits` (`admits_ignoring_names`
when the words stood in for the name) and ranks them at `i32::MAX`, as high as
a name match can reach. It runs on both paths, so **recognised words answer
even with the indexer off**. See [Text in Pictures](peek-ocr.md) for what
fills the cache.

## Recent

Recent is the query `-kind:folder sort:modified`, and a different sentinel path:
`/dev/null/otto-recent`, against search's `/dev/null/otto-search`. They must
differ because the sidebar lights the place whose path matches the pane's, and
one shared sentinel lit *Recent* as soon as anyone typed a query.

`recent.rs` buckets the results by day (Today, Yesterday, This Week, This
Month, Earlier) cut at local midnight through `localtime_r`. Entering Recent
forces the grid and a descending sort by modification time, and pins the sort
control off.

Both Recent and a result set are **synthetic panes**: `Column::synthetic` gives
them an idle loader, a dead `DirWatch` (inotify declines the sentinel) and
`epoch = 0`, so the view can tell "still filling" from "empty". Results are
deliberately unwatched. A file appearing three levels down must not reshuffle
the list while it is being read.

Selection in a synthetic pane is keyed by path rather than name, so three
`Cargo.toml`s in one result set select one at a time.

Anything that needs a folder behind it refuses on these panes with
`files-synthetic-no-action`, and results can be shown as a list or a grid but
never as Miller columns (`files-search-no-columns`).

## The strip

`app/searching.rs` is the controller and `view.rs` draws it.

- **Ctrl+F** opens the strip, records where you were and what it was called,
  and focuses the field. Pressed again while the field has focus it closes;
  pressed while the strip is up but blurred it takes the focus back and selects
  all.
- **Return** runs the query. Typing does not: the only thing a changed field
  does by itself is treat an emptied field as a cancel and navigate back to
  where the search began.
- **Up, Down, Page Up, Page Down, Tab** hand the keyboard to the listing with
  the strip still up. **Escape** clears.
- The two pills switch between *This folder* and *Everywhere*, re-running the
  query against the other haystack.

The field is deliberately leaky: it takes the editing keys and the keys that
belong to a search, and lets every other chord through to the window. Escape
unwinds in a fixed order: info panel, Peek selection, search, Peek, filter
menu, picker, selection.

A plain printable key in the listing is **type-ahead**, not search: a one-second
prefix match that moves the cursor and filters nothing (`app/cursor.rs`).

Running a search replaces the pane with the results, forces the grid if the
view was Miller columns, and records a history entry, but only for the first
query, so refining does not stack up entries to walk back through. Going back
to a search re-runs it rather than restoring stored rows.

The strip shifts everything below the header, and two dozen geometry helpers
take an area rather than a browser, so its height lives in a process-global
atomic (`view::set_search_band` / `search_band_h`) that the scene cache key
includes. The magnifier is drawn by hand, a circle and a stub, so it cannot go
missing on a sparse icon theme.

## The command

`otto-search [--in DIR]... [--limit N] [--json] QUERY...` is the crate's
binary (`src/main.rs`). The words are joined into one query; `--in` sets the
default roots (home otherwise), and a relative `in:` in the query is relative
to the current directory. It prints one path per line, or with `--json` one
object per line, `Found::to_json`: `path`, `name`, `kind` (`file` or
`folder`), `modified` (RFC 3339, local time), `size` (`null` for a folder) and
`snippet`. Parse issues go to stderr as `hint:` lines. Exit status: 0 found,
1 nothing found, 2 index unavailable, 3 usage error.

The command does not read Files' OCR cache, so words read from pictures answer
in Files but not here.

## Opening Files on a search

`launch.rs` reads the browser's arguments: `parse` takes the words, `resolve`
decides against a probe of the disk (both are pure, and tested that way).

- `otto-files --search QUERY [--in DIR]` opens on the results:
  `Browser::start_search` is Ctrl+F where the window opened (DIR, or home for
  *Everywhere*), the query typed and Return pressed, so Escape and Back lead
  back there.
- `otto-files --select PATH...`, or a file as a plain argument, opens the first
  path's folder with every given path in that folder selected.
- With both, the selection waits for its row in the results.

The selection is held in `Browser::pending_select` as selection keys and
settled each frame by `settle_select`: a folder settles when its read lands,
results as soon as every path has arrived or when the search ends with at
least one of them. Settling a selection in results hands the keyboard from
the query to the listing. Missing paths and unknown options are warnings on
stderr; the window then opens at home.

## Beyond the window

- The in-app **command palette** has *Search* and *Recent* in its Go group. A
  query given as the command's argument is a Return already pressed.
- **otto-launcher** does not search files yet. It shares only
  `otto_search::matching::score` with otto-files.
- otto-files exports **no search interface** over D-Bus. Its only interface is
  `org.otto.FilePicker1`, which the portal brokers, and the picker has no Find
  strip.

## Testing

```sh
cargo test -p otto-search                     # parser, SPARQL, wire, scorer, results, the command's arguments
cargo test -p otto-files --lib search         # the provider and the strip
cargo test -p otto-files --lib launch         # the browser's arguments
cargo test -p otto-search --test live_index -- --ignored --nocapture
cargo test -p otto-files --lib live_index -- --ignored --nocapture
```

The unit tests never touch the bus: the unavailable-index test passes empty
roots so the query is refused before a connection is made. The two `live_index`
commands do, and need a running indexer: the first runs every query shape
against it with a time budget, the second the whole worker.

Strings live under `files-search-*` and `files-recent-*` in
`resources/locales/en-GB.ftl`.
