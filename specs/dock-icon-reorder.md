# Dock Icon Reordering

**Status:** draft
**Related specs:** context-menus.md

## Summary

The user arranges the dock by dragging its icons: press an icon, move along the
dock, and the icon takes the place it is dropped on. The order this produces is
the user's, so it outlives the session — it is the order the dock's bookmark list
is stored in.

## Goals

- Let the user reorder the dock's launchers by dragging, without opening a menu.
- Make the icons a drag passes over move out of its way, so the order the user is
  about to commit is visible before they let go.
- Persist the resulting order, so the dock comes back the same way next login.
- Never lose a click: a press that does not move still launches or focuses the app.

## Non-Goals

- Dragging icons out of the dock to remove them. Removal stays a context-menu
  action.
- Dragging files, windows or anything from outside the dock onto it.
- Reordering the minimized-window strip. Those entries are ordered by when they
  were minimized and have nothing to persist.
- Reordering the running-but-not-bookmarked section among itself (see Behavior 8).

## Behavior

### Starting a drag

1. Pressing the left button on an app icon does not, on its own, start a drag.
   The press becomes a drag only once the pointer has travelled a short distance
   **along the dock's long axis** — along x for a bottom dock, y for a side one.
   Below that distance the press is still a click and releasing launches or
   focuses the app as usual.
2. A right-button press never starts a drag; it opens the icon's context menu.
3. The dock goes on magnifying under the pointer for the whole drag, exactly as
   it does under any other pointer. Any tooltip is hidden.
4. The dragged icon is lifted: it is drawn above its neighbours, a tenth larger
   than the slot it came out of, and is **centred on the pointer** along the
   dock's long axis. It does not follow the pointer across the dock — it stays in
   line with the row of icons, so a drag away from the screen edge does not pull
   it out of the dock.
4a. The lifted icon is the size of the slot it left, magnification included, and
   grows and shrinks with it. That slot is under the pointer, where the
   magnification peaks, so a lifted icon is a large one and stays large for the
   length of the drag.
4b. The row the lifted icon lines up with is the row as it is drawn at that
   moment: icons are aligned to the screen edge, so a magnified one sits further
   from that edge than an unmagnified one, and the lifted icon sits with it.
5. The slot the icon came from stays in the layout and stands empty. The dock
   neither grows nor shrinks while an icon is being dragged.

### Moving

6. The dock is divided into slots of one icon each, of the sizes the
   magnification gives them. The dragged icon takes **the slot the pointer is
   over**, so it changes places when the pointer crosses from one slot into the
   next. Every icon between the slot it left and the slot it took shifts one
   place the other way, and **animates** into its new place rather than jumping —
   by the width of the slot it crossed, which under magnification is not the
   same for every icon.
6a. A slot's size and position follow from its index and where the pointer is,
   never from which application is standing in it, so a swap moves icons between
   slots without moving the slots. The boundary the pointer crosses to cause a
   swap therefore stays put across that swap, and is crossed once.
7. A drag that moves several slots in one motion is the same thing: every icon it
   passed shifts one place, all of them animating.
8. A drag is confined to the launcher section. The running apps that are not
   bookmarked follow the launchers and have no persisted place, so the dragged
   icon cannot be pushed past the last launcher, nor before the first.

### Running apps that are not bookmarked

9. Dragging a running app that is not a bookmark **adds it to the bookmarks**, at
   the end of the launcher list, at the moment the drag starts. From then on the
   drag is an ordinary reorder, and the app stays in the dock after it quits —
   the same outcome as the context menu's *Keep in Dock*.
10. This happens only for a real drag. Clicking such an icon, or pressing it and
    releasing without moving, does not add it to the dock.

### Ending

11. Releasing the button drops the icon into the slot it currently occupies: it
    animates from the pointer into that slot, letting go of the tenth it was
    lifted by to settle at the size the magnification gives that slot, and the
    slot shows it again.
12. The release that ends a drag does **not** activate the app.
13. Once the icon is dropped, the new order is written to the stored bookmark
    list. Nothing is written if the icon ended where it started.
14. The magnification never stopped, so the drop changes nothing about it.

## Constraints & Edge Cases

- **Release outside the dock.** The button may come up anywhere. Wherever it
  does, the drag ends as in (11)–(14) and the icon is dropped in the slot it
  last occupied; the icon is never left lifted.
- **The pointer leaving the dock mid-drag** does not end the drag. The dock
  stays magnified where the pointer last was until the button comes up.
- **Bookmarks the dock could not load** — an entry whose desktop file is missing —
  have no icon to drag and no place in the visible order. Persisting a new order
  must keep them, in their existing relative order, rather than dropping them.
- **The app list changing mid-drag** (an app launches or quits) must not move the
  dragged icon or abandon the drag.
- **Layout order and model order must agree.** The dock decides which icon to
  magnify from its position in the list, so any reorder has to move the icon in
  the layout and in the list together, or the wrong icon bulges under the pointer.

## Rationale

- **Only launchers are sortable.** The dock shows bookmarks first and running
  apps after; there is nowhere to store the position of an app that is only
  running. Rather than forbid the drag, dragging one promotes it — the user
  asking to place an app is asking to keep it.
- **Magnification stays on during a drag.** A dock that flattens the moment an
  icon is picked up pulls the whole row out from under the hand carrying it, and
  hands the icon back a second time on the drop. Keeping it on costs nothing in
  precision: the magnified slots are an uneven ruler, but a ruler all the same,
  because a slot's size depends on the pointer and not on what is standing in
  it. The drag reads the ruler — it asks which slot the pointer is in — rather
  than counting equal pitches out from where the press landed.
- **A movement threshold, not a time delay.** Waiting before a press becomes a
  drag makes the dock feel unresponsive to clicks; a distance threshold
  distinguishes the two intentions immediately.
- **The order is written on drop, not on every swap.** A drag crosses several
  slots; persisting each one would rewrite the config file a dozen times for one
  gesture.

## Open Questions

- Should a drag that ends outside the dock, well away from it, be read as
  "remove from dock" rather than a drop in place? macOS does this; it is
  deliberately not implemented here.
