# Workspace Reordering

**Status:** draft
**Related specs:** [workspace-rename.md](./workspace-rename.md),
[workspaces-multi-output.md](./workspaces-multi-output.md),
[dock-icon-reorder.md](./dock-icon-reorder.md)

## Summary

The user arranges the workspaces by dragging them in the workspace selector:
press a workspace, move it along the strip, drop it where it should go. The
order this produces is the order the workspaces are in — the order the
compositor scrolls through, the order the shortcuts count in, and the order the
strip shows.

## Goals

- Let the user change the order of the workspaces by dragging one, without
  opening a menu.
- Make the workspaces a drag passes over move out of its way, so the order the
  user is about to commit is visible before they let go.
- Draw the workspace being dragged over the ones it passes, so it is visibly
  the thing being moved rather than one more preview sliding along.
- Keep every window with the workspace it is on, wherever that workspace ends
  up.
- Never lose a click: a press that does not move still switches to the
  workspace, and a second click still opens the rename editor.

## Non-Goals

- Reordering the window grid in exposé. Dragging a window there moves it
  between workspaces and is a different gesture entirely.
- Moving a workspace from one output to another. Workspaces are per output;
  a drag is confined to the strip it started in.
- Dragging a workspace out of the strip to remove it. Removal stays the
  preview's close button.
- Reordering by keyboard or by a protocol request.

## Behavior

### Starting a drag

1. Pressing the left button on a workspace preview, or on its label, does not
   on its own start a drag. The press becomes a drag only once the pointer has
   travelled a short distance **along the strip**. Below that distance the
   press is still a click: releasing switches to that workspace, and a second
   click on the label opens the rename editor.
2. A press on a preview's close button never starts a drag, and neither does a
   press on the add button.
3. When the drag starts, the dragged workspace is lifted: it is drawn **above**
   its neighbours, slightly larger than they are, and is centred on the pointer
   along the strip. It does not follow the pointer off the strip — a drag
   towards the desktop keeps it in line with the row.
   The lift is slight — enough to read as picked up, not as zoomed.
   The lifted workspace must be **visible for the whole gesture**, and must
   show the workspace itself — its wallpaper, its live windows and its name —
   not a blank rectangle. It rides inside the selector, which is the surface
   exposé puts on screen; it must not be moved onto any layer exposé hides.
   Its name is carried on a translucent rounded-rectangle plate sized to the
   text, so it stays legible over the previews and labels the drag passes
   over. The plate follows the desktop's corner setting like the rest of the
   chrome, and its padding, corner and text all scale with the output.
4. The slot it came from stays in the strip and stands empty. The strip neither
   grows nor shrinks while a workspace is being dragged.
5. **No close button is shown anywhere in the strip while a drag is in
   flight.** A drag sweeps the pointer across the previews it passes, and the
   release parks it on one, but the pointer is carrying a workspace rather
   than pointing at one — so none of that counts as hovering. Any button
   already showing when the lift starts is put away, none appears until the
   drop has finished, and hovering reveals them again afterwards.

### Moving

6. The strip is divided into slots of one workspace each. The dragged workspace
   takes the slot its centre is nearest, so it changes places once it has
   covered half of one. Every workspace between the slot it left and the slot
   it took shifts one place the other way, and **animates** into its new place
   rather than jumping.
7. A drag that crosses several slots in one motion is the same thing: every
   workspace it passed shifts one place, all of them animating.
8. A drag is confined to the strip: the dragged workspace can be pushed no
   further than the first or the last slot.
9. The selected workspace keeps its border on the workspace that had it,
   wherever the shuffle puts it — including when the selected workspace is the
   one being dragged, which carries the border with it while it travels.
9b. Every workspace keeps its **name** through the shuffle. A name identifies a
   workspace, not the slot the workspace is standing in, so the label the user
   picked up is the label they are still looking at when they drop it — and the
   labels of the workspaces that were pushed aside move with them. This holds
   for a name the user typed and for the default `Workspace N` alike: the
   number is the workspace's own, handed out when it is created, and is not
   re-derived from its position.
10. Anything that would refresh the strip while a drag is in flight — a window
   opening in a preview, a workspace being added elsewhere — must not put the
   strip back into the order the compositor still holds.

### Ending

11. Releasing the button drops the workspace into the slot it currently
    occupies: the lifted copy animates into that slot, settles at the size of
    its neighbours, and the slot shows the workspace again.
12. The release that ends a drag does **not** switch workspace, whatever the
    pointer is over when it comes up.
13. On drop, the new order becomes the compositor's order for that output:
    - the workspaces scroll in the new order;
    - the workspace the user is on is still the workspace they were on, at its
      new position, and the desktop does not scroll to somebody else's
      workspace as a result of the drop;
    - every window is still on the workspace it was on, at the same place on
      screen, and every workspace position it remembers points at the same
      workspace it did before — including a position that is not where the
      window currently is, which is how a fullscreen window remembers the
      workspace to restore itself to;
    - workspace names travel with their workspaces, and the strip comes back
      in the dropped order after a restart.
14. Nothing is committed if the workspace ended in the slot it started from;
    the click is still swallowed.

## Constraints & Edge Cases

- **A new workspace takes the lowest free number.** Default labels are numbers
  that belong to a workspace for its lifetime, so they cannot simply count the
  strip. A workspace added when 1, 2 and 3 exist is 4; close 2 and the next one
  added is 2 again. A strip can therefore read `1, 3, 4` after a removal, and
  read in any order at all after a reorder — which is the point.


- **Release outside the strip.** The button may come up anywhere. Wherever it
  does, the drag ends as in (11)–(13) and the workspace is dropped into the
  slot it last occupied; it is never left lifted.
- **The pointer leaving the strip mid-drag** does not end the drag.
- **A second output.** Workspaces are independent per output, so a reorder
  applies to the output whose selector was dragged in and leaves every other
  output's strip and current workspace alone. Each output's selector can be
  dragged in independently.
- **A workspace collapsing out of the strip** (its close button was pressed) is
  not a drag target and cannot be picked up.
- **A rename in flight.** A press anywhere outside the field commits the
  rename first; the same press may then start a drag.
- **Unfullscreen after a reorder.** A fullscreen window sits on a temporary
  workspace of its own and remembers the position of the workspace it came
  from. That remembered position is a position like any other and moves with
  the reorder, so leaving fullscreen puts the window back on the workspace it
  actually came from, wherever the drag left it.

- **Layout order and model order must agree.** The strip decides which
  workspace a slot belongs to from its position, so a reorder has to move the
  workspace in the strip and in the compositor together, or the drop lands the
  wrong workspace in the slot.

## Rationale

- **The lifted copy mirrors the workspace, not the preview.** The preview in
  the strip is itself a mirror of the workspace's wallpaper and windows. A copy
  made by mirroring that preview is a mirror of a mirror, and is fragile — and
  because lifting a workspace blanks the slot it came from, a lifted copy that
  fails to draw does not degrade to "no lift", it makes the workspace vanish
  for the length of the drag. The copy is therefore built the way a preview is,
  from the workspace's own nodes.


- **Same gesture as the dock.** The dock already reorders by dragging with a
  travel threshold, live displacement and a commit on drop
  ([dock-icon-reorder.md](./dock-icon-reorder.md)). Two reorder gestures that
  behave differently in the same desktop would be worse than one, so this is
  deliberately the same one.
- **A movement threshold, not a time delay.** Waiting before a press becomes a
  drag would make clicking a workspace feel unresponsive; a distance threshold
  tells the two intentions apart immediately.
- **Scoped to one output.** Workspaces are already independent per output —
  each has its own count, its own current workspace and its own selector — so
  there is no order shared between outputs for a drag to change. Reordering one
  output's strip from another output's selector would move workspaces the user
  cannot see.
- **Windows follow their workspace, they do not change workspace.** A window's
  place along the scroll axis is a function of its workspace's position, so the
  windows travel with their workspace for free. What does *not* travel for free
  is every workspace position remembered elsewhere, and those are **remapped
  through the move, not re-derived** from where things currently sit. The
  difference matters for exactly one case, and it is a case that exists: a
  fullscreen window's remembered workspace is deliberately not the workspace it
  is on, and re-deriving would overwrite it with the temporary one.

- **Positions, not identities.** Everything here is stored as a position in the
  strip — the current workspace, the workspaces windows remember, the keys the
  names are saved under. Reordering is therefore a permutation that every one
  of those has to be put through. Giving workspaces stable identities and
  storing those instead would remove the whole class of bug, and is the better
  design; it is a larger change than this feature, and is not made here.
- **The order is committed on drop, not on every swap.** A drag crosses several
  slots; committing each one would rewrite the compositor's workspace list — and
  the config file that holds the names — a dozen times for one gesture.
- **Persistence is as good as workspaces get.** Workspaces themselves are not
  persisted — a session always starts with the configured number of them — so
  the order cannot outlive a restart as an identity. What is stored is the
  names, by position, and those are rewritten on drop, so a strip the user has
  named comes back reading the way they dragged it.

## Open Questions

- Should a drag be able to carry a workspace onto another output's strip when
  two outputs are side by side? It would have to move the workspace's windows
  between outputs, which is a different feature; deliberately not implemented.
