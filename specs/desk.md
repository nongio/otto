# Desk

**Status:** partly implemented — `otto-files --desk` shows one surface on the
output the compositor picks, grid arrangement only, with the `[desk]` config
read at startup and followed live, the single-instance lock, edit mode
for the panel's size and position, and `overflow = "stack"` with its
overflow tile and overflow panel. Not yet: a surface per output, free
placement and its state file, Arrange and Clean Up in the menu, watching for
a missing folder to appear, and Peek above the windows (it is still a
subsurface of the desk, so windows cover it).
**Related specs:** [settings-app.md](./settings-app.md), [file-browser.md](./file-browser.md), [peek.md](./peek.md),
[context-menus.md](./context-menus.md), [multi-output.md](./multi-output.md),
[usable-area-refit.md](./usable-area-refit.md)

## Summary

The desk is a folder shown on the desktop itself: a transparent panel that
sits above the wallpaper and below every window, listing the files in the
Desktop folder. It is another shell over the file browser's view layer, so the
icons, thumbnails, selection, right-click menu, drag and drop and Peek are the
browser's own.

## Goals

- Show the contents of one folder on every output, between the wallpaper and
  the windows. The default folder is `XDG_DESKTOP_DIR`.
- Look and behave like the browser's icon view: same icons, thumbnails, labels,
  selection highlight, right-click menu, drag and drop, rename and Peek.
- Work from the keyboard once focused: arrows, type-ahead, Enter to open,
  Space for Peek, the browser's file shortcuts.
- Select with the pointer, one item at a time or several with a rubber band.
- Size and position come from configuration. The default fills the usable area
  of each output, with padding. Edit mode, started from Settings, sets them
  with the pointer.
- Offer two arrangements: a sorted grid, or free placement where icons stay
  where they are dropped.

## Non-Goals

- A background of its own. The panel is always transparent. Legibility over a
  busy wallpaper is the person's call.
- Resizing or moving the panel with the pointer in normal use. It has no
  chrome and no grips; the pointer moves and resizes it only in edit mode,
  which is entered from Settings.
- The command palette, the sidebar, the path bar, the header and navigation.
  The desk shows one folder and never navigates away from it. Opening a folder
  opens it in the browser.
- Special compositor handling. The desk is a plain layer-shell client and gets
  exactly what any other bottom-layer surface gets.
- Different folders per output.

## Behavior

### Placement

- The desk is a `zwlr_layer_shell_v1` surface in the `bottom` layer, with
  namespace `otto-desk` and an exclusive zone of 0. It never reserves space.
- One surface per output. A surface appears when an output is added and goes
  away when it is removed.
- It covers the output's usable area: the compositor already leaves out the
  zones the dock, the bar and any other exclusive surface reserve. Padding is
  applied inside the surface, so icons never touch the edges or the zones.
- `anchor = "fill"` (the default) stretches the panel over the usable area.
  Any other anchor places a panel of `size` at that edge, corner or the centre.
  `size` is in logical points or as a percentage of the usable area. It is
  clamped to the usable area.
- `position`, when set, places the panel's top-left corner instead of the
  anchor's edge or corner, in points or as a percentage of the usable area.
  A panel that would hang off the area is moved back inside it. `fill`
  ignores it. Edit mode writes it.
- The surface itself always covers the whole usable area, whatever the
  anchor: the panel is a rect inside it, worked out from the size the
  compositor configures, and only the panel takes pointer input. That keeps
  percentages and the clamp exact without the desk having to know the
  reserved zones.
- Every workspace shows the same desk, since it sits below all of them.

### Input

- The panel is transparent, and pointer input on it is limited to where it
  draws: the icons and, while one is in progress, a rubber band. The rest of
  the panel still takes presses so a rubber band can begin there, and a press
  on empty space clears the selection.
- Keyboard interactivity is on demand. A click on the desk gives it keyboard
  focus. A click on a window moves focus there. Show Desktop does not focus
  the desk.
- With focus, the browser's keys apply: arrows move the selection through the
  grid, typing selects by name, Enter opens, Space toggles Peek, F2 renames,
  Delete moves to the Trash, Ctrl+A/C/X/V/Z work as in the browser. Escape
  clears the selection.
- A double click opens an item. A folder opens in the browser, a file in its
  default application.
- Right click shows the browser's context menu for the item, or the
  background menu for empty space (New Folder, Paste, Arrange, Clean Up, Open
  in Files).

### Edit mode

- Settings ▸ Appearance ▸ Desk ▸ *Size and position* ▸ **Edit…** calls
  `EditLayout` on `org.otto.Desk1` (object `/org/otto/Desk1`), which the
  desk serves while it runs. The call returns at once. The button is
  inactive while the desk is off.
- In edit mode the desk moves to the `top` layer, above the windows, with
  exclusive keyboard interactivity, and takes presses on its whole surface.
  Nothing outside the panel is dimmed; the panel keeps showing its icons.
- The panel has a thin rounded outline in the accent colour. Each corner has
  a white bracket that follows the rounded corner, and the middle of each
  side a white pill lying along it; both carry a soft shadow so they read on
  any wallpaper. The cursor shows what a press would do: a resize arrow on
  an edge or corner, an open hand inside the panel (closed while dragging),
  a pointer over the buttons. A drag on a handle or within a
  few points of an edge resizes from that edge or corner, the opposite edges
  staying put. A drag inside the panel moves it. The panel stays inside the
  usable area and never gets smaller than 180 points either way, which is
  room for an icon and for the two buttons. The grid reflows as it goes.
- **Cancel** and **Done** sit at the bottom of the panel. Cancel puts the
  panel back where it was. Done, Return or Escape keep the new geometry and
  write it to `files.toml`: `anchor = "top-left"` with `size` and `position`
  as percentages of the usable area, to one decimal, so the panel keeps its
  place when the output or a reserved zone changes. A panel that covers the
  whole area is written as `anchor = "fill"`, dropping `size` and
  `position`.
- Leaving edit mode puts the desk back in the `bottom` layer with on-demand
  keyboard interactivity and its input region back to the panel.
- **Reset** beside Edit… writes `anchor = "fill"` and removes `size` and
  `position`. The desk follows it like any other change to the file.

### Drag and drop

- Items drag out of the desk like out of the browser: onto a window, onto
  the dock (including the Trash), onto a browser window.
- Items dropped on the desk from elsewhere move into the folder, or copy if
  they come from another filesystem, following the browser's rules.
- In free placement, dragging an item within the desk moves its icon and
  leaves the file alone.
- Dropping onto a folder icon on the desk moves the items into that folder.

### Arrangement

- **Grid** (default): icons fill the panel in rows, left to right, then top
  to bottom, starting from the top-left corner, in `sort` order (`name`, `kind`, `modified`). New files take their sorted
  place.
- **Free**: each icon has a slot on an invisible grid of cell-sized slots. A
  dropped icon snaps to the nearest free slot, so icons never overlap. New
  files take the first free slot in grid order. When a file goes away, its
  slot is freed. Positions are kept per output and file name in
  `~/.local/state/otto/desk.toml`, never in the folder itself.
- A slot that no longer fits (the panel shrank, or the output changed size)
  reflows its icon to the first free slot, without forgetting the saved one.
  Under `overflow = "stack"`, an icon with no free slot left goes into the
  overflow tile, which takes the last free slot in grid order.
- **Clean Up** in the background menu snaps free-placed icons back into sort
  order. **Arrange ▸ Grid / Free** switches arrangement from the menu and
  writes it back to the config. Switching to grid keeps the free positions
  for when free placement comes back.

### Icons that don't fit

`overflow` decides what happens when the folder holds more items than the
panel's grid has whole cells. A cell counts when it fits entirely: every
column, times every row whose cells end inside the padded panel.

- **`scroll`** (default): the grid runs on past the bottom of the panel and
  scrolls, like the browser's icon view.
- **`stack`**: the grid never scrolls. It fills in sort order, and once
  there are more items than cells, the last cell in fill order becomes the
  **overflow tile**, holding that cell's own item and every item after it.
  With room for 12 cells and 20 items, cells 1 to 11 show items 1 to 11 and
  the tile holds items 12 to 20. The config value stays `stack`.
  - The tile draws its first three items' icons on top of each other, the
    first on top, each one below it a few points up and to the right and
    turned a few degrees, alternating sides. A pill in the accent colour
    over the icon's top trailing corner shows how many items it holds
    (999+ past that), the number centred on its own ink. Its caption is
    *More items*. It lights like a selected icon while the panel is open
    and while any of its items is selected. The tile stays in its cell
    while the panel is open.
  - A press on the tile, or Return or Space with the keyboard on it, opens
    the **overflow panel**: the tile's items as a small grid of the desk's
    own cells, at most six across and two rows tall (captions included),
    scrolling for more with the same momentum and elastic ends as every
    other list. It is a surface of its own above the windows, not part of
    the desk's surface: a transparent layer-shell overlay on the top layer,
    the same size as the desk's surface and at the same place, carrying the
    panel as a card. The compositor blurs and tints what is behind the card,
    rounds it and casts its shadow; its ground is dark so the desk's white
    captions read on it. It sits above the tile when there is room, below
    it otherwise, and centred on it, kept inside the desk's surface.
  - The panel grows out of the tile's icon and fades in (about 240 ms), and
    shrinks back into it and fades out when it closes (about 190 ms). The
    animation runs in the desk's process a frame at a time: each frame the
    card is moved and sized and painted again, and a paint waits for the
    compositor to have shown the last one. A panel closed part way in goes
    back from where it got to.
  - Inside it the items behave like any icon on the desk: a click selects,
    a double click opens, Ctrl and Shift extend, F2 renames, Space peeks,
    and the right-click menu is the item's (hung off the overlay, so it is
    above the panel too). A press on its ground between items clears the
    selection. A drag carries them out: the panel goes back into its tile
    as the drag starts, and its surface stops taking the pointer at once,
    so the drop lands on the desk or the window under it.
  - Escape, a press anywhere outside the panel (the overlay takes it, so it
    goes no further — not to the window under it either), the keyboard
    going to another application's window, switching `overflow` back to
    `scroll`, changing `icon_size`, entering edit mode, or the tile going
    away closes it. A rename under way in the panel is committed as it
    closes.
  - Clicking the panel gives its overlay the keyboard, on demand; a key
    typed with the desk focused reaches the panel all the same.
  - The tile is not an item of its own. A right click on it is the desk's
    background menu, and a drop on it lands on the desk, as a drop on empty
    space does; only a folder icon takes a drop into itself. The rubber
    band stops before the tile. Select All still selects the tile's items.
  - The keyboard walks the cells up to the tile and stops on it; with the
    panel open, the arrows walk the panel's items instead (Up and Down by
    one of its rows), scrolling it to keep the item in view, and Escape
    puts the keyboard back on the tile.
  - A screen reader reads the closed tile as one list item, labelled with
    how many items it holds ("12 more items"); activating it opens the
    panel, whose items are then read like any other icon.

### Configuration

In `~/.config/otto/files.toml`, beside the sidebar's settings:

```toml
[desk]
folder = "~/Desktop"    # default: XDG_DESKTOP_DIR, then ~/Desktop
arrange = "grid"        # grid | free
sort = "name"           # name | kind | modified
anchor = "fill"         # fill | top-left | top | top-right | left | center
                        # | right | bottom-left | bottom | bottom-right
size = [800, 600]       # ignored with fill; points, or strings like "60%"
position = ["10%", 40]  # optional top-left corner; ignored with fill
padding = 24            # points, inside the panel
icon_size = 64          # points
overflow = "scroll"     # scroll | stack
```

- Every key is optional. A file that cannot be read or parsed is a warning in
  the log, and the defaults stand.
- The config is read at startup and re-read when the file changes (the desk
  watches the folder it lives in), so edits apply without restarting: a new
  folder is listed at once, and the order, icon size, padding, anchor, size,
  position and overflow take effect on the next frame.
- Settings ▸ Appearance ▸ Desk writes `folder` (through the desktop portal's
  folder picker, under the home folder as `~/…`), `overflow` (*When icons
  don't fit*), `icon_size` (*Icon size*, a slider from 32 to 160 points in
  steps of 4, written once per step while dragging) and, through Reset, the geometry. It edits the file in place and keeps everything else in it.
- A folder that does not exist shows an empty desk and is watched for being
  created. The desk never creates it.

### Lifecycle

- The desk is `otto-files --desk`, a separate process from any browser window.
- Whether it runs is a session setting, **Show files on the desktop**, in
  Settings ▸ Appearance ▸ Desk (key `desk.enabled`, off by default). The setting applies
  live: switching it on starts the desk, switching it off stops it. At login
  the compositor starts the desk when the setting is on.
- The compositor owns the process. If the desk exits unexpectedly while the
  setting is on, it is restarted, with a back-off so a crash loop cannot
  spin.
- Only one desk runs per session. A second `otto-files --desk` exits at once.
- The folder is watched like a browser listing: files that appear, disappear
  or change update in place.

## Constraints & Edge Cases

- Thumbnails come from the shared thumbnail cache, so the desk and the browser
  never make the same thumbnail twice.
- A desk with thousands of files must stay responsive: it is a listing like
  any other, and the browser's async loading applies.
- Hidden files follow the browser's default and are not shown.
- The desk must not keep the GPU busy while idle. No frames are drawn unless
  something changed.
- Fullscreen and maximized windows cover the desk on their output. It keeps
  running and needs no special case.
- Peek opens as its own surface above the windows, like it does from the
  browser.

## Rationale

- **An overflow tile rather than hiding what does not fit.** A desk that
  stacks is a surface that never moves under the pointer, and a file must
  never seem to vanish: the tile says how many more there are and opens
  next to itself, so every file stays one click from where it was. The
  panel reuses the desk's own cells, so a file in it is still the same icon
  with the same gestures.
- **The panel on an overlay above the windows.** Drawn into the desk's own
  surface it was covered by every window and clipped to the desk. An
  `xdg_popup` of the desk does not work either: the compositor hit-tests a
  bottom-layer surface's popups only through the desk's input region and
  only after every window, and a grabbed popup keeps the pointer grab,
  which refuses the drag that carries an item out. An overlay the size of
  the desk's surface shares its coordinates, so every hit test and rect
  stays in the desk's space, and the card on it gets the compositor's blur
  the way the palette's and Peek's cards do.

- **A shell over the browser's view layer, not a new app.** The value is that
  a file on the desk behaves exactly like a file in the browser. Two copies
  of the icon view would drift.
- **Transparent, config-sized, no chrome.** The desk is part of the desktop,
  not a window. With nothing to grab, size and position belong in config.
  Edit mode is how that config is set by hand without writing numbers: it
  is entered on purpose, from Settings, so the desk never grows grips a
  stray drag could catch in normal use.
- **Defaults are what a plain layer surface gets.** Respecting reserved zones,
  focus on click and one surface per output all fall out of the layer-shell
  protocol. The compositor stays out of it.
- **Every output shows the whole folder.** A file must never seem to vanish
  because it sits "on the other screen". Free positions are per output
  because outputs differ in size.
- **Enabling is a session setting, the look is the desk's config.** Whether
  a desktop shows files is a choice about the session, so it lives in
  Settings next to the other desktop choices and applies live. Folder,
  arrangement and geometry belong to the desk, beside the browser's own
  settings in `files.toml`.
- **Positions in Otto's state directory, not in the folder.** Nothing is
  written into the person's Desktop folder that they did not put there.

## Open Questions

- Grid origin: top-left, or top-right so the desk stays clear of where windows
  usually open? Top-left until decided.
- Per-output overrides of `anchor`, `size` and `padding`.
