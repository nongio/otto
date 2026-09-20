# File Search

How otto-files finds a file: the Find strip, the Recent listing, and the one
place both of them ask.

Behaviour is specified in
[specs/file-browser.md](../../specs/file-browser.md#recent-and-find). This page
describes the structure as it is built today. Text recognised in pictures is a
second source of matches and has its own page,
[Text in Pictures](peek-ocr.md).

## One source, and why

`components/otto-files/src/search.rs` is the whole provider. Every query goes
to LocalSearch (TinySPARQL) over D-Bus, and there is no second implementation
behind it.

That is deliberate. A search of our own that reads every directory under home
takes seconds where the index takes a fraction of one, and it answers a
*different* question: the index matches names by substring, a walk of our own
would match by subsequence. Which one ran would decide what you found. One
source is slower to be unavailable and never quietly disagrees with itself.

The cost is visible: `otfl` finds `otto-files.rs` on a machine with no indexer
and not on one with. Otto keeps the index.

Otto builds no index of its own and searches no file contents. Consulting an
index the desktop already keeps is a different thing, and is what this is.

## The wire

| | |
|---|---|
| Bus name | `org.freedesktop.LocalSearch3` |
| Object | `/org/freedesktop/Tracker3/Endpoint` |
| Interface | `org.freedesktop.Tracker3.Endpoint` |
| Method | `Query(sparql: s, fd: h, args: a{sv})` |

Rows come back down a **pipe**, not in the reply. `query` (`search.rs`) makes
an `O_CLOEXEC` pipe, hands the write end to the daemon and drains the read end
on a thread of its own, since a result set larger than the pipe buffer would
otherwise deadlock both ends. The search worker is a plain `std::thread` with
no handle to the application's runtime, so it builds a current-thread `tokio`
runtime inline to drive zbus.

The cursor's wire format is undocumented upstream, so `cursor_rows` parses it
by hand: a `u32` column count, one `u32` value type per column, one `u32` end
offset per column, then a blob of NUL-terminated strings. Only the first
column is read. A truncated stream yields the rows parsed so far rather than
an error.

`sparql_string` is the injection boundary: `"` and `\` are escaped and control
characters dropped.

## The query

`sparql_for` builds one statement for both modes:

```sparql
SELECT DISTINCT ?f WHERE {
  ?f a nfo:FileDataObject .
  ?f nfo:fileName ?n .
  ?f nfo:fileLastModified ?m .
  ?f nie:interpretedAs/nie:mimeType ?mt . FILTER(?mt != "inode/directory")
  FILTER( STRSTARTS(STR(?f), "file://<root>") || … )
  FILTER( CONTAINS(fn:lower-case(?n), "<query>") )
} ORDER BY DESC(?m) LIMIT 1000
```

The mime-type line is how folders are excluded when `files_only` is set.
Testing for the `nfo:Folder` type instead made the same query take seventeen
seconds rather than one.

`ORDER BY DESC(?m)` is the ranking for Recent. For a query it is only the
least arbitrary way to choose which thousand matches come back, since the
local rank decides the order that is shown.

**The index is asked for paths and nothing else.** Size, modification time,
kind and is-a-directory are read from the filesystem by `model::entry_for_path`,
because an index is always a little behind the disk. Statting each row makes a
result an ordinary `Entry`, which the grid, the thumbnailer and Peek handle
like any other. A row whose file has gone is dropped without a word.

## A request

`Request` carries the query, the roots to look under, whether folders are
wanted, and a debounce.

| Constructor | Query | Roots | Folders |
|---|---|---|---|
| `Request::recent()` | none | `recent_roots()` | no |
| `Request::folder(q, dir)` | `q` | `[dir]` | yes |
| `Request::everywhere(q)` | `q` | `[home]` | yes |

`recent_roots()` is the XDG user directories that exist, less Recent itself
and less `$HOME`, falling back to `$HOME` when there are none. Home's most
recently written files are caches, dot directories and build output.

The `debounce` is `Duration::ZERO` at every call site. A search runs when
Return is pressed, not on each keystroke, so there is no burst to wait out;
the field is kept as a seam for a caller that needs one.

## Ranking

Scoring is `otto_kit::matching::score`, shared with the launcher. It matches by
**subsequence**, not edit distance: +8 for a matched character, +14 at a word
boundary, +20 at the start, +12 for staying adjacent, −1 per skipped character
up to ten, and a length penalty of −len/6 at the end. A space in the query
resets the adjacency run rather than matching, so *fire dev* reaches *Firefox
Developer Edition*.

Recent has no query, so its rank is the modification time in epoch seconds.
A file whose time cannot be read sorts to the bottom rather than disappearing.

`Best` holds the winners: it collects to twice the limit, then sorts and
truncates, so a thousand results cost one sort rather than a thousand
insertions. `LIMIT` is 500, and `Best::entries` deduplicates by path, so a
picture found by both its name and its recognised words is one row.

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
capped. The machinery streams, but the LocalSearch path sends exactly one
batch, with `done` set, per request.

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
`$XDG_CACHE_HOME/otto/ocr/`, matching the query as a lowercase substring of the
words, and skips an entry whose picture has been modified since it was read.

`ask_pictures` filters those hits to the request's roots, honours `files_only`,
stats them into entries and ranks them at `i32::MAX`, as high as a name match
can reach. It runs on both paths, so **recognised words answer even with the
indexer off**. See [Text in Pictures](peek-ocr.md) for what fills the cache.

## Recent

Recent is the same search with no query, and a different sentinel path:
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

## Beyond the window

- The in-app **command palette** has *Search* and *Recent* in its Go group. A
  query given as the command's argument is a Return already pressed.
- **otto-launcher** does not search files. It shares only
  `otto_kit::matching::score` with otto-files.
- otto-files exports **no search interface** over D-Bus. Its only interface is
  `org.otto.FilePicker1`, which the portal brokers, and the picker has no Find
  strip.

## Testing

```sh
cargo test -p otto-files --lib search        # the provider and the strip
cargo test -p otto-kit  --lib matching       # the scorer
cargo test -p otto-files --lib live_index -- --ignored --nocapture
```

The unit tests never touch the bus: the unavailable-index test passes empty
roots so the query is refused before a connection is made. The last command is
the one that does, and needs a running indexer.

Strings live under `files-search-*` and `files-recent-*` in
`resources/locales/en-GB.ftl`.
