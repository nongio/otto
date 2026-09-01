# Files

`otto-files` is Otto's file manager — browse the filesystem, open things, move
them around, and preview a file without opening it at all.

> **First version.** Files is new and aims to be genuinely useful for everyday
> browsing, but it is not finished. What is missing is listed at the bottom of
> this page rather than left for you to discover.

## Opening it

Launch **Files** from the Dock or the launcher, or run `otto-files`. Other
applications can open it too: "Open Containing Folder" in a browser or editor
goes through the standard `org.freedesktop.FileManager1` interface and lands
here.

`Ctrl+N` opens a new window at your home directory. To open a specific folder
in a new window, hold `Ctrl` and double-click it.

## The three views

| View | Key | What it is for |
|------|-----|----------------|
| List | `Ctrl+1` | One row per entry, with size and date |
| Icon | `Ctrl+2` | A grid of large icons and thumbnails |
| Column | `Ctrl+3` | Miller columns — each folder opens a pane to the right, with a preview column at the end |

Pictures, PDFs and videos show a thumbnail instead of a generic type icon.
Files reads the shared thumbnail cache that other file managers write, so
folders another manager has already been through come up with pictures
immediately. Files does not write to that cache yet, so thumbnails it makes for
itself are not reused elsewhere.

## Getting around

- **Double-click** or `Ctrl+O` opens the entry at the cursor — descend into a
  folder, or open a file in its default application.
- **Backspace** goes up a directory, as do `Alt+Up` and the back arrow's
  neighbour in the header. The arrow keys move the cursor; `Right` descends.
- **`Alt+Left` and `Alt+Right`** step back and forward through where you have
  been, the same as the arrows in the header. `Alt+Home` goes home.
- **`Ctrl+L` types a path.** The title turns into a field holding the folder
  you are in; type or paste a path and press `Return` to go there. `Tab`
  completes against the folder being typed — to the one match, or as far as
  every match agrees — and adds the `/` for you, so you can walk down a tree
  without reaching for it. `~` means your home directory, and a bare name is
  taken relative to the folder on screen. A path to a *file* opens the folder
  holding it with the file selected, which is what pasting one out of a
  terminal usually means. `Escape` puts the title back.
- **Type a few letters** to jump to the first entry whose name starts with
  them. This selects, it does not filter — the whole folder stays on screen.
  The typed text expires after about a second, and repeating one letter cycles
  through the entries beginning with it.
- **The sidebar** holds your home, desktop, documents, downloads, music,
  pictures and videos.

## Selecting

Click selects, `Ctrl+click` adds, `Shift+click` extends. In icon view, dragging
from empty space sweeps a rubber band over the grid and takes everything it
touches; hold `Ctrl` or `Shift` while you drag and it adds to what was already
selected. Clicking empty space selects nothing.

## Working with files

| Key | Action |
|-----|--------|
| `Return` or `F2` | Rename, inline, with the extension left out of the selection |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Delete`, `Ctrl+Delete`, `Ctrl+Backspace` | Move to trash |
| `Ctrl+Z` | Undo the last operation |
| `Ctrl+I` | Show info for the selection |
| `Space` | Quick view (see below) |
| `Escape` | Cancel a rename, else clear the selection |

Copy, move and trash run in the background, with progress and a cancel that
leaves nothing half-written: every copy is written under a temporary name and
renamed into place only when it is complete. When a destination file already
exists you get Replace, Skip or Keep Both, with "apply to all remaining".
Folders merge rather than being replaced.

Undo goes 32 operations deep for the current session and covers moves, copies,
renames, new folders and trashing. Permanent deletion is not undoable, and the
undo entry says so rather than quietly disappearing.

**Drag and drop** works within a window, between windows, and in and out of
other applications — dragging a picture into a browser upload field or a chat
window does what you expect.

## Quick view

Select something and press `Space`. A panel grows out of the row showing the
file itself: pictures, text and code, PDFs, a listing for a folder, and — for
audio and video — the tags, dimensions and duration read from the file's header
rather than the whole file. There is no playback yet.
Arrow keys move to the next file and the preview follows, `Space` closes it,
and `Escape` closes it before it clears your selection.

While the panel is up it owns the pointer: the wheel scrolls a text preview, a
pinch zooms a picture, and a two-finger scroll pans a zoomed one with momentum
and springy ends. Files never decodes a file itself — the bytes are parsed in a
separate sandboxed process, and only a validated result is drawn.

## Opening and saving in other applications

The same code is the desktop's file picker, through the XDG Desktop Portal. When
Firefox or Chrome asks you to pick a file to upload, or to save a page, you get
this window rather than a GTK dialog. Save mode gives you a name field with the
proposed name's stem preselected, refuses a name that is not a single file name,
and asks before replacing an existing file. The picker never creates the file —
the application does that once you accept.

## Trash

Move to trash follows the freedesktop trash specification: the file goes to
`~/.local/share/Trash/`, with a record of where it came from and when. Files on
another filesystem than your home directory cannot be trashed yet — Files says
so and offers permanent deletion instead, rather than silently copying the file
across.

## Not there yet

- Tabs, split views, and persisted column widths.
- Network and virtual filesystems — `smb://`, `sftp://`, MTP. Local paths only.
- Mounting, unmounting and ejecting devices. Volumes that are already mounted
  do appear.
- Searching file contents, batch rename, archive browsing, tags and labels.
- `Shift+Delete` (delete permanently) is deliberately inert for now.
- Writing to the shared thumbnail cache.
