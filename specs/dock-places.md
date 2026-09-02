# Dock Places

**Status:** draft
**Related specs:** [dock-icon-reorder.md](./dock-icon-reorder.md), [file-browser.md](./file-browser.md), [context-menus.md](./context-menus.md)

## Summary

The dock's second strip, past the divider: the things that are *locations*
rather than applications. The Trash is the one that ships. It is in the dock
because it is a place a file can be in, not because it is an app somebody
launched, and the strip it lives in says so.

## Goals

- A places strip, drawn past the dock's divider, before the minimized windows.
- The Trash in it by default, with an icon that says whether the can is empty.
- That icon is right whether or not the Trash window is open.
- A right-click menu that offers what the place itself can do — Empty Trash —
  and not the things that only make sense for an app.

## Non-Goals

- Folders and stacks. The strip is built for them; nothing else is in it yet.
- Dragging a folder into the dock to make a place. Places are configuration
  for now.
- Reordering places by dragging. There is one.

## Behavior

**A place is a desktop entry.** It is configured exactly like a dock bookmark
is — `[dock] places` is a list of desktop ids, or tables carrying a label —
and it is drawn with the same slot, icon, label and running dot an application
gets. What differs is which strip it is in, and the menu it opens.

**The strip.** Places are drawn between the dock's divider and the minimized
windows. An empty places list takes no room at all: no stub, no gap. The
strip's slots magnify under the pointer along with every other icon in the
dock, in one continuous sweep across the apps, the places and the minimized
windows.

**The Trash is a place by default.** A configuration that has never mentioned
places gets one holding the Trash. A user who removes it does not get it back.

**Its icon follows the can, not the window.** When the trash holds anything,
the icon is the icon theme's full wastebasket; when it does not, the empty
one. This is true while the Trash window is open, while it is closed, and
whether the change came from Otto or from anything else on the system: the
state is read once at startup and then whenever the trash directory changes.
It is never polled. An icon theme that has no full wastebasket leaves the
desktop entry's own icon in place.

**Clicking it** opens the Trash window, or focuses it when it is already open —
the same thing clicking a launcher does.

**The wastebasket is named, not hardcoded.** `[dock] trash_desktop_id` says
which place's icon follows the can; it defaults to Otto's own Trash entry.
Everything else about that place is its desktop entry's: the command a click
runs is that entry's `Exec`, and the menu is that entry's `Actions=`. Pointing
the setting — and the `places` list — at another file manager's entry is the
whole of using that one instead, and nothing in the dock knows the difference.
`[dock] trash_path` says which directory the full/empty icon watches,
defaulting to the freedesktop location; it exists for the file manager that
keeps its trash somewhere else, and it never changes where Otto itself puts
what it throws away.

**Right-clicking it** opens a menu of:

- **Open**, when the window is not already open.
- **Empty Trash** — the desktop entry's own action. For Otto's entry it opens
  the Trash window with the Empty Trash question already asked, so the
  question, the count in it and the operation behind it are the window's and
  cannot drift from its own button. An entry that declares different actions,
  or none, offers those instead: the menu is the entry's, not the dock's.
- **Quit**, when the window is open.

It does not offer *Keep in Dock* or *Remove from Dock*: a place is in the dock
because it is a place.

**Every app's own actions are in its menu.** The entries a desktop entry
declares in `Actions=` are offered on that icon, above the dock's own entries,
for places and applications alike. Selecting one runs that action group's
command. This is how the Trash's Empty Trash gets there — it is data in
`otto-trash.desktop`, not a special case in the dock.

## Constraints & Edge Cases

- **The trash directory need not exist.** A session that has never thrown
  anything away has no `Trash/files`; that is an empty trash, not an error, and
  the watch sits on the nearest ancestor that does exist so the directory's
  creation is itself an event.
- **A burst is one change.** Emptying the trash deletes many files at once; the
  icon is looked at once per burst, not once per file.
- **A running place is not also an app.** With the Trash window open, the dock
  shows one wastebasket — in the places strip, with a running dot — never a
  second icon appended to the applications.
- **A place does not reorder.** Dragging one does not start a reorder: the
  launcher order it would be counted against is not the strip it is in.
- **A missing desktop entry** is a warning in the log and a place that is not
  drawn, exactly as a missing bookmark is.
- **The strips move as one.** While the dock is being resized, the places strip
  stays on the applications' line frame by frame, not only once the animation
  settles. Anything that lets one strip's thickness settle independently of the
  other's shows up as the icons bobbing.
- **The divider keeps up with the icons.** It is laid out between the strips,
  so it moves every time an icon beside it grows; it has to be moved frame by
  frame with them, not left to the next full render, or it stands still while
  the strips slide past and overlaps the places icons.
- **With the pointer off the dock nothing is magnified.** The pointer's
  position is mapped onto the icons with the gaps between the strips skipped;
  before the first icon and past the last one the distance keeps growing rather
  than being clamped to the nearest strip, or the icon at that end would sit
  magnified whenever the pointer was elsewhere — including at startup, before
  it has ever been over the dock.

## Rationale

**Why the compositor watches the trash rather than the app telling it.** The
dock icon is on screen almost always and the Trash window almost never. An icon
driven by the app would be honest only while the app was running, which is the
wrong way round. It also means no protocol is needed for what is, from the
dock's side, a question about a directory.

**Why the menu comes from the desktop entry.** `Actions=` is the standard way
an application declares what its icon can do, it works when the application is
not running, and every other app gets its own quicklist from the same code. A
dynamic menu over the dock protocol would only work while a window was open,
which is the same trap as the icon.

**Why places are not just bookmarks in the apps strip.** A bookmark is an app
the user chose to keep. The Trash is not an app the user chose; it is where
deleted files are. Putting it past the divider is the dock saying which of the
two it is.

## Open Questions

- Folders as places: what a folder's icon shows, and what its menu offers.
- Localising a desktop action's name. `Actions=` names are localised in the
  desktop file (`Name[it]=`), not in Otto's own catalogues.
