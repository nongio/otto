# File Browser

**Status:** draft — nothing implemented
**Wire contract:** `org.freedesktop.FileManager1`, defined inline below
**Related specs:** [file-picker.md](./file-picker.md), [quickview.md](./quickview.md),
[launcher.md](./launcher.md), [context-menus.md](./context-menus.md),
[settings-app.md](./settings-app.md)

## Summary

`otto-files`, the standalone application for browsing and managing the
filesystem — Otto's Finder. It is the second shell over the view layer defined
in [file-picker.md](./file-picker.md): the same directory model, async I/O,
watching, sorting, thumbnails and list/icon/column presentations, wrapped in a
document window that can act on files instead of merely returning their names.

This spec also owns the **shared foundations** that other components depend on:
the thumbnail cache, file-type detection, and the quick-view invocation
contract. Those are defined here once, and consumed — not redefined — elsewhere.

## Goals

- Navigate the local filesystem, see what is there, and open a file in its
  default application.
- Perform the file operations a desktop is expected to have: rename, new
  folder, copy, move, move to trash, delete permanently — each on a background
  worker, with progress, cancellation that leaves nothing half-written, and
  undo.
- Open a directory of 10,000 files responsively, with the same guarantees the
  picker makes: first rows within a frame of the first batch, full frame rate
  while scrolling, and no filesystem call on the UI thread.
- Other applications can ask Otto to show a file or folder through the standard
  `org.freedesktop.FileManager1` interface, so "Open Containing Folder" in a
  browser or editor lands here.
- Every part of the window is reachable from the keyboard, with the same
  navigation, type-ahead and multi-select semantics as the picker — a user who
  learns one has learned the other.
- Share the picker's code, not merely its behaviour. A fix to sorting or
  thumbnailing lands in both at once.

## Non-Goals

- Being a picker. The browser never returns a selection to another process, and
  has no accept/cancel action.
- Decoding previews. Pressing space draws a panel, but the bytes are parsed by
  quick view's sandboxed worker ([quickview.md](./quickview.md)); the browser
  draws a validated payload and never interprets a file itself.
- Network and virtual filesystems: `smb://`, `sftp://`, MTP, GVfs backends of any
  kind. The browser deals in local paths.
- Mounting, unmounting, formatting or ejecting devices. Already-mounted volumes
  appear; udisks2 integration is not in v1.
- Searching file contents, or any persistent index. See Non-Goals in
  [launcher.md](./launcher.md) — the same reasoning applies.
- A scripting or plugin interface.
- Being configurable by theme file. It follows the desktop's colour scheme and
  icon theme.

## Behavior

### Relationship to the picker

One crate, `components/otto-files`, produces a library and two binaries:
`otto-files` (this spec) and `otto-file-picker`
([file-picker.md](./file-picker.md)). The library holds the directory model,
the worker pool, the watcher, the thumbnail cache, file-type detection, the
sort comparators, the glob matcher, and the three view presentations. The two
binaries hold their chrome and their meaning of "done".

Everything in [file-picker.md](./file-picker.md) under *The view model*, *Async
I/O*, *Thumbnails*, *Filesystem watching* and *Sorting* applies here unchanged
and is not restated. Where this spec and that one disagree, that one wins for
the shared layer.

### The window

A client-decorated toplevel. Layout:

- **Titlebar** — the current directory's name.
- **Toolbar** — back/forward, parent, the view switcher (list / icon), a sort
  control, a new-folder action, an action menu, and a search field.
- **Path bar** — clickable ancestor segments; ancestors collapse into an
  overflow control when the path is too long for the width.
- **Sidebar** — places: the XDG user directories that exist, the user's
  bookmarks, Trash, and currently mounted volumes. A place takes a drop, which
  files into that place's directory; dragging a directory onto the sidebar to
  *bookmark* it is a separate gesture and is not yet built, so bookmarking is
  still a menu action on the selection.
- **File view** — the shared list / icon / column presentation. List and icon
  ship in v1. Every scrolling pane draws **only the rows its scroll view is
  asking for**: the visible band, taken from the scroll view's own content
  rect, and inclusive at both ends so a row resting on an edge keeps its
  stripe and hairline. A rubber-banded pane draws the rows the pull brings
  into view, and a Miller column panned off the window draws none. Row
  geometry comes from one walk shared by drawing, hit-testing and the Quick
  View anchor, so what is painted and what is clickable cannot disagree. This
  is what the 10,000-file goal above rests on — measuring and shaping the text
  of a row nobody can see is most of the cost of a frame.
- **Thumbnails in place of icons** — an entry whose thumbnail is available is
  drawn as itself rather than as its type's icon, in all three presentations:
  list rows, grid cells, and the column view's rows. The three take different
  routes to the screen — the first two draw immediately, while a Miller column
  records its rows into a cached picture — so a thumbnail reaching one of them
  says nothing about the others, and column view is what the browser opens in. The picture is **fitted, never cropped**: it keeps its own proportions
  inside the box the icon would have had and sits on the same baseline, so a
  panorama and a portrait line up with the icons around them, and it is never
  enlarged past its own pixels. A hairline closes its edge, without which a
  photograph with a pale sky has no boundary against the window. Everything
  with no thumbnail — every folder, and every file where none was found —
  keeps its icon, so the two are always mixed and must agree on geometry.
- **Only what is on screen is fetched, a few at a time** — the visible range of
  the pane the user is looking at, capped at four outstanding fetches. A folder
  of ten thousand pictures therefore costs what a folder of thirty costs. A
  fetch that finds nothing is remembered for the window's lifetime so it is not
  retried every frame, and a thumbnail landing invalidates only the panes that
  could show it, through the pane content key.
- **Row density** — rows are compact and **abut with no gap between them**, in
  list and column views alike: a listing is scanned rather than read, and the
  fewer pixels between one name and the next, the more of the directory the
  eye takes in at once. Because rows touch, a selection highlight fills its
  row's full height, and a **contiguous run of selected rows is drawn as one
  shape**: rounded on top only at the run's first row, rounded on the bottom
  only at its last, square where one selected row meets the next.
- **Icon-view selection is icon-sized** — cells do not touch, so the rule above
  is inverted: a cell-wide wash would read as a block of colour rather than as
  one picked-out file. The highlight is a rounded square standing a few points
  off the icon on every side, and the caption wears a pill of its own beneath
  it. The two meet edge to edge and read as one highlight.
The sidebar and the header band are translucent materials over the
compositor's backdrop blur, the sidebar heavily tinted and the header only
slightly, so the frost runs across the whole top of the window. The file area
below the header is opaque: a wall of small text needs one flat ground.

The **traffic lights** sit at whichever end of the window the desktop's
`window_controls_side` setting names — see
[window-decorations](window-decorations.md). At the leading end they sit in
the window's top corner, over the sidebar. At the trailing end they share the
header band with the view switcher, so they take the switcher's centre line
and the switcher steps left to clear them.

An **unfocused window steps back**: the title and subtitle drop a step down
the text scale, the traffic lights go gray, the accent is muted almost to
gray, the blur behind the window is dropped, and the sidebar, header and
action row are filled in to full opacity since there is no longer anything
blurred for them to be translucent over. Muting the accent covers everything
drawn in it — the selected rows and grid captions, the selected place in the
sidebar, the cursor ring, the drop outline, the rubber band and the picker's
accept button — so only the focused window carries the user's colour, and a
desktop of open browsers has one place for the eye to go. It stops short of a
flat gray: a trace of the hue keeps the window looking like the same desktop.
This is the toolkit's behaviour rather than the browser's — see
[otto-kit-window-focus](otto-kit-window-focus.md) — and applies to the
picker's window as well, minus the traffic lights it does not have.

- **Status bar** — the entry count, the selection count and its total size, and
  the free space on the filesystem holding the current directory. It is the
  progress and cancel surface for a running operation.

Multiple windows are supported and are independent. Tabs are not in v1.

### Column view scrolling

The column (Miller) view scrolls on two axes, and they are two independent
scroll views, not one nested inside the other:

- **Each column scrolls vertically on its own**, with its own overlay bar on
  its right edge — a column can be scrolled to its end while its neighbour
  rests at the top.
- **The stack pans horizontally as a whole**, on a horizontal scroll view
  whose bar runs along the bottom of the file area. The stack is one
  continuous strip that happens to be divided into panes, so panning it is a
  scroll: it flings with momentum, stretches with rising resistance past
  either end, and springs back — the same physics, from the same widget, as
  the vertical panes.

The two never fight over one gesture. **A touchpad gesture picks its axis from
its first delta and keeps it until the fingers lift**, so a diagonal swipe
pans or scrolls, never both by turns.

**A gesture places the stack freely; navigation aligns it.** A fling rests
wherever it lands, with a partly visible column at each edge — that is a
normal resting state and nothing snaps it straight. Moving into a column by
click or keyboard is what aligns: the stack pans the shortest distance that
brings that column fully into view, landing on its exact edge, and drops any
fling still in flight first so the two are never both moving it.

**The window's own border outranks a column divider.** Both are grabbable
edges and their bands overlap: the last pane's right edge sits exactly on the
window's right border whenever the stack is panned fully over, which is the
ordinary resting state once a preview column is up, and the divider's grab band
is the narrower of the two. A press resolves the window border first, so the
cursor must too — otherwise it promises a column resize while the click
underneath it resizes the window.

### Column surfaces

Painting every column into the window's one buffer makes a scroll in a single
column a repaint of the whole window, and — because the toolkit damages the
whole buffer on commit — tells the compositor that everything changed. Under
`OTTO_FILES_PANE_SUBS=1` each column instead gets its own Wayland subsurface,
sized and positioned by `otto_surface_style_v1`. The client still does all the
drawing, translating and clipping itself; what changes is the *scope*, so a
scroll damages one column and leaves the toplevel alone.

This is opt-in while it settles. The default path is unchanged.

Three things follow from the columns no longer being in the window's buffer:

- **Input still belongs to the toplevel.** Every column surface carries an
  empty input region, so pointer events fall through and hit-testing stays in
  window coordinates exactly as it was.
- **Nothing above clips the columns.** In the window they were cut off by the
  content area's clip; a subsurface is a child of the toplevel and has no such
  parent, so a column panned past the sidebar would draw over it. Each column
  surface is cropped to its intersection with the content area instead, and
  its drawing shifted by whatever was cropped off the left.
- **Scrolling chrome has to move with the content it describes.** A column's
  vertical bar is drawn into that column's own surface, since the window is no
  longer repainted for a scroll and a bar left there would fade in and freeze.
  The stack's horizontal bar belongs to no column, so it gets a surface of its
  own — a strip along the bottom of the content area, stacked above every
  column, and restacked whenever the stack grows, because a subsurface created
  later starts out on top of it.

The hairlines between columns are still drawn in the window, so a sideways pan
— unlike a vertical scroll — does still repaint it, or they would be left
behind while the columns slide.

### The preview column

Column view ends in a preview of the single selected entry, when there is room
for one. It is laid out from the bottom up — the facts, then the name above
them, and whatever is left over is the preview's — so the name and the facts sit
on the same line whatever the file is, instead of riding up and down with the
size of the thing above them.

**Everything is cropped to fit.** The name and each fact are truncated with an
ellipsis to the column's width, and the stage above them is clipped, so no
preview can draw over the caption or out of the column. This is not left to the
decoders being well behaved: the content belongs to a *file* — an archive with
hundreds of long entry names, a text file with no line breaks — and the crop is
what makes the column's size a property of the column rather than of whatever
was selected. In a listing (an archive's contents) the size column is reserved
on every row whether or not that row has a size, so the names crop to a common
edge and the listing reads as a column instead of a ragged one.

**A card falls back to a picture.** A metadata card is a title, a subtitle and a
table of facts, and this column already carries every one of those in the
caption below — drawn as a card it says the same things twice, in the space
meant for the thing itself. What the card has that the caption does not is its
artwork: cover art, an embedded thumbnail, an mp4's poster frame. That is shown
as the picture it is, and a card with none falls back to the file's own icon,
drawn large.

**A decoder that gave up is not a blank panel**, and neither is one still
running: the file's own icon is still true, and drawn large it is a preview of a
kind rather than a placeholder apologising for itself.

**Selecting does not pan to it.** The preview opens where it is; the stack is
aligned by navigation, not by a selection changing what the last column holds.

### Places

The sidebar's places come from three sources:

1. The XDG user directories — Desktop, Documents, Downloads, Music, Pictures,
   Videos, Public, Templates — read from `user-dirs.dirs`, and shown only when
   the directory exists. Home is always first.
2. The user's bookmarks, from Otto's own `$XDG_CONFIG_HOME/otto/places`: one
   absolute path per line, optionally followed by a display name. The browser
   is the only writer. On first run, if that file does not exist and
   `$XDG_CONFIG_HOME/gtk-3.0/bookmarks` does, it is read once to seed Otto's,
   and never read again.
3. Mounted volumes, from `/proc/self/mounts`, filtered to mounts under
   `/run/media`, `/media` and `/mnt` and to filesystems that are not virtual.

Bookmarks can be reordered and removed. A bookmark whose path no longer exists
is shown dimmed and offers removal rather than silently disappearing — a
disconnected drive is not a deleted bookmark.

Trash is a place. Selecting it lists the trash contents with their original
locations, and offers Restore and Empty Trash. Files in Trash cannot be opened
or renamed in place.

### Opening

Double-click, or Ctrl+O. Return is not the key here — it is spelled for rename,
the way it is on macOS. Double-click opens in every view: in column view a
directory is already open by the time the second click lands, since that view
shows a child eagerly on the first, but a file there needs the double-click like
anywhere else.

**Opening is acknowledged before it happens.** A ghost of the whole selection —
its highlight, its icon and its name, drawn by the same code that drew it in the
listing — grows out of itself and fades over 280ms. Handing a file to another
process or re-reading a directory can take long enough that a double-click with
no answer reads as one that did not land. The ghost echoes the selection rather
than the icon alone, so it reads as *that file* opening: in the grid the caption
pill goes with it, in the row views the highlight band does.

- A directory is navigated into, in this window. **Ctrl+double-click opens it
  in a new window instead** — a second `otto-files` process pointed at that
  directory, since the app shell is one toplevel per process. The Ctrl+click
  that starts the pair still toggles the row into the selection the way a
  single Ctrl+click does; the second click opens rather than toggling back out.
  In the picker, which answers one request in one window, Ctrl+click only ever
  toggles.
- A file is handed to `xdg-open`, started detached — stdio closed, reaped on a
  thread of its own — so the application outlives the browser. The desktop's own
  answer to "what opens this", rather than one resolved here: it already knows
  about `mimeapps.list`, the portal and the fallbacks, and a file manager that
  disagreed with the rest of the session about what opens a `.pdf` would be the
  thing that was wrong.
- An executable file is *not* run. It is opened in its default application like
  any other file, or reported as having none. Running a program by
  double-clicking it in a file manager is a well-known way to be tricked into
  running one.
- A file with no association shows an "Open With" chooser listing applications
  that declare support for its type, plus every other installed application
  behind a disclosure. Choosing an application optionally sets it as the
  default, which writes `mimeapps.list`.
- A broken symlink reports that its target is missing, naming the target.

`Open With` is always available on the context menu.

### Navigation history

Back and Forward are the two halves of one **split button** in the header: a
single rounded capsule divided by a hairline seam, matching the view switcher
at the other end of the header. Each half dims when there is nowhere for it to
go, and neither half is a window-drag area.

Every navigation that moves the window somewhere else — descending into a
directory, going up, clicking a place — first records where it was; navigating
anew drops any Forward history, the rule a web browser follows.

A recorded location is the **whole column stack**, not just the deepest path,
and for each pane both its selection and its cursor. Going back therefore
restores every pane that was open *and* the entry that was selected in each,
scrolled back into view. The selection is remembered by name, so it survives
the re-read of the directory; the cursor is re-derived from that selection once
the listing lands, so a file added or removed while the user was away cannot
leave the keyboard one row off from the highlight.

### Selection, keyboard and type-ahead

Identical to the picker, including the type-ahead-is-not-search distinction, the
anchor-based multi-select rules and rubber-band selection. See
[file-picker.md](./file-picker.md) under *Keyboard*.

A plain click on empty space inside a pane — past the last row, between grid
cells, on the background — selects nothing, in every view. The pane still takes
The "Loading" placeholder belongs to a *first* read only. An in-place re-read —
after a delete, a paste, or a watcher notification — leaves the listing that is
already on screen up until the new one lands, because the alternative is the
whole pane blinking through an empty placeholder and back for every operation.

the keyboard, since the click was in it, and in column view the panes to its
right close with the selection: they are there because something in that pane
was selected, and now nothing is. Clicks on the header, the sidebar and the
status strip are not clicks on nothing and change no selection.

**The rubber band.** In icon view that click is the corner of a rubber band: a
press on empty space and a drag sweeps a rectangle out over the grid, and
everything it touches is selected. A cell counts as caught the moment the band
touches its rect at all, so a band flicked through a row of icons takes the
row; enclosing each one is not required. The selection is recomputed from the
band on every motion rather than accumulated, so pulling the band back off an
entry gives it up again. A band with no extent at all — a click that never
travels — catches nothing, which is exactly the "selects nothing" rule above:
the two are one behaviour, not two.

Held with Ctrl or Shift the band *adds* to what was already selected, the way
Ctrl+click does, and the press does not clear first. The band is anchored in
the pane's content coordinates, not on the screen, so scrolling the wheel
mid-drag keeps it around the same files and lets it reach past the bottom of
the window. Releasing puts the band away and leaves the selection it made.

There is no band in list or column view. Rows there span their pane's whole
width, so a rectangle could only ever say what dragging down the rows already
says; the plain click-clears rule is the whole of the empty-space behaviour in
those two.

The browser adds:

| Key | Effect |
| --- | --- |
| Space | quick view of the selection (see below) |
| Return / F2 | rename the entry at the cursor, inline, with the extension unselected — in every view mode, icon view included |
| Delete / Ctrl+Delete / Ctrl+Backspace | move the selection to trash — the modified forms because the chord people reach for is Cmd+Delete, and on a keyboard whose big key is Backspace that arrives as Ctrl+Backspace. Plain Backspace goes up a directory instead, and always has |
| Shift+Delete | delete the selection permanently, after confirmation — **not built**; the chord is deliberately inert rather than trashing, which is the wrong answer to a keystroke that means "and I mean it" |
| Ctrl+C / Ctrl+X / Ctrl+V | copy / cut / paste. These are file management, so the picker does not have them — and while an inline rename holds the keyboard they act on the *name being edited* rather than on the selection, as every other key in the field does |
| Ctrl+Z | undo the last operation |
| Ctrl+N | new window, at the **default location** (the home directory for now, a preference later) rather than at this window's directory — a new window is a fresh start, and inheriting wherever the focused window was pointed makes it read as a copy of that window. Ctrl+double-click is the gesture for "that directory, in another window". Nothing in the picker, which answers one request in one window |
| Ctrl+O | open the entry at the cursor — exactly what a double-click does: descend into a directory, or activate a file (in the picker, accept it). Return is not free for this, since it renames |
| Ctrl+I | show info for the selection |
| Ctrl+1 / Ctrl+2 / Ctrl+3 | list / icon / column view |
| Escape | cancel an inline rename, else clear the search field, else clear the selection |

Return renames, in every view mode; F2 is an alias for it, for the Linux
convention. Opening is a double-click or Right arrow, never a key that is one
mistake away from launching everything in a selection. The picker is the
exception: it does no file management, so there Return means "this one" —
descend into a directory, or accept the selection.

### File operations

Every operation runs on the worker pool. The UI thread starts it, watches its
progress, and can cancel it.

- **Rename** — inline in the view. A name containing `/`, or empty, or `.`
  or `..`, is rejected while typing. A name that collides with an existing
  entry offers to replace it or to keep editing.
- **New folder** — creates `untitled folder`, disambiguating with a numeric
  suffix, and immediately enters inline rename on it.
- **Copy and move** — the clipboard holds paths and a copy/cut intent. Paste
  into a directory starts the operation, and so does a drop on one: a drag is
  the same operation reached with the pointer, and both run through the same
  code with the same conflict rules. A move within one filesystem is a
  rename syscall and is instantaneous; a move across filesystems is copy,
  verify, then unlink the source, and the source is unlinked only after the
  destination is fully written and fsynced.
- **Conflicts** — when a destination entry exists, a sheet offers Replace,
  Skip, Keep Both (numeric suffix), each with an "apply to all remaining"
  option. Directories merge rather than replace; the conflict question is asked
  per file inside them.
- **Cancellation must be clean.** Every copied file is written to a temporary
  name in the destination directory and renamed into place on completion, so a
  cancelled or crashed operation leaves no truncated file wearing the real
  name. Temporary files left by a crash are named recognisably and cleaned on
  the next operation into that directory.
- **Move to trash** — the freedesktop trash specification: the file is moved to
  `$XDG_DATA_HOME/Trash/files/`, with a `.trashinfo` recording its original
  path and the deletion time, both under a name disambiguated against what is
  already there. **v1 trashes only files on the same filesystem as the home
  trash.** Trashing on another mount requires a `.Trash-$uid` directory at that
  mount's root, with its own rules about creation and stickiness; until that
  exists, the browser says plainly that the file is on another volume and
  offers permanent deletion instead. It never silently copies a file across
  filesystems in the name of trashing it.
- **Delete permanently** — always confirmed, always says it cannot be undone,
  and is not undoable.
- **The selection survives a delete.** It moves to the entry that takes the
  deleted one's place: the first survivor below the deleted run, and failing
  that the nearest one above it, so holding Delete clears a run of files
  without reaching for the mouse between each. Which entry that is is decided
  against the listing on screen, before the re-read replaces it. A pane left
  with nothing in it hands the keyboard back to its parent, where the folder
  itself is selected; in column view the empty pane stays on screen, since it
  is what the parent's selection points at.
- **Undo** — a stack, within the session, 32 operations deep. A move is undone
  by moving back; a trash by restoring from trash; a copy by taking away what
  was created; a rename by renaming back; a new folder by removing it if still
  empty. A permanent delete is not undoable and the undo action says so rather
  than being silently absent.

  Undoable means *changed a file*. Selecting, navigating, sorting and switching
  view are not on the stack: a Ctrl+Z that could spend itself taking back a
  click would make the ones that take back a delete unreliable, because the
  user would never know which of the two the next press was going to reach.

  What goes on the stack is the list of changes an operation actually made, not
  the operation as asked for. A paste of ten files that moves eight, skips one
  and fails on one records the eight, and undoing puts back exactly those. A
  copy that replaced an existing file records nothing for that file — removing
  the copy would not bring the original back, so it is honestly not undoable.

  **Nothing undo does is itself unrecoverable.** Taking back a copy trashes it
  rather than deleting it; only a directory this browser created, and that is
  still empty, is removed outright. An undo that put a file back where the name
  is now taken says so and leaves both files alone rather than choosing for the
  user. There is no redo: undoing a delete is a *restore*, and re-deleting it
  would be a second trip to the trash rather than the inverse of anything.
- **Progress** — an operation shorter than 500 ms shows nothing. Beyond that,
  the status bar shows the current file, the count, and a cancel action; per-byte
  progress appears for files above 32 MB.
- **Errors** — a failure part-way through a multi-file operation stops and
  reports which files were done, which failed and why, and offers to continue
  with the rest or to stop. It does not abort silently and it does not retry
  in a loop.

### Sounds

File operations are audible, from the desktop's XDG sound theme — the same
player the compositor uses for its volume clicks and its lock chime, so the
browser sounds like part of the desktop rather than like an application with
opinions about audio. Otto publishes the theme name over the settings portal
(`org.gnome.desktop.sound theme-name`) and otto-kit plays through it.

The sound follows the **effect**, not the command:

| Outcome | Sound |
| --- | --- |
| files arrived — a paste, a drop, a restore | `drag-accept`, else `device-added`, else `complete` |
| files went away — a delete, or an undo that took a copy back | `trash-empty`, else `device-removed` |
| nothing happened | silence |

Choosing from the outcome is what makes undo sound right with no special case:
undoing a delete is a restore, so it gets the arriving sound; undoing a copy
takes files away, so it gets the other one.

The preference order exists because the sound naming spec is thinner than a
desktop needs — there is no "paste" event — and theme coverage of the drag
events is patchy. The first name the installed theme actually has is the one
that plays. A theme with none of them is silent, which is a normal outcome and
not an error.

### Drag and drop

Files are dragged out of the browser and dropped onto it, including onto itself.
Both directions speak `wl_data_device`, so a drag to another application is the
same gesture as one within the window.

**Starting a drag.** A press on a row arms; travelling `6pt` with the button
still down starts. Below that the press stays an ordinary click, so an unsteady
hand still selects and a double-click still opens. The whole selection travels,
not the row under the pointer: dragging one of several selected files takes all
of them. The payload is the same three types a copy puts on the clipboard —
`x-special/gnome-copied-files`, `text/uri-list`, `text/plain` — which is what
makes the drag legible to other file managers and to text editors.

**The preview column is a handle too.** In Miller view the trailing preview
pane is a picture of one file, drawn large, and pressing it picks that file up
exactly as pressing its row does — the same `6pt` arm, the same payload. The
picture is what lifts off: the drag image is the preview at the size it is on
screen, so the file travels looking like the thing the eye was actually
resting on rather than like a row it is nowhere near. Only the picture is a
handle; the caption of name and facts along the foot of the column is a label,
and stays one.

**Nothing expensive may happen between the press and `start_drag`.** The
compositor honours the request only while the pointer grab that authorised it
is still held, and it refuses a stale one *silently* — no error, no drag, and
nothing on screen to say why. Building the drag icon's EGL and Skia surfaces
before sending the request cost enough to lose a quick drag outright: the
button was back up before the request arrived. So the icon surface is created
bare, the drag is started, and only then is the previous drag's icon torn down
and this one given something to draw with. A drag that a user makes briskly —
which is most of them — depends on that order.

**The drag image is shaped like the view it came from.** Every dragged file is
drawn, each where it sits on screen right now and with the alignment its view
gives it, so the picture under the cursor reads as the files being carried
rather than as a new object invented for the drag. Icons are drawn plain and
only the *name* is highlighted: the accent pill behind a name says "this one is
coming" without the picture turning into a second selection rectangle over the
one still showing in the window. The thumbnail travels with it where there is
one. Up to fifty are drawn — the fifty nearest the entry grabbed, put back in
listing order so the pile stacks the way the eye saw it — and a count badge at
the cursor's bottom right says how many are coming in total. The cap bounds the
drag surface as much as the clutter: the picture is one surface the size of the
bounding box of what it holds, so drawing the whole of a select-all would ask
for one as tall as the listing.

The badge is the palette's **red**, not the accent. The names travelling under
it are already highlighted in the accent, and a badge in that same colour reads
as one more of them instead of as the count of them.

**Taking a drop.** Every target resolves to a *directory*: a directory row or
cell, a pane's background, or a sidebar place. Dropping on a file is not a
thing, so a hit on one lands in the directory that file is in. The target is
outlined while the drag is over it — an accent ring, drawn over the rows, whose
corners take the shape of what it outlines: rounded around a grid cell or a
sidebar place, which have rounded highlights of their own, and square around a
Miller column, a pane or a row band, where a rounded ring would read as a
separate object floating over the target rather than as the target being picked
out — and resolved again at the drop's own position rather than trusting the
last motion, since a release may land somewhere no motion reported.

**The conversation is per-position.** The browser answers every enter and every
motion, even to say no: a target that accepts once and goes quiet has told the
source it stopped accepting. Over anything that is not a directory, or under a
drag carrying no files, it answers `None`, and the cursor says the drop will be
refused.

**A drop is a paste.** It runs `paste` with the same `KeepBoth` conflict rule,
so a drop cannot destroy an existing file, and a directory dropped into itself
is refused with a message by the same check the clipboard path uses. Files
dropped back into the directory they already live in are skipped when the action
is a move — there is nothing to do — but kept when it is a copy, which is how a
duplicate is made.

**A move is performed by the receiver.** The source deletes nothing when the
drop finishes. A foreign target that accepted `text/uri-list` has not
necessarily written those files anywhere, and deleting on its say-so would lose
them; the receiving file manager has the paths and can do the move itself.

**A drop from this window is served from memory.** Source and target are one
thread, so asking for the payload over the pipe would block that thread waiting
for a write only it could make. The drag's own payload is read directly instead,
and the source is still told the transfer finished.

**A refused drop flies home.** The compositor animates the icon back to the
point it was grabbed by — `initial position + grab offset`, not the cursor's
start, or it lands short by however far into the file the user happened to
press, which is a different distance every time. An accepted drop is not
animated: the files are where they were asked to go and the listing says so, and
an icon flying away as well is a flourish over an answer already given.

The picture is taken down **when the flight lands**, not when the next drag
sweeps it up. An icon has to outlive its own drag — that is what the flight is
— but tying its teardown to the next drag leaves it, and the buffer behind it,
in the scene for the rest of a session in which the user drags once. The
teardown rides an animation of its own rather than the flight's `on_finish`: a
later `set_position` on that layer replaces the transaction and drops its
handlers without a word. The next drag still sweeps, as a safety net for a
flight that never lands and for an icon whose client exited mid-air.

**A drop does not move the view.** The reload a drop causes keeps every pane
scrolled exactly where it was, and the entry that landed is not scrolled to.

With `OTTO_FILES_PANE_SUBS=1` the Miller columns are subsurfaces over the
window's own canvas, so the drop outline is hidden behind them; that mode is
opt-in and needs its own drop feedback.

### Get Info

A panel for the selection: name, full path, type, size (recursive for a
directory, computed on a worker and updating as it counts), created, modified
and accessed times, permissions, owner and group, the default application, and
a preview thumbnail. Permissions and the default application are editable;
everything else is read-only.

**It is a window of its own, not a sheet.** Ctrl+I opens a second toplevel and
Ctrl+I again closes it; it is dragged by its top strip, dismissed by its close
dot or by a close asked for from outside, and the compositor gives it the same
shadow and stacking as any other window. It carries the browser's own `app_id`,
so it groups under the file manager's dock icon rather than adding one of its
own.

- **The browser is not dimmed and not blocked.** The panel is not modal: the
  file list goes on scrolling, selecting and opening behind it while it is up.
  Escape closes it, taking its turn in the same unwinding order as everything
  else that is up rather than owning the key outright.
- **The panel opens on the selection and is not re-targeted.** Arrow-keying in
  the browser moves the cursor without changing what the open panel is about;
  a panel for another file is another Ctrl+I.
- **A close request for the panel closes the panel.** A secondary window's
  close is not the application's, and must not end the process.

### Quick view

Pressing space with a selection previews it. Quick view
([quickview.md](./quickview.md)) is a **library the browser embeds**, not a
service it calls: the panel is drawn into the browser's own surface, and the
decoding happens in a sandboxed worker process.

**This replaces an earlier `org.otto.QuickView1` D-Bus contract**, which is
deleted. It is recorded here so nobody reconstructs it: a subsurface's parent
must be a `wl_surface` owned by the same client, so "the preview is parented to
the file view" and "the previewer is a separate process" cannot both be true.
Parenting also dissolves the anchor problem — the row's rect is already in the
browser's coordinates — and hands stacking, focus and dismissal to the browser's
window instead of leaving them to be managed by hand on an overlay.

What the browser does:

- **`run_worker_if_requested()` is the first statement in `main`**, before the
  async runtime starts a thread and before anything connects to Wayland. The
  sandboxed decoder is this binary re-executed, so without it a preview starts a
  second file browser instead of decoding a file.
- **Decoding runs off the UI thread.** The call blocks until the worker answers
  or its deadline expires; inline it would stall the frame loop for as long as
  the file takes.
- **Every decode carries a generation, and a stale result is dropped.**
  Arrow-keying is much faster than decoding, so a slow PDF must not land on top
  of a file the user moved off three keys ago. Dismissing also bumps the
  generation, so an in-flight decode cannot re-open a panel the user closed.
- **The browser never interprets file bytes.** It receives a validated payload —
  UTF-8 lines, bounded rows, or a pixel buffer whose dimensions were checked
  against its length — and draws it with `otto_kit::preview`, which is
  canvas-pure and shared with every other file view.
- **The anchor is the selected row's rect in the browser's own surface
  coordinates**, and the panel grows out of it. **An empty rect when there is
  nothing to grow from** — no cursor, or a row scrolled out of view or in a
  panned-away Miller column — is a documented answer meaning "open in place",
  not a missing value.
- **The browser keeps the keyboard.** Space toggles the panel, Escape dismisses
  it before it clears the selection, and the arrow keys move the cursor and
  re-decode in place rather than dismissing.
- **The panel owns the pointer while it is up**: a click outside dismisses, the
  wheel scrolls a listing or text preview, a pinch zooms an image and a
  two-finger scroll pans a zoomed one, and nothing reaches the file list
  underneath. Which of the two pointer handlers sees the gesture depends on
  where the panel is — the panel takes its own input when it is centred on the
  display and hangs outside the window, and the toplevel takes it otherwise —
  so both routes feed one decision about what the gesture means.
- Enter still opens the selection in its default application. The previewer
  never launches anything.
- **Losing the keyboard closes the panel.** A preview is a preview of what
  *this* window has selected; once focus is on another window there is nothing
  for it to be a preview of, and a card left floating over a background window
  is just litter. The signal is `wl_keyboard.leave` on the browser's own
  toplevel — a leave on the Get Info panel is focus moving between two of the
  browser's windows and does not count.
- **This is also how expose reaches it.** The panel is a subsurface, not a
  popup, so the compositor's popup dismissal on the way into Show All cannot
  take it down. Otto drops keyboard focus when expose opens (see
  `Otto::enter_expose_focus`), and the browser closes on that leave like any
  other. Restoring the panel on the way back is deliberately not done: expose
  ends by focusing a window, which is a fresh start.

Quick view **consumes** the thumbnail cache and the file-type detection defined
below, and may **produce** into the thumbnail cache under the same rules. It
must not define a second cache, a second cache location, or a second
type-detection path.

Get Info shows the cached thumbnail and nothing decoded. The preview panel is
the one place decoded payloads are drawn.

### Wire contract: `org.freedesktop.FileManager1`

The browser owns the standard interface other applications already call — bus
name `org.freedesktop.FileManager1`, object path `/org/freedesktop/FileManager1`,
interface `org.freedesktop.FileManager1`, D-Bus activated:

```
ShowFolders(uris: as, startup_id: s) → ()
ShowItems(uris: as, startup_id: s) → ()
ShowItemProperties(uris: as, startup_id: s) → ()
```

- `ShowFolders` opens a window at each URI that is a directory.
- `ShowItems` opens a window at each URI's *parent* directory with that entry
  selected and scrolled into view. This is what "Open Containing Folder" and
  "Show in Folder" call.
- `ShowItemProperties` opens the Get Info panel for each URI.
- URIs that are not `file://`, or that do not exist, are skipped with a log
  line; the remaining ones are still shown. A call naming only bad URIs opens
  nothing and returns successfully — it is not the caller's business that the
  file vanished.
- Several URIs in one directory are coalesced into one window with a multiple
  selection, not one window each.
- If a window is already showing the target directory, it is reused, its
  selection set, and it is raised.
- `startup_id` is used for startup notification where the compositor supports
  it, and ignored otherwise.

This is the standard interoperability contract rather than an invented
`org.otto.Files1`, so applications that already know how to reveal a file get it
for free.

## Shared foundations

These three are defined here and consumed by the picker, by quick view, and by
anything else that grows a need for them.

### 1. The thumbnail cache

**Implemented: the reading half.** `otto_files::thumbcache` resolves, validates
and reads shared-cache entries, and `otto_files::thumbnails` is the per-window
store and scheduler over it. The browser consumes what every other file manager
has already written — verified against the real cache on a developer machine,
where all 4,686 checkable entries agreed on the key and every live source file
resolved. **Otto does not yet write into the cache**: generation happens in
process and is kept in memory only, so the *Writing*, *Eviction* and *Other
Otto components may write into it* rules below describe the intended end state,
not current behaviour. Publishing into a cache the whole desktop reads is a
promise about the bytes and their size, and it is deliberately a separate step
from consuming it.

**It is the freedesktop shared thumbnail cache, not an Otto-private one.**
Location, keying and validity follow the freedesktop thumbnail managing
standard exactly, so Otto reads thumbnails other applications wrote and they
read Otto's:

- **Location** — `$XDG_CACHE_HOME/thumbnails/<size>/`, with `<size>` one of
  `normal` (128 px), `large` (256 px), `x-large` (512 px), `xx-large`
  (1024 px). Failures go in `$XDG_CACHE_HOME/thumbnails/fail/otto-files/`,
  keyed identically, holding a zero-content PNG.
- **Key** — the lowercase hex MD5 of the file's absolute, percent-encoded
  `file://` URI, plus `.png`. The URI is the key, not the path: it must be
  byte-identical to what another implementation would produce, or the caches do
  not interoperate. Neither size nor mtime is part of the key.
- **Validity** — the stored PNG carries `Thumb::URI` and `Thumb::MTime` `tEXt`
  chunks. A thumbnail is valid when `Thumb::MTime` equals the source file's
  current modification time in seconds. Anything else — missing chunk, differing
  time — is stale and regenerated. Size is not compared; the standard says
  mtime, and a second implementation checking something else means two
  processes each believing the other's thumbnail is wrong.
- **Writing** — write to a temporary file in the same directory, `fsync`, then
  `rename` into place, with mode `0600` and the cache directory `0700`. Two
  processes generating the same thumbnail concurrently is safe and expected;
  neither locks, and last writer wins because both wrote the same thing.
- **Eviction** — none by Otto for valid thumbnails. The cache is shared and
  contains other applications' data; unilaterally trimming it is not ours to do.
  Otto does prune its own `fail/otto-files/` entries older than 30 days, on
  idle.
- **The API is a library, not a bus interface.** `otto-files`' thumbnail module
  exposes pure functions over the filesystem — resolve a URI to a cache path,
  look up and validate, generate, store — and any process that wants a
  thumbnail links it and calls it. There is no thumbnail daemon and no D-Bus
  request/ready round trip. The filesystem *is* the cross-process contract, and
  it already is one: correctness across processes comes from atomic rename and
  from the mtime check, not from a coordinator. A second process may generate a
  thumbnail the browser is also generating; the cost is one duplicated decode
  and the benefit is no protocol.
- **In-memory decoded images are per process** and never shared. Each process
  keeps its own LRU bounded by count and bytes.
- **Other Otto components may write into it**, and quick view does: its decode
  worker already produces a scaled image of the file the user is looking at, so
  it stores the thumbnail rather than making the browser decode the same file
  again in a less isolated process. Any writer obeys the rules above exactly —
  the standard key, the `Thumb::` chunks, atomic rename, and **only the four
  standard size buckets**. An entry at a non-standard size is invisible to every
  other implementation and is not a thumbnail.
- **The cache holds small images only.** It is never asked for a
  full-resolution decode, and there is no "decode this at an arbitrary size for
  someone else" call. A consumer needing a large image decodes it itself.
- **What Otto generates** — images Skia decodes natively and SVG. It never runs
  an external `/usr/share/thumbnailers/*.thumbnailer` program, but reads
  whatever such programs left in the cache. Full reasoning in
  [file-picker.md](./file-picker.md).

### 2. File-type detection

One implementation, **in otto-kit as `otto_kit::filetype`**, not in
`otto-files`. It sits beside the icon lookup that already lives there, and every
component needs it: the picker for portal filters, the browser for icons and
the Kind column, quick view to choose a renderer. Putting it in the file
manager's crate would make the previewer depend on the file manager.

It answers two different questions, with two calls, and the distinction is
load-bearing:

**`mime_for_name(name) -> Option<&str>` — the type of record.** Decided by the
file's name, against the shared MIME database's `globs2`
(`weight:mimetype:glob`), highest weight first, a literal match beating a glob
and a longer glob beating a shorter one — the standard's own precedence.
`subclasses` supplies the hierarchy, so asking "is this `text/plain`" is true
for `text/x-rust`. Both files are line-oriented plain text; **no XML parser and
no `mime.cache` binary format is involved**, which is why the full database is
affordable rather than a hardcoded table. It has to be the full database: a
portal request may name any MIME type at all, and the browser's Kind column has
to name types nobody enumerated in advance.

This is the answer used for the icon, the Kind column, portal filters, and
default-application association. It is stable, cheap, and needs no I/O beyond
the database load.

**`sniff(bytes) -> Option<&str>` — what the content looks like.** A bounded
magic-byte check over at most the first 4 KB, covering the signatures that
matter for safety and for decoder dispatch. It performs no I/O itself; the
caller supplies the bytes it already read.

**They do not compete, because they are not asked the same thing.** Display
follows the name: an empty `.rs` file is Rust source, and an icon that flips
because a sniff was inconclusive is a bug the user sees. Decoding follows the
content: **a consumer that is about to parse a file must dispatch its decoder on
`sniff`, never on the name**, so a `.png` that is really something else is never
handed to the PNG decoder. Quick view and the thumbnailer both do this. Where a
consumer wants to report the disagreement — "this file is named `.png` but is
not one" — it has both answers and can.

- Directories are `inode/directory`. Symlinks resolve to their target's type for
  display, and report brokenness separately. Devices, sockets and FIFOs get
  `inode/*` types.
- A small **kind** classification over MIME types — image, video, audio, text,
  document, archive, application, folder, other — is what the Kind column
  shows, what the icon lookup falls back to, what decides whether a thumbnail is
  attempted, and what quick view dispatches its renderer on. It is a lossy
  convenience over the MIME type, not a replacement; consumers needing precision
  use the MIME type.
- The glob matcher is shared with the picker's portal filters. A portal MIME
  filter is resolved through exactly this path.
- `filetype` must be callable from a bare canvas context: no `AppContext`, no
  `wayland-client`, no runtime — the same constraint every otto-kit draw
  function is under.

### 3. Icon resolution

The type icon for an entry comes from the icon theme: the MIME type with `/`
replaced by `-`, then the generic type (`text-x-generic` and friends), then the
kind's fallback, then a final unknown-file icon. Resolution goes through
otto-kit's `find_icon_in_theme` / `cached_file_icon`, never `named_icon_sized`,
which reaches into `AppContext` and is unavailable off the client runtime.

## Constraints & Edge Cases

- **A file operation must never lose data.** The two rules that guarantee it:
  a cross-filesystem move unlinks the source only after the destination is
  written and fsynced, and every write goes to a temporary name and is renamed
  into place. Everything else is recoverable; violating either is not.
- **Deleting the directory being viewed** navigates to the nearest surviving
  ancestor, and a pending operation targeting it fails with that reason.
- **An operation whose source or destination changes underneath it** (the user
  moves a file being copied) fails that file and continues, reporting it in the
  summary.
- **Free space** in the status bar is the filesystem holding the current
  directory, and a paste that obviously will not fit is refused up front with
  the numbers, rather than filling the disk and failing part-way.
- **Trash restore into a directory that no longer exists** recreates the
  directory path if it can, and otherwise asks where to put the file.
- **Non-UTF-8 names** survive every operation. Names are bytes throughout;
  display is lossy, the model is not. A rename dialog on a non-UTF-8 name is
  editing a lossy rendering and must say so rather than silently rewriting the
  bytes.
- **Permissions.** An operation needing privileges the user does not have fails
  with the reason. The browser never escalates, never invokes `pkexec`, and has
  no "run as administrator".
- **The clipboard holds paths, not contents.** A cut whose source disappears
  before the paste fails cleanly. A cut is not applied until pasted, so the
  source is never removed on Ctrl+X.
- **Two windows on the same directory** share the model and the watch; an
  operation in one is reflected in the other through the ordinary watch path,
  not through a special case.
- **The trash can be enormous.** Listing it must be as lazy as any other
  directory, and Empty Trash is an ordinary cancellable background operation
  with progress.
- **Must run under the windowed development backend**, and must not require the
  compositor to be built with development features.

## Rationale

**Two shells over one view layer, and the browser is where the shared layer
lives.** The picker is the harder deadline (applications are broken without it)
but the browser is the fuller consumer of the model — it needs everything the
picker needs plus mutation. Putting the library with the browser and having the
picker link it, rather than the reverse, means the shared code is exercised by
the more demanding caller.

**`org.freedesktop.FileManager1` rather than an Otto interface.** Firefox,
Thunderbird, editors and chat clients already call it. Implementing the standard
gets "Show in Folder" working across the desktop with no per-application work,
and costs three methods.

**The thumbnail cache is the freedesktop one, and its API is a library.** A
private cache would be a second copy of every thumbnail on the system and would
interoperate with nothing. A thumbnail *daemon* with a D-Bus request/ready
protocol would add a process, a protocol, a lifecycle and a failure mode, to
solve a coordination problem the filesystem already solves: atomic rename makes
concurrent generation safe, and the mtime check makes it idempotent. Two
processes occasionally decoding the same JPEG is a cheaper bug than a daemon.

**File-type detection is one implementation in otto-kit, with two calls that
answer two questions.** A single "what type is this file" call has to pick
between name and content, and either choice is wrong somewhere: content-wins
makes an empty source file's icon flip, name-wins hands a mislabelled file to
the wrong decoder. Splitting it means display is stable and decoding is safe,
and the two can never *disagree* because they were never asked the same thing.
It lives in otto-kit rather than in the file manager so that the previewer, the
picker and the compositor's own icon lookup can all call it without depending on
a file manager.

**The full shared MIME database, not a table of common types.** `globs2` and
`subclasses` are line-oriented text — no XML parser, no `mime.cache` — so the
complete database costs a file read and a hash map. A hardcoded table cannot
serve a portal filter naming
`application/vnd.oasis.opendocument.text`, and cannot fill a Kind column with
types nobody enumerated in advance.

**Return renames and never opens.** Otto follows macOS here rather than the
Linux Enter-opens convention, for the same reason the browser refuses to run
executables on double-click: a key that opens whatever is selected is one
mistake away from launching a screenful of files. Renaming is recoverable —
Escape cancels, and the field starts with the extension unselected — while
opening is not. F2 is accepted as an alias so the Linux habit still lands on
the same action, and Right arrow (or a double-click) opens.

**Executables are not run on double-click.** "Double-click ran the file" is a
malware delivery mechanism, and the convenience it buys is a keystroke in a
terminal.

**Trash is home-filesystem-only in v1, and says so.** The alternative that looks
like it works — copy the file to the home trash — moves gigabytes across a bus
to "delete" something and breaks restore. The alternative that is correct
requires `.Trash-$uid` handling with its own security rules. Refusing clearly is
better than either, and it is a small, self-contained thing to add later.

**Drag and drop moves by default and copies when asked.** A drag between two
directories on one filesystem is a move — what every other file manager does —
and the browser says so by asking the compositor for `move`, preferring it out
of `copy | move`. It is a *preference*, not a decision: the compositor picks
the action from what both sides offer, and a source that only offers a copy
gets a copy. The negotiated action is read at the drop, never assumed from what
was asked for, because assuming it is how files get moved that were meant to be
copied.

## Out of scope for v1, explicitly

Tabs. Split views. Network and virtual filesystems. Mounting and ejecting.
Content search and any index. Batch rename. Archive browsing or extraction.
File comparison. Tags, labels, colours, or any metadata Otto would have to store
itself. Custom per-directory view settings beyond sort order. Templates. Running
external thumbnailers. Persisted column widths.

## Resolved decisions

### Negotiated with quick view

With [quickview.md](./quickview.md), recorded so they are not reopened:

- The thumbnail cache is a **filesystem layout, not a service**. No daemon, no
  D-Bus request/ready protocol, no coordinator. Cross-process correctness comes
  from the standard key, atomic rename, and the `Thumb::MTime` check.
- Quick view is a **producer as well as a consumer** of the cache. It stores the
  scaled decode it was making anyway, at the standard buckets only.
- The cache is **small images only**. Full-resolution decoding is each
  consumer's own business; the cache never serves it and never brokers it.
- File-type detection lives in **otto-kit**, not otto-files, and splits into
  name-based (display, filters, associations) and content-based (decoder
  dispatch). Content never overrides the name for display.
- The MIME source is the **full shared database via `globs2`/`subclasses`**, not
  a hardcoded table. They are plain text; no XML parser is needed.
- Quick view is **embedded as a library**, and the browser keeps the keyboard,
  the pointer, and the panel's place in its own window. The earlier decision —
  that quick view owned a D-Bus invocation interface and held the keyboard —
  was reversed when it became clear that parenting the preview to the file view
  and running it as a separate process are mutually exclusive.
- The **decode worker stays a separate process**, and is the host binary
  re-executed. The sandbox is about untrusted bytes, not about where the UI
  lives; embedding the panel does not put a parser in the browser.

### Settled defaults

Questions this spec left open in an earlier draft, resolved with the obvious
answer rather than carried:

- **Per-directory sort order and view mode live in one central bounded file**,
  `$XDG_STATE_HOME/otto/file-browser-views`, keyed by absolute path, a
  least-recently-used cap of 512 entries. **Otto never writes a dotfile into a
  user's directory** to record its own view state: it pollutes directories the
  user did not ask us to write to, it travels with the files into archives and
  version control, and it is visible in every other file manager. A bounded
  central file is forgettable in the way this state deserves to be — losing an
  old entry costs a sort order.
- **`mimeapps.list` is written by the browser directly**, not routed through the
  settings service. It is a freedesktop file with a defined format and other
  writers on the system; the settings schema is for Otto's own configuration
  keys, and putting a shared standard file behind `Set(id, value)` would give it
  an owner it cannot actually have. This is the same single-writer reasoning as
  [settings-app.md](./settings-app.md), applied honestly: the owner of
  `mimeapps.list` is the standard, not Otto.
- **Recursive directory size streams a running total and is not cached.** A
  cache would need invalidating on any change anywhere beneath the directory,
  which is the genuinely hard problem, in exchange for a number the user asks
  for rarely and watches converge in seconds. The count is cancelled when the
  panel closes.
- **`ShowItems` on a directory URI selects it in its parent**, and does not open
  it. The method's contract is to show items; `ShowFolders` is the one that
  opens. A caller that wanted the directory opened had a method for that.

## Open Questions

None outstanding.
