# Accessibility bugs

Gaps found while testing Otto's AT-SPI support against a live session on
2026-08-29, with `pyatspi` probes rather than a screen reader — see
`docs/developer/accessibility.md` for how to run them. Ordered by how much they
cost a user who depends on this.

Two bugs found in the same pass are already fixed and are recorded here only so
the list reads as a whole: the Displays pane's unbound rows being unreachable
from the keyboard, and the shell describing a freshly started application as
not running for as long as nothing else changed.

## Bug: kit applications report window-local coordinates as desktop coordinates

Every node an otto-kit application publishes carries bounds in **window-local**
coordinates, while AT-SPI declares them `DESKTOP_COORDS`. Their frame extents
come back as `(-1,-1 -1x-1)`, because nothing ever calls
`Adapter::set_root_window_bounds`. The compositor's own chrome is correct — it
does call it.

Anything that maps a screen position to a widget therefore lands somewhere
else, often in a different application: mouse review, magnifier tracking and
braille routing all do this.

Measured on the live session: otto-bar's status buttons reported x=10..222,
although the status area sits at the right of a 1645px screen; otto-settings'
sidebar claimed desktop (0,0 214x640) — the dock's rectangle — so
`getAccessibleAtPoint` over a dock icon answered with Settings' sidebar.

The awkward part is that a Wayland client genuinely cannot know where its
window is, and no Otto protocol carries it. The compositor has to say.

- [x] Decided: a `desktop_frame` event on `otto_surface_style_v1`, which is
      already the per-surface channel the compositor answers `output_frame`
      on. Version 4.
- [x] Compositor: `src/surface_style/desktop_frame.rs` sweeps the style
      surfaces once per pass of the event loop and sends the ones whose rect
      has changed. Taken from the scene layer the surface is drawn into, so
      it cannot disagree with what is on screen, and diffed rather than
      hooked into the dozen places that move a window
- [x] otto-kit: the frame is filed against the window's `wl_surface` and fed
      to `Adapter::set_root_window_bounds` before each tree is published
- [x] Only root surfaces are told. A popup or a subsurface moves with its
      parent and describes itself against the parent's origin, so telling it
      where it is would only offer a second, wrong origin to use
- [x] Verified in a nested Otto: otto-settings reports its window at
      (504,298) 900x640 instead of (-1,-1), `getAccessibleAtPoint` over a
      sidebar row answers with that row, and a point outside the window
      answers with nothing — which is the bug, since the sidebar used to
      claim the dock's rectangle. Maximising moves the reported rect to the
      usable area and restoring puts it back, so it follows the window
- [ ] Re-run the sweep on a real session across dock, bar and an application
      window once the compositor on this branch is installed

## Bug: a pop-up can be focused from the keyboard but not opened

`Control::Select` rows — the Displays pane's resolution and refresh, the
General pane's themes — take the focus and are described as combo boxes, but
Space does nothing: `DropdownMenu` has no keyboard handling at all, so opening
the menu would leave the user inside something they could neither navigate nor
dismiss.

A screen reader is told the current value, which is not nothing, but the
control cannot be changed without a pointer.

- [ ] `DropdownMenu`: arrows move the highlight, Enter picks, Escape closes,
      Home/End go to the ends
- [ ] Describe the open menu — the items as a list box, the highlighted one
      as the focus — so it is not a hole in the tree while it is up
- [ ] otto-settings: `activate_focused` opens the menu for a `Select` row,
      anchored the way `select_hit` anchors it, using the key event's serial
- [ ] Return the focus to the row when the menu closes, whichever way it did

## Bug: the Displays arrangement canvas is not described

The strip of screen rectangles at the top of the Displays pane is drawn by the
pane rather than built as rows, so it is neither a keyboard stop nor in the
accessible tree. Choosing *which* display the rows below apply to is therefore
pointer-only, which makes the rest of the pane's keyboard reach much less
useful than it looks on a multi-display desktop.

- [ ] Describe the canvas as a list of screens, each named as the canvas
      labels it, the selected one selected, with its own bounds
- [ ] A stop per screen, or one stop for the strip walked with the arrows —
      the latter matches the sidebar and is probably right
- [ ] `Action::Click` on a screen selects it, the same path
      `screen_hit` takes
- [ ] Dragging a screen to rearrange it stays pointer-only for now; say so
      in the spec rather than leaving it implied

## Bug: otto-files describes its listing but not its chrome

The file list and the previews are described well — kind and size per row, text
previews as readable runs. Nothing else in the window is: the Favourites
sidebar, the path bar, the toolbar and the view-mode switcher are all absent
from the tree, so a screen reader user can read a folder but cannot navigate to
another one, or tell which folder they are in beyond the window title.

- [ ] The sidebar as a list, its sections as groups, the current place
      selected, clicking one navigating there
- [ ] The path bar as a breadcrumb, each component clickable
- [ ] The toolbar buttons — back, forward, view mode, sort — each named
- [ ] Say which view mode is current, since the same listing reads
      differently in list, column and grid

## Bug: the shell's containers carry no bounds

In the shell tree the dock's toolbar node, the workspace list box and every
workspace item come back with no extents; only the dock's own buttons carry
them. Mouse review over the workspace strip finds nothing, and an AT that asks
where the dock *is* — to draw a focus highlight around it, for instance — gets
no answer.

- [ ] Give the dock container the dock's own rect
- [ ] Give the workspace list and each workspace item their rects from the
      workspace selector's layout
- [ ] Give the switcher and the overview list their rects too
- [ ] Extend the existing bounds test so a container without bounds fails

## Bug: at-spi2-core rejects AccessKit's cache items

Every time an AccessKit tree changes, at-spi2-core 2.60.6 logs, once per node:

```
AT-SPI: AddAccessible with unknown signature (so)(so)(so)iiassusau
AT-SPI: Unknown signature so for RemoveAccessible
```

The listener's cache drops the item and falls back to querying each object
directly, so everything works — Orca included — but every tree change costs a
round trip per node instead of one broadcast. It is loud in the journal and it
will scale badly on a large tree.

The signature is emitted by `accesskit_atspi_common` 0.19.1, so this is
upstream rather than ours.

- [ ] Confirm which side is stale: at-spi2-core's expected `CacheItem`
      against what AccessKit sends
- [ ] Check whether a newer `accesskit_unix` already emits the current form
- [ ] Report upstream with the version pair if it does not
- [ ] Measure whether it actually costs anything on a big tree — 70 file
      rows would be the test — before treating it as urgent

## Bug: a tray item reads its whole tooltip

Tray buttons are named from their SNI tooltip, which is right for most of them
("Rete", "1Password") but wrong for any application that writes several lines
into it. The battery announces:

```
Battery is discharging (99% remaining)\n22 hours, 10 minutes remaining\n[BAT1] Maximum Capacity: 81,05%
```

as its *name*, so a screen reader reads three lines where it should read one.

- [ ] Name the button from the tooltip's title, and put the rest in the
      description, which is what descriptions are for
- [ ] Collapse newlines in whatever is left, so nothing reads as one run-on
      sentence

## Bug: a shortcut line is neither described nor reachable

The Keyboard pane's shortcut lines are three controls on one line — the action
pop-up, the key combination field, the delete button — and `describe_row`
skips them rather than announcing the line as one thing it is not. So the whole
shortcuts list is invisible to a screen reader and unreachable from the
keyboard, including the line that adds a new shortcut.

- [ ] Describe a line as a group of its three controls, each with its own
      bounds and its own stop
- [ ] The add line as a button
- [ ] Editing a combination from the keyboard needs care: the field captures
      the next key press, which is also how the user would leave it

## Not yet verified

Not bugs, but not covered by the last pass either — both need input into the
session to reach, which is not something to do to a desktop somebody is using:

- the app switcher, while it is held open
- the all-windows overview
