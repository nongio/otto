# File Picker

**Status:** v1 in progress — `OpenFile`, `SaveFile` and `SaveFiles` implemented end to end
**Wire contract:** `org.otto.FilePicker1`, defined inline below
**Related specs:** [file-browser.md](./file-browser.md),
[portal-access-dialog.md](./portal-access-dialog.md),
[settings-app.md](./settings-app.md), [context-menus.md](./context-menus.md)

## Summary

The dialog an application gets when it opens or saves a file. Otto serves it as
the `org.freedesktop.impl.portal.FileChooser` backend: `otto-portal` brokers the
request, and a D-Bus-activated picker presents it as a real client-decorated
toplevel window.

The picker and the file browser ([file-browser.md](./file-browser.md)) are two
shells over one view layer: the same directory model, the same async I/O,
watching, sorting and thumbnail machinery, and the same list/icon/column
presentations. Only the chrome around them differs.

### What is implemented

All three modes work end to end: an application's portal request opens a
picker window, and the path the user chooses comes back as a `file://` URI.

- `otto-files` is now a **library plus a binary**. `otto-files --picker` claims
  `org.otto.FilePicker1` and serves requests; `otto-files [path]` is the
  browser. One `Browser`, one view layer, one process image — the picker is the
  `Browser::picker` field being `Some`, and every difference between the two
  shells reads off it.
- Filters work, including MIME rules, which are expanded to globs through
  `otto_kit::filetype::globs_for`. Directories are never hidden by a filter.
- The action row carries the filter control, Cancel, and the request's own
  `accept_label`.
- The chrome is titleless: no traffic lights, no window title, one toolbar row.
- Quick View works in the picker exactly as it does in the browser.
- URIs are encoded by `otto_quickview::uri::path_to_uri`, which lives beside the
  decoder every consumer already uses, so the round trip is tested as a pair.
- **Save modes.** `SaveFile` shows the name field, pre-filled and with the stem
  preselected; `SaveFiles` asks only for a directory. Both put up the replace
  confirmation when something is already at the target, both refuse a name that
  is not a single path component, and both grey out accept — with the reason
  written under the field — when the directory cannot be written to. The name
  field holds the keyboard focus, so printable keys name the file rather than
  driving type-ahead; Up, Down and the page keys still move the listing, and
  clicking a file copies its name into the field. Ctrl+C, Ctrl+X and Ctrl+V
  work on the name text — the picker does no file management, so the chords are
  free for the field.

### What is not

- **"New Folder" in the picker.** The browser has it; the picker does not yet,
  so a save into a directory that does not exist means creating it elsewhere
  first. The picker's Non-Goals already say it should have it.
- **Drag-selection inside the name field.** A click places the caret and
  Ctrl+A selects the lot, but dragging across the text does not extend a
  selection the way it does in an in-place rename.
- **Concurrent requests open one window at a time.** A second application's
  request queues behind the first and is served when it finishes. Neither is
  dropped and neither hangs, but the second user waits. The app shell is built
  around a single toplevel; a second, half-supported window would be worse than
  queueing. This is the one deliberate departure from *Concurrent requests*
  below.
- **Choices** (`a(ssa(ss)s)`) are carried over the wire and not yet rendered.
- **Search and the location popup.** The toolbar's location control says where
  you are; it does not yet open the ancestor menu. Type-ahead is built: typing
  printable characters walks the cursor, as *Keyboard* below describes.
- **Per-`app_id` directory memory.** `Request::starting_directory` takes the
  remembered directory as an argument and is always passed `None`; nothing is
  persisted yet.
- **The header is still the browser's 92 px.** The layout is the reference's —
  one toolbar row, no title bar — but 19 pieces of geometry in `view.rs` are
  written against the `HEADER_H` constant, so making the picker's strip shorter
  is a separate, mechanical pass that threads the header height the way
  `Frame::footer` is threaded today.

## Goals

- An application calling the standard XDG desktop portal `OpenFile`, `SaveFile`
  or `SaveFiles` gets Otto's own picker, and receives back URIs that satisfy the
  request it made.
- Every option the portal request carries is honoured: file-type filters,
  multi-select, directory-only selection, a proposed name and folder, an accept
  label, and the app's own extra choice widgets.
- The picker is fully usable from the keyboard: reaching, filtering, selecting
  and accepting a file with no pointer at any point.
- Opening a directory of 10,000 files shows rows within one frame of the first
  batch landing, scrolls at full frame rate, and never blocks the UI thread on
  the filesystem — including on a stalled network mount.
- It reads as part of Otto: the same window chrome, material, typography and
  icon theme as the settings app and the browser.
- The picker holds no state between requests that the user would notice as
  wrong: a second request from a different app does not inherit the first app's
  directory, filter, or selection.

## Non-Goals

- File management. Copy, move, delete, trash, and drag-and-drop belong to the
  browser. The picker shares the browser's code and turns all of it off — a
  drag started in a picker window does not begin, and a drag over one is
  refused. The picker creates directories (apps need it while saving) and
  renames nothing.
- Being the browser's window. The picker is a transient serving someone else's
  app; the browser is a document window. They share a view layer and nothing
  else.
- `org.freedesktop.impl.portal.FileTransfer`. Out of scope entirely.
- Recursive or content search. The search field filters the current directory.
- Remote or virtual filesystems (GVfs, MTP, `sftp://`, `trash://` as a namespace
  the picker can return). The picker deals in local paths that a `file://` URI
  can name and the requesting app can `open(2)`.
- Running external `.thumbnailer` programs. See Thumbnails.
- Serving the GTK or Qt native file choosers. Applications that bypass the
  portal get their own toolkit's dialog, and that is their choice.

## Behavior

### Shape: who owns the window

`otto-portal` implements `org.freedesktop.impl.portal.FileChooser` and brokers
each request to `otto-file-picker` over a private, strongly typed interface —
the same broker/renderer split, and for the same reason, as
[portal-access-dialog.md](./portal-access-dialog.md): all "which renderer, is
one available, fall back how" policy stays in one place, and the portal binary
stays a headless zbus service with no Wayland connection.

The renderer is **not** `otto-islands`, and the picker does **not** follow the
island-panel pattern. It is its own component with its own toplevel:

- The picker is a window — resizable, remembering its geometry, with a sidebar,
  a scroll view and a titlebar. An island panel is a fixed-size overlay hung off
  the bar, and would have to grow a second, window-shaped rendering path to host
  this.
- It must be parented to the requesting application's window (see
  `parent_window` below), which requires an `xdg_toplevel`. An island is
  layer-shell and cannot be a child of a client's toplevel at all — so choosing
  a window keeps that door open, where choosing an island would close it
  permanently.
- Its lifetime is per request, not per session. `otto-islands` is a permanent
  daemon; the picker should not be resident when nobody is picking a file.
- It shares its entire view layer with the browser. Linking that model,
  thumbnail cache and worker pool into `otto-islands` would put a filesystem
  watcher and an image decode pool inside the process that draws the always-on
  status bar.

The picker is therefore **D-Bus activated**: it owns `org.otto.FilePicker1`, is
started by the bus on the first method call, and exits once it has nothing left
to serve. If the picker cannot be activated, the portal returns `response = 2` —
an application that asked for a file and got neither a file nor a dialog must be
told the request failed, not left waiting.

**As built, the renderer is `otto-files` itself rather than a separate
`otto-file-picker` binary.** When this spec was written there was no view layer
to share; there is one now, and it is `otto-files`. Splitting a third crate out
of it would have meant either duplicating the browser's chrome or extracting a
library whose only other consumer is the picker. The shape the spec actually
argues for — the portal brokers, something else renders, and the renderer is a
window rather than an island — is unchanged; only the binary's name is.

An idle picker holds no window, no Wayland connection and no watchers: the
process parks on the request queue before it ever calls into otto-kit, so
activation costs nothing until somebody actually asks for a file.

Activation needs `org.otto.FilePicker1.service` installed alongside the binary,
and the session's D-Bus activation environment must carry `WAYLAND_DISPLAY`.

### Wire contract: portal → picker

Bus name `org.otto.FilePicker1`, object path `/org/otto/FilePicker`, interface
`org.otto.FilePicker1`. Both ends are owned by Otto, so the signature is typed
rather than `a{sv}` — the same decision recorded for `org.otto.Dialog1`.

```
Present(request: (u s s s s b b b s s s as a(sa(us)) s a(ssa(ss)s)))
    → (response: u, uris: as, current_filter: s, choices: a(ss))

Close(handle: s) → ()
```

The request tuple, in order:

| Field | Type | Meaning |
| --- | --- | --- |
| `mode` | `u` | `0` open, `1` save, `2` save-multiple |
| `handle` | `s` | opaque request id, unique per in-flight request |
| `app_id` | `s` | requesting application, may be empty |
| `parent_window` | `s` | `wayland:<handle>`, `x11:<xid>`, or empty |
| `title` | `s` | window title; empty means a mode-appropriate default |
| `accept_label` | `s` | confirm button text; empty means a default |
| `multiple` | `b` | open mode only |
| `directory` | `b` | select directories rather than files |
| `modal` | `b` | carried for contract parity; not enforced in v1, see *Prerequisites* |
| `current_name` | `s` | proposed file name (save) |
| `current_folder` | `s` | absolute path to start in; empty means "decide" |
| `current_file` | `s` | absolute path of the file being re-saved |
| `files` | `as` | save-multiple: the names to be written |
| `filters` | `a(sa(us))` | `(label, [(kind, pattern)])`, `kind` `0` glob, `1` MIME |
| `current_filter` | `s` | label of the filter to preselect |
| `choices` | `a(ssa(ss)s)` | `(id, label, [(option_id, option_label)], default)` |

The response: `response` is `0` accepted, `1` cancelled by the user, `2` ended
for another reason (withdrawn, picker died, request invalid). `uris` are
percent-encoded absolute `file://` URIs, empty unless `response = 0`.
`current_filter` is the label of the filter in effect when the user accepted.
`choices` maps each choice group id to the selected option id (`"true"` /
`"false"` for a group with no options).

`Close(handle)` withdraws a pending request: the picker dismisses that window
and its pending `Present` resolves with `response = 2`.

These names, types and the field order are a permanent contract from the first
release, exactly as settings identifiers are.

### Translation at the portal

The portal's job is mechanical and must not lose information:

- `OpenFile` → `mode = 0`; `SaveFile` → `mode = 1`; `SaveFiles` → `mode = 2`.
- `current_folder` and `current_file` arrive from the portal as `ay`,
  NUL-terminated byte strings. The portal strips the trailing NUL and passes the
  path through; a non-absolute or non-UTF-8 path is dropped rather than
  rejected, and the picker falls back as if it were absent.
- `filters` and `choices` pass through unchanged in shape.
- `multiple`, `directory`, `modal`, `accept_label`, `current_name` pass through;
  absent options take the defaults in the table above.
- `Request.Close` on the portal's request object → `Close(handle)`.
- If the picker is unreachable or its call fails, the portal returns
  `response = 2` and logs why. It never returns `0` with no URIs.

The portal does not itself interpret filters, resolve paths, or check that the
returned files exist. That is the picker's contract with the user.

### Presentation

One window per request. Layout:

- **Titlebar** — the request's `title`, or "Open" / "Save As" / "Save Files".
  Client-decorated, per Otto's convention.
- **Toolbar** — back/forward, a parent-directory control, the view switcher
  (list / icon), a sort control, and a search field.
- **Path bar** — the current directory as clickable ancestor segments; clicking
  a segment navigates to it.
- **Sidebar** — the same places list as the browser: the XDG user directories
  that exist, the user's bookmarks, and currently mounted volumes. The picker's
  sidebar omits Trash, and omits any place the current request cannot select
  into.
- **File view** — the shared list / icon / column presentation.
- **Name field** — save modes only, pre-filled from `current_name` or the base
  name of `current_file`, with the extension unselected on focus.
- **Filter control** — a dropdown of the request's filters, plus "All Files"
  when the request supplies none. Hidden when the request supplies no filters
  and the mode is not save.
- **Choices row** — the app's extra `choices`, each rendered as a labelled
  dropdown or, for an empty option list, a checkbox. Same semantics as the
  Access dialog's choices.
- **Action row** — the accept button (using `accept_label`) and Cancel.

### Selection and acceptance

- **Open, single**: accept is enabled when exactly one entry is selected and it
  is selectable under the current mode. Double-clicking a directory descends
  into it; double-clicking a selectable file accepts immediately.
- **Open, multiple**: accept is enabled when at least one selectable entry is
  selected; every selected entry is returned, in view order.
- **Directory mode**: only directories are selectable. Files are still shown,
  greyed, so the user can see where they are. Accept with nothing selected
  accepts the directory currently being viewed.
- **Save**: the file goes into the directory being *viewed* — a folder merely
  selected in the listing is somewhere the user is looking, not somewhere they
  have gone. Accept is enabled when the name field is non-empty and does not
  contain `/`; `.` and `..` are refused for the same reason `/` is. If the resulting path exists as a file, a confirmation sheet
  appears — Replace / Cancel — and the request resolves only after the user
  answers. If it exists as a directory, accept instead navigates into it and
  clears the name field. If the target directory is not writable, accept is
  disabled and the reason is shown.
- **Save-multiple**: the user picks a directory — the selected one if exactly
  one is selected, otherwise the one being viewed; the picker returns one URI
  per entry in `files`, all inside that directory, with no name mangling. Each
  name is reduced to its final component first: "no name mangling" is about not
  inventing `file (1).txt`, not a licence for an application to reach out of the
  chosen directory with `../`. If any of
  them already exists, one confirmation sheet lists them all and offers Replace
  All / Cancel.
- What counts as "already there" is decided with `symlink_metadata`: a dangling
  symlink is still something in the way, and a symlink to a directory is a name
  being overwritten rather than a folder to descend into.
- A request in save-multiple mode carrying no `files` at all is malformed and is
  refused at the service with `response = 2`. There is nothing to write, and
  answering `0` with an empty list is the one thing the contract forbids.
- Accepting a symlink returns the symlink's own path, not its target. Following
  is the requesting application's decision.
- The picker does not create, truncate or open the file in save mode. It
  returns a URI; writing is the application's job.

### Starting directory

In order of preference: the directory of `current_file`; `current_folder`; the
directory this `app_id` last accepted from, if the picker still remembers it
(see below); the user's home directory. A path that does not exist, or is not a
directory, or cannot be read, falls through to the next candidate.

The picker remembers the last accepted directory **per `app_id`**, in its own
state file, and nothing else across requests. It never remembers a selection, a
filter, or a scroll position across requests. An empty `app_id` gets no
memory. This is the one deliberate exception to "no state between requests":
returning a user to where they were last time is the behaviour they expect, and
keying it on the app is what stops one app's directory leaking into another's
dialog.

### Filters

A filter is a list of `(kind, pattern)` rules; an entry passes the filter if it
matches any rule.

- **Glob rules** (`kind = 0`) match the entry's file name against the pattern.
  The supported syntax is the shell globbing subset that appears in practice:
  `*`, `?`, and `[...]` character classes. Matching is case-sensitive, except
  that a pattern which is entirely lowercase also matches an uppercase
  extension — `*.png` matches `PHOTO.PNG`, which is what applications mean.
- **MIME rules** (`kind = 1`) are resolved to globs through
  `otto_kit::filetype`, defined in [file-browser.md](./file-browser.md) under
  *Shared foundations*. The shared MIME database's `globs2` maps
  `weight:mimetype:glob`; a MIME rule expands to the globs registered for that
  type and for every type declaring it as a parent in `subclasses`.
  `inode/directory` matches directories. A MIME type with no registered glob
  matches nothing, and the filter reports that it is empty rather than silently
  matching everything.
- Filtering is by name. Content sniffing exists in `otto_kit::filetype` but is
  not used here: it would mean reading every file in the directory to decide
  what to display, and an application's filter is a statement about names.
- **Filters never hide directories.** A directory is always shown and always
  navigable, whatever the filter says, except that in directory mode the filter
  applies to directories and files alike.
- Filtering is applied to the loaded model, not to the read: changing the filter
  re-filters in place with no filesystem access.
- The filter in effect at acceptance is returned as `current_filter`, so an
  application can use it to decide an output format.

### Keyboard

The picker is operable with no pointer. Focus moves between sidebar, path bar,
search field, file view, name field, choices and buttons with Tab and
Shift+Tab, and the focused control is visibly ringed. This depends on the focus
and tab-order work that
[otto-kit-roadmap](../docs/developer/otto-kit-roadmap.md) records as the
toolkit's outstanding infrastructure gap; the picker must use it rather than
inventing a private one.

Within the file view:

| Key | Effect |
| --- | --- |
| Arrows | move the cursor; in icon view all four axes, in column view Left/Right change column |
| Home / End | first / last entry |
| Page Up / Page Down | one viewport |
| Enter | descend into a directory, or accept the selection |
| Backspace, Alt+Up | parent directory |
| Alt+Left, Alt+Right | back / forward in this window's history |
| Ctrl+H | toggle hidden entries |
| Ctrl+F | focus the search field |
| Ctrl+A | select all (multi-select requests only) |
| Ctrl+Shift+N | new folder |
| Escape | close a sheet if one is open; else clear the search field if non-empty; else cancel the request |
| `~`, `/` | open a location field pre-filled with that character, accepting an absolute or `~`-relative path |

Any key that moves the cursor **scrolls it into view**: the pane scrolls the
shortest distance that brings the cursor's whole row or cell inside the
viewport, and does not move at all when it is already there. Walking a long
directory with the arrow keys therefore never leaves the selection off screen.

**Type-ahead** is distinct from search and must not be conflated with it.
Typing an unmodified printable character while the file view has focus appends
to a type-ahead buffer and moves the cursor to the first entry whose name starts
with that buffer, case-insensitively, without changing what is displayed. A key
held with Ctrl, Alt or Super is a chord, never a letter of a name: it reaches
the shortcut that binds it — Ctrl+I opens the info panel — and leaves the buffer
alone, whether or not this app binds that chord. Modifier state is read from
what the compositor reports, so a modifier held before the window took focus
counts. The buffer
resets after one second of no typing, or on any navigation key. Repeatedly
pressing the same single character with no other input cycles through the
entries beginning with it. Search, reached with Ctrl+F, filters the view instead
and leaves the cursor where it can see the result.

**Multi-select** applies only when the request set `multiple`. An anchor tracks
the last entry selected without a modifier. Shift+click and Shift+Arrow select
the contiguous range from the anchor to the cursor, replacing the previous
range. Ctrl+click toggles one entry and moves the anchor to it. Ctrl+Arrow moves
the cursor without changing the selection; Ctrl+Space toggles the entry at the
cursor. A pointer drag beginning on empty space rubber-band selects, in icon view. In a
single-select request every one of these collapses to "select the entry under
the cursor".

### The view model

One model, three presentations, and the model is shaped for all three from the
start so that the third is not a rewrite.

The unit is a **directory view**: one directory's entries in display order,
plus the sort key and direction, the filter and search predicates in force, a
selection set, an anchor and a cursor. A window holds a **path stack** — a
vector of directory views from some root to the directory currently being
viewed — plus a back/forward history of paths.

- **List view** renders the last element of the stack as rows with columns:
  name, size, kind, date modified. Column headers sort; clicking the active
  header reverses. Columns are resizable; widths are per window, not persisted
  in v1.
- **Icon view** renders the last element as a grid of icon-over-name cells,
  with a size control. This is where thumbnails matter.
- **Column view** renders the whole stack as adjacent panes, each a narrow list
  of the corresponding directory, with the selection in pane *n* determining
  the contents of pane *n+1*, and horizontal scrolling that keeps the active
  pane visible.

**List and icon views ship in v1. Column view is deliberately deferred.** The
path stack exists from the first commit — every navigation pushes and pops it,
and list and icon views simply render its top — so column view is a third
renderer over an unchanged model rather than a change to how navigation works.

The view is **virtualised**: laying out and drawing a directory view must cost
time proportional to the number of visible entries, not to the number of
entries. This is a requirement of the model's interface, not an optimisation to
add later — the difference is invisible at 200 files and fatal at 10,000.

Every presentation is an otto-kit component in the established shape: a draw
function taking a canvas, a rect, a slice of entries and a theme, plus a
hit-test helper reading the same geometry, with state held by the caller. None
of them may touch `AppContext` or `wayland-client`, so the compositor can draw
a file list server-side if it ever needs to.

### Async I/O

The UI thread never performs a filesystem call. Not `readdir`, not `stat`, not
`open`, not `readlink`. A stalled NFS or SSHFS mount must cost a spinner, never
a frame.

- A **model thread** owns every loaded directory view and is the only thread
  that touches the filesystem, delegating to a small pool of **worker threads**
  (`min(4, available_parallelism)`) for reads, stats and thumbnail decoding.
- The UI thread holds an immutable snapshot of the directory view it is
  drawing. When the model produces a new snapshot it sends it over a channel and
  wakes the event loop; the UI thread swaps it in and requests a frame. It does
  not paint directly — otto-kit paints on frame callbacks.
- Reading a directory is one job that streams results in batches (2,000 entries
  or 50 ms, whichever comes first). The first batch is displayed as soon as it
  arrives; the view shows a progress indication until the read completes.
- The initial listing uses only what `readdir` returns: the name and, where the
  filesystem supplies it, the entry type. **No `stat` on the fast path.** Size,
  modification time and resolved link target arrive from a second streaming
  pass, and the columns that need them render as placeholders until they do.
  Sorting by a not-yet-loaded key waits for that pass and says so.
- Navigating away cancels the in-flight read of the directory being left. Every
  job carries a cancellation flag the worker checks between batches.
- Threads, not an async runtime. There is no async filesystem interface on Linux
  worth its complexity here, and `tokio`'s file API is a thread pool wearing a
  costume. tokio stays where zbus already needs it.

### Thumbnails

The cache itself — location, key, validity, atomicity, who may write to it — is
defined in [file-browser.md](./file-browser.md) under *Shared foundations*, and
is shared with quick view. What follows is how the picker uses it.

- **Who generates them:** the picker/browser process itself, on the worker pool,
  never the compositor and never a separate daemon.
- **What gets thumbnailed in v1:** images Skia decodes natively (PNG, JPEG,
  WebP, GIF, BMP, ICO) and SVG, which `usvg`/`resvg` already render. Nothing
  else is decoded.
- **What is not:** external `/usr/share/thumbnailers/*.thumbnailer` programs are
  not executed. They are arbitrary binaries run against untrusted files, and
  sandboxing them properly is a project of its own. The picker still *consumes*
  thumbnails other applications have already written to the shared cache, so a
  video another file manager has thumbnailed shows its thumbnail here. Running
  `.thumbnailer` entries is a later change, behind a setting.
- **Where they are cached:** the freedesktop shared thumbnail cache,
  `$XDG_CACHE_HOME/thumbnails/{normal,large,x-large,xx-large}/<md5 of the file's
  URI>.png`, with the `Thumb::URI` and `Thumb::MTime` PNG text chunks written
  and honoured. A cached thumbnail whose `Thumb::MTime` differs from the
  source's modification time is stale and regenerated. Failures are recorded in
  `thumbnails/fail/otto-files/` so an undecodable file is not retried on every
  scroll.
- **In memory:** decoded images live in an LRU cache bounded by both entry count
  and bytes (512 entries / 64 MB as the starting figures). 10,000 files must
  never mean 10,000 live images.
- **What is requested:** thumbnails for the visible range plus one viewport of
  margin above and below, and nothing else. The request queue is a
  visibility-keyed priority stack, not a FIFO: scrolling replaces the pending
  set rather than appending to it, so jumping to the end of a 10,000-entry
  directory queues one screenful of work, not ten thousand jobs.
- **Before the thumbnail:** the entry's type icon, resolved through the icon
  theme, drawn immediately. A thumbnail replacing an icon is a swap, never a
  reflow.
- **Size ceiling:** files above a threshold (128 MB as a starting figure) are
  not thumbnailed, and neither are files on a mount flagged as remote. The type
  icon stands in.

### Filesystem watching

Only the directories currently displayed are watched — the path stack — using
inotify directly. One inotify instance serves the process, with a single reader
thread blocked in `poll(2)` until an event lands or a debounce comes due; the UI
thread never blocks on it. A watch belongs to the column showing that directory
and is dropped with it, so the watch set stays equal to what is on screen. Two
panes on the same directory share one watch. Nothing is watched recursively.

`IN_CREATE`, `IN_DELETE`, `IN_MOVED_FROM`, `IN_MOVED_TO`, `IN_CLOSE_WRITE` and
`IN_ATTRIB` mark the directory dirty. A dirty directory is re-read after a
100 ms debounce, and the re-read happens **in place**: only the column's listing
is replaced. The selection is held by name and survives untouched; the scroll
view — offset, measurements and momentum — is never rebuilt, because a change
somebody else made must not move the view out from under the person reading it.
The cursor is an index, so it alone is re-derived from the selection, and
nothing is scrolled to reveal it. `IN_Q_OVERFLOW` forces a re-read of everything
watched. `IN_DELETE_SELF`, `IN_MOVE_SELF` or `IN_IGNORED` on a displayed
directory drops the panes at and below it and lands on the nearest surviving
ancestor, saying why in the status line.

Operations the user performs themselves — a paste, a delete, a drop — re-read
every displayed column immediately by the same in-place path, rather than
waiting out a debounce: the watcher would get there on its own, but the view
should not visibly lag behind the user's own hand.

Watching the places file, and watching `/proc/self/mounts` for removable volumes
appearing and disappearing in the sidebar, are not yet built. Volumes that are
not already mounted are not shown and cannot be mounted from the picker in v1.

A filesystem that inotify cannot watch (many network mounts) simply produces no
events; the view is correct when read and refreshes on navigation or on an
explicit reload. It must not poll.

### Sorting

Sort keys: name, size, kind, date modified. Directories always sort before
files, in both directions, and this is not configurable in v1.

Name ordering is natural and case-insensitive: runs of digits compare
numerically, so `file2` precedes `file10`. Comparison is done on Unicode
scalars with ASCII case folding, without locale collation — a known limitation
for languages where that is wrong, accepted rather than pulling in a collation
crate.

Each view has a default sort. List view leads with date modified, newest first
— it is the view that shows the modified column, and the question it is usually
opened to answer is "what changed last". Icon and column views sort by name,
ascending. Switching views takes the new view's default until the user picks a
sort of their own by clicking a column header; from then on their choice
follows them between views.

The picker does not persist sort order between requests. The browser does, per
directory.

## Constraints & Edge Cases

- **`parent_window` does not work yet, and v1 does not depend on it.** Otto
  ignores `xdg_toplevel.set_parent` for toplevels: window order comes from
  workspace stacking alone, and nothing reads a toplevel's parent. The request
  is not an error — Smithay records it — so the picker may send it, and it
  becomes correct for free once the compositor honours it. But **v1 must be
  designed as if it were absent**, which means:
  - The picker does not stay above the application that opened it and can be
    buried by it. Nothing prevents the parent being focused or raised while the
    picker is up.
  - **`modal` is accepted and not enforced.** The picker is a plain toplevel.
    Nothing in this spec may be written as "the user cannot do X while the
    dialog is open" — the picker must be correct when the user goes away, uses
    the app, and comes back to it.
  - Placement uses `parent_window` only as a hint for *which output* to open on,
    resolved through xdg-foreign where the handle imports; otherwise the output
    holding the pointer. Position within that output is centred.
  - The window is titled and attributed clearly enough to be found again after
    being buried, and the taskbar/switcher entry names both the picker and the
    requesting `app_id`.
  See *Prerequisites* below.
- **The requesting app dies mid-dialog.** The picker keeps the window up — the
  portal frontend will `Close` the request when it notices, and until then the
  user is entitled to finish what they were doing without the window vanishing.
  On `Close` the window disappears immediately.
- **The picker dies mid-request.** The portal must notice the peer disappear and
  resolve the pending call with `response = 2`. A file dialog that never returns
  hangs the requesting application's UI thread.
- **Concurrent requests.** Multiple windows in one process. They share the
  thumbnail cache and the worker pool; they share no selection, directory or
  filter. A per-request window must not be able to starve another's I/O — jobs
  are round-robined between windows, not served in arrival order.
- **Unreadable directories.** A directory the user cannot read shows an
  explanation in place of the listing, not an empty list. An entry that cannot
  be stat'd shows the name with unknown metadata rather than being dropped.
- **Non-UTF-8 names.** Linux file names are bytes. An entry whose name is not
  valid UTF-8 is displayed with the invalid sequences replaced, but the picker
  keeps the original bytes and returns a URI built from them. Round-tripping a
  file must never depend on it being nameable in Unicode.
- **Symlink loops** must not hang navigation. A directory is read as itself; the
  picker follows no link it was not asked to.
- **The URI encoding is not optional.** Every byte outside the unreserved set is
  percent-encoded. A file called `a b&c#d` must survive the round trip; this is
  the single most likely place for a silent data bug.
- **Save into a directory that disappears** while the dialog is open: accept
  fails with an explanation and navigation falls back to the nearest surviving
  ancestor, rather than returning a URI in a directory that no longer exists.
- **A filter that matches nothing** in the current directory shows a "no
  matching files" state naming the filter, with the filter control still
  reachable. It must be obvious that files exist but are filtered out.
- **Directory mode with `multiple`** is legal in the portal contract and returns
  several directories.
- **Zero visible entries after hidden-file filtering** in a directory that
  contains only dotfiles must be distinguishable from an empty directory.
- **The picker must not require the compositor to be built with development
  features**, and must run under the windowed development backend.

## Prerequisites

Two pieces of work outside this spec that the picker depends on. Both are named
here with what they block, so neither is discovered late as an assumption.

**1. Toplevel parenting in the compositor — not started.** Otto does not honour
`xdg_toplevel.set_parent`: nothing in `src/shell/` or `src/workspaces/` reads a
toplevel's parent, and stacking comes from workspace order alone. Until it does,
a dialog cannot be kept above the window that opened it and there is no modal
grouping of any kind. The portal file chooser is the archetypal parented dialog
and is likely the first thing in Otto to want this.

This is compositor work, and it is a prerequisite for *modality*, not for the
picker: **v1 ships without it** and is specified above to not need it. When it
lands, the picker's `modal` handling becomes enforcement rather than a no-op,
and no other part of this spec changes. Owner: the compositor, not this
component.

**2. Focus and tab order in otto-kit — not started.** Already the toolkit
roadmap's named outstanding gap. Unlike parenting, this one genuinely blocks:
the keyboard goal above cannot be met without it, and the picker must not invent
a private focus model that later has to be unpicked. `button` and `toggle` also
need their own hover and press state before they can sit in a toolbar.

## Rationale

**The portal brokers; a separate component renders.** This is the shape
[portal-access-dialog.md](./portal-access-dialog.md) already established, and it
holds for the same reasons: the portal binary stays a headless D-Bus service,
policy about renderers lives in one place, and the renderer can be replaced. The
one thing that changes is *which* renderer — an island cannot be a window, and
this dialog has to be one.

**The picker is D-Bus activated rather than resident or spawned per request.**
Resident costs memory and a filesystem watcher for a dialog most users open a
few times a day. Spawned per request means a cold process, a fresh Wayland
connection and an empty thumbnail cache every time, and forces the portal to
manage a child's lifetime. Activation gives the portal a plain method call, the
bus the process management, and a run of picks in one session the benefit of a
warm cache.

**One view layer, two shells.** The picker and the browser differ in chrome and
in what the accept action means. Everything below that — reading directories
without blocking, watching them, sorting them, thumbnailing them, drawing them
three ways, navigating them from the keyboard — is one body of work, and it is
the hard half. Writing it twice would guarantee the two dialogs diverge in
exactly the details users notice.

**MIME filters collapse into glob filters.** Resolving a MIME type through
`globs2` and `subclasses` turns rule kind 1 into rule kind 0, leaving one
matcher to write and test instead of two mechanisms with different bugs. The
cost is that a file whose name lies about its type is filtered by its name,
which is what every other implementation does in practice anyway.

**The shared thumbnail cache, at the cost of writing MD5 and a PNG chunk
splicer by hand.** Two small pieces of well-specified code — RFC 1321 with
published test vectors, and PNG `tEXt` chunk framing with a CRC32 — buy
interoperability with every other thumbnailing application on the system: we
read what they wrote, and they read what we wrote. Both are the kind of thing
this project prefers to write rather than depend on. Skia already encodes and
decodes the PNG itself.

**Threads and channels, not an async runtime, for the filesystem.** The
requirement is "never block the UI thread", and a worker thread satisfies it
exactly. Adding an async filesystem abstraction over what is ultimately blocking
syscalls buys nothing and costs a runtime in a process that would otherwise not
need one.

**Type-ahead is not search.** Conflating them — typing filters the list —
destroys the user's spatial memory of the directory and makes arrow navigation
mean something different depending on what was typed. They are separate
mechanisms with separate keys, and this is worth the extra state.

**No `stat` on the initial listing.** 10,000 files is 10,000 syscalls before
anything is drawn, and on a network mount that is seconds. Names first, metadata
second, is the difference between a dialog that opens and one that hangs.

**The picker remembers a directory per app, and nothing else.** Users expect to
return to where they last saved. They do not expect the image editor's dialog to
open in the directory their mail client last attached from, and they do not
expect a stale selection or filter from an unrelated request. Keying on `app_id`
and forgetting everything else is the smallest thing that gets the expected
behaviour without the surprising ones.

## New otto-kit components

Reused as they stand: `source_list` (places sidebar), `scroll`, `text_input`
(search and name fields), `dropdown` (filters, sort, choices), `context_menu`,
`list`, `titlebar`, `window`, `button`, `toggle`, and the `icons` lookups that
do not touch `AppContext` — `cached_file_icon` and `find_icon_in_theme`, never
`named_icon_sized`.

New, and generic enough to belong in the toolkit:

| Component | Draw + hit-test | Notes |
| --- | --- | --- |
| `search_field` | `draw`, `clear_at` | Already recorded as an open toolkit item |
| `segmented_control` | `draw`, `segment_at` | View switcher; already an open item |
| `breadcrumb` | `draw`, `segment_at`, `overflow_at` | Path bar, with ancestor collapsing |
| `table` | `draw`, `row_at`, `header_at`, `divider_at` | Sortable, resizable columns; virtualised over a visible range |
| `icon_grid` | `draw`, `cell_at`, `cells_in_rect` | Cell grid with rubber-band support |

Prerequisite, and the real blocker: **focus and tab order** in otto-kit. The
picker's keyboard goal cannot be met without it, and it is already the
roadmap's named outstanding gap. `button` and `toggle` also need their own hover
and press state before they can be used in a toolbar.

Also joining otto-kit, because more than the file components need them:
`filetype` (MIME resolution, magic sniffing, the kind classification) and the
thumbnail cache, both defined in [file-browser.md](./file-browser.md) under
*Shared foundations* — quick view consumes both and must not have to depend on
a file manager to do so.

Staying in the file components, because they encode this application's
semantics rather than a shape: the directory model, the worker pool and
watcher, the natural-order comparator, and the confirmation sheet.

## Out of scope for v1, explicitly

Column view. Drag and drop, in or out. The toolkit supports it and the browser
does it, but the picker deliberately does not: dropping files into the
directory it happens to be showing is file management, which is the browser's
job, and the picker refuses the drop rather than half-doing it.
Recursive search. Content-based type detection. Running external thumbnailers.
Mounting unmounted volumes. Remote filesystems. Previews beyond the thumbnail.
Tagging, starring, or any metadata Otto would have to invent a store for.
Persisted column widths.

## Resolved decisions

- **No "recent files" place in v1.** It needs `recently-used.xbel`, and nothing
  else in Otto reads or writes it. A recents list containing only what Otto's
  own picker did is worse than no recents list — it looks broken rather than
  empty. Revisit when something else in the desktop writes that file.
- **The per-app last-directory memory is the picker's own state file**, at
  `$XDG_STATE_HOME/otto/file-picker-dirs`: `app_id` to absolute path, one per
  line, a bounded LRU of 64 entries. State, not configuration — it is not
  hand-edited, not a preference, and has no business in the settings schema or
  in a file the user is invited to edit.
- **Save-mode accept does not create the file.** The portal contract does not
  require it, no other implementation does it, and a zero-length file left
  behind when the application then fails to write is a worse outcome than the
  race it would close. What does help is checked instead: the destination
  directory's writability is tested at accept time, and a failure is reported
  there rather than becoming the application's problem.

## Open Questions

None outstanding. Both remaining dependencies are recorded under
*Prerequisites*, not here — they are known work, not unresolved requirements.
