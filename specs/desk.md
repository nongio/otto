# Desk

**Status:** partly implemented — `otto-files --desk` shows one surface on the
output the compositor picks, grid arrangement only, with the `[desk]` config
read at startup and the single-instance lock. Not yet: a surface per output,
free placement and its state file, Arrange and Clean Up in the menu, config
live reload, watching for a missing folder to appear, and Peek above the
windows (it is still a subsurface of the desk, so windows cover it).
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
  of each output, with padding.
- Offer two arrangements: a sorted grid, or free placement where icons stay
  where they are dropped.

## Non-Goals

- A background of its own. The panel is always transparent. Legibility over a
  busy wallpaper is the person's call.
- Resizing or moving the panel with the pointer. It has no chrome and no grips.
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
- **Clean Up** in the background menu snaps free-placed icons back into sort
  order. **Arrange ▸ Grid / Free** switches arrangement from the menu and
  writes it back to the config. Switching to grid keeps the free positions
  for when free placement comes back.

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
padding = 24            # points, inside the panel
icon_size = 64          # points
```

- Every key is optional. A file that cannot be read or parsed is a warning in
  the log, and the defaults stand.
- The config is read at startup and re-read when the file changes, so edits
  apply without restarting.
- A folder that does not exist shows an empty desk and is watched for being
  created. The desk never creates it.

### Lifecycle

- The desk is `otto-files --desk`, a separate process from any browser window.
- Whether it runs is a session setting, **Show files on the desktop**, in
  Settings ▸ General (key `desk.enabled`, off by default). The setting applies
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

- **A shell over the browser's view layer, not a new app.** The value is that
  a file on the desk behaves exactly like a file in the browser. Two copies
  of the icon view would drift.
- **Transparent, config-sized, no chrome.** The desk is part of the desktop,
  not a window. With nothing to grab, size and position belong in config.
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
