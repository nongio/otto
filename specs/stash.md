# Stash

**Status:** draft
**Related specs:** [launcher.md](./launcher.md), [file-browser.md](./file-browser.md),
[side-canvas.md](./side-canvas.md)

## Summary

`otto-stash` collects things to ask about from anywhere on the desktop (selected
text, files, a part of the screen) into one small card, and hands them to Ask
together. It lets a question be put together across several apps before it is
asked, without copying and pasting into the launcher.

## Goals

- One shortcut adds whatever is selected in the app in front: the selected
  text in a text field, the selected files in a Files window, or the primary
  selection of any other app.
- otto-stash never takes the keyboard, so the app in front keeps its focus,
  its selection and its caret.
- The card shows exactly what will be sent, and anything on it can be taken
  off or left out before sending.
- The same thing stashed twice is on the card once.
- Everything stashed reaches Ask as attachments to the next request, and the
  stash ends when that request is sent.

## Non-Goals

- Dictation, and replacing the selection with the answer. Both are planned for
  later milestones and are not part of this spec.
- Sending to an agent directly from the card. The card collects; Ask asks.
- More than one stash at a time, or a stash that survives a restart of
  the service or the session.

## Behavior

**The service.** `otto-stash` runs once per session, started without
arguments, and stays up. It owns `org.otto.Stash1` on the session bus. Every
other use of the command is a trigger that talks to the running service:
`otto-stash add`, `add-file PATH`, `add-region`, `send` and `cancel`. A
trigger with no service running fails and changes nothing.

**Adding the selection.** `otto-stash add` starts a stash if there is
none, then adds what is selected in the app in front, looking in this order:

1. The focused text field, when it reports its surrounding text with a
   selection in it: the selected text is added.
2. Otherwise, the Files window that has the keyboard, when there is one: its
   selected files are added, in the order the window shows them. With nothing
   selected in it, nothing is added.
3. Otherwise, the primary selection: its text is added, unless it is empty or
   only whitespace.

The call returns once the item is on the card, or once it is clear nothing
was selected. Text taken from the primary selection is capped at 1 MiB; the
rest is dropped.

**Adding a file.** `otto-stash add-file PATH` adds that file or folder. Files
offers the same thing on its selection as **Add to Stash** (see
[file-browser.md](./file-browser.md)).

**Dropping.** Files dragged from any app and dropped on the card are added,
one item per file, in the side canvas as on the floating card.

**Starting a stash by dropping.** When the side canvas announces drags
(`otto-canvas-v1` version 4) and there is no card in it, a drag that offers
`text/uri-list`, or does not say what it offers, puts a drop invitation in the
canvas where the card would sit: a short card with a dashed outline that
reads "Drop files here to stash them", in the accent colour while the drag is
over it. The canvas opens when the drag rests at the right edge of the screen
(see [side-canvas.md](./side-canvas.md)). Files dropped on the invitation
start a stash with them on the card, which takes the invitation's place once
drawn; the canvas stays open throughout. A drag that ends anywhere else takes
the invitation away, and the canvas closes if the drag opened it. A drag that
offers something else, or starts while the card is up, adds no invitation.

**Adding a region.** `otto-stash add-region` hides the card, lets the person
drag out a rectangle on screen, and adds a capture of it as a picture. The
card is hidden for the pick so it is never in the capture (in the side
canvas, the canvas is hidden too), and comes back afterwards. A pick that is cancelled adds nothing. Only one pick runs at a
time; a second request while one is open is ignored.

**Deduplication.** An item equal to one already stashed is not added again:
the same text, the same file path, or the same region. Its place on the card
does not change. When the equal item is struck out, adding it again brings it
back instead.

**Where a stash shows.** In the side canvas, a new stash starts on the card,
which slides the canvas in; `otto-stash send` opens Ask with it. Otherwise
a new stash opens Ask with its first item attached, and what is added while
Ask is up joins it there. When Ask closes without
sending, the stash is set aside on the card, and from then on adds go to the
card until the stash ends. Ask closing on a stash that is still empty (nothing
was selected) ends it instead of leaving an empty card. When Ask cannot be
started, the stash goes to the card at once.

**Where the card goes.** When the compositor offers the side canvas
(`otto-canvas-v1`) at version 3 or later, the card is an item in the canvas.
Otherwise (another compositor, or an older Otto) it floats as a balloon.
The choice is made once, when the service starts.

**The card in the side canvas.** Once the stash is set aside and until it
ends, the card sits at the top of the canvas column, above the Agents panel,
at the column's width and as tall as its content (up to the same limit).

- When the card appears, and after every add (a selection, files, a region,
  a drop), it asks the canvas to show itself. The canvas slides in on the
  output under the pointer; the keyboard stays with the app in front, so its
  selection and caret are kept. The request is ignored while the canvas is
  already shown, while the session is locked and during exposé; the next add
  asks again.
- The canvas then stays open until the person hides it (swipe, toggle, a
  click outside it) or the stash ends. A click in the app reaches the app,
  and the canvas hides when the button is released outside it; a press that
  starts a drag leaves it open, so files dragged from a Files window can be
  dropped on the card. Because the canvas opened by itself, Escape stays
  with the app. See [side-canvas.md](./side-canvas.md).
- Clicking the card never takes the keyboard. The card does not move; there
  is nothing to drag.
- When the stash ends, or Ask shows it, the card leaves the canvas at once.
  The canvas stays as it is, open or closed, with its other items. When Ask
  closes without sending, the card comes back and the canvas shows itself
  again.
- While the canvas is hidden the card does no frame work: a scroll glide
  waits for the next show, and an item on its way out goes at once.

**The floating card.** Once the stash is set aside and until it ends, a card
shows what is stashed. It sits on top of everything, first placed in the
top-right corner of the output, just below the bar, and can be dragged
anywhere by pressing on it away from its buttons. It never takes the
keyboard.

**The card, either way.** It wears the launcher's frosted material and
corners (the floating card also its shadow), and follows the desktop's
colour scheme and accent; without the compositor's material it draws a
plain, near-opaque background of its own.

- The title reads "Ask about…".
- While anything is stashed, a gray **Clear** button sits at the right end
  of the title line, with the count just before it: the number of items, or
  "N of M" when some are struck out.
- The items are listed newest first, as Ask lists attachments: selected text
  reads as the text itself, a region as its picture, a picture file as its
  picture under its name, any other file as its icon or thumbnail and its
  name, as in Files.
- With nothing stashed yet (an add that found nothing selected), the card
  says "Select something, then add it".
- Under the items, "Send to Ask" and the shortcut that opens Ask, when one is
  bound: the shortcut bound to Ask, or failing that the one bound to
  `otto-stash send`. The shortcut is looked up each time the card opens, so a
  rebound key shows. With no such shortcut, the line is left out.
- The card grows with its items up to a limit, past which the items scroll
  between the title and the footer. In the side canvas the limit is the
  card's share of the column's height, so the panels under it keep theirs
  (640 points on a compositor that does not share the height); floating, it
  is 640 points or the room on the screen, whichever is less. On the floating card,
  changes of size spring, as the launcher's card does.

**On the card, with the pointer.** Pointing at an item highlights it. A click
on an item (pressed and released on it without the floating card moving)
strikes it out, or brings it back: a struck item stays on the card, dimmed, and is not
sent. Each item has a remove button; clicking it shrinks the item away and
takes it out. Buttons take the hand cursor.

**Clearing.** Clicking **Clear** throws the whole stash away, struck items
included. `otto-stash cancel` does the same from a shortcut. Taking out the
last item also ends the stash.

**Closing.** The card closes when the stash ends (sent, cleared, cancelled
or emptied) and while Ask shows the stash in its place. The floating card
fades out rather than vanishing, in about 150 ms; without the compositor's
styles there is nothing to fade with, and it goes at once. In the side
canvas the card leaves at once, and the column closes up behind it.

**Handing over to Ask.** `otto-stash send` opens Ask when something is
stashed, and does nothing otherwise. However Ask is opened (by this command,
its own shortcut, agents mode, or `otto-launcher --selection`), it shows the
stash with the next request, following it live: items added while Ask is
up appear there, and striking out or removing an item in Ask does the same in
the stash. While Ask is up the card steps aside. Closed without sending,
Ask lets go and the card shows the stash as it was left. When
Ask sends a request while anything is stashed, the items not struck out go
with it and the stash ends, struck items included. How attachments look
and behave in Ask is in [launcher.md](./launcher.md).

**Asking about the selection in one step.** `otto-launcher --selection` adds
what is selected in the app in front, as `otto-stash add` would, before its
card takes the keyboard, then opens Ask with it. The selection joins whatever
is already stashed, and the stash card never shows for it. The launcher
waits for the add only briefly (under a second) before opening regardless.

**Shortcuts.** The default configuration binds the first four, and starts
otto-stash with the session. Each can be rebound in the compositor's shortcut
configuration. The rest are suggestions:

| Keys | Command |
|------|---------|
| Ctrl+Alt+G | `otto-stash add` |
| Ctrl+Alt+Shift+R | `otto-stash add-region` |
| Ctrl+Alt+Shift+G | `otto-stash send` |
| Ctrl+Alt+Shift+C | `otto-stash cancel` (clear) |
| Ctrl+Alt+A | `otto-launcher --ask` (Ask, shown on the card as the send key) |

Inside Files, Ctrl+G adds the selection to the stash (see
[file-browser.md](./file-browser.md)).

### Wire contract: `org.otto.Stash1`

Bus name and interface `org.otto.Stash1`, object path `/org/otto/Stash1`, on
the session bus:

```
Add() → ()                      add what is selected in the app in front
AddFile(path: s) → ()           path must be absolute
AddRegion() → ()
Send() → ()                     open Ask
Cancel() → ()                   throw the stash away
Items() → a(sb)                 everything stashed, oldest first
Toggle(index: u) → ()           strike out, or bring back
Remove(index: u) → ()
Hold() → ()                     the caller shows the stash
Sent() → ()                     the stash went with a request
signal Changed(items: a(sb))    after every change
```

- Each item travels as an absolute path and whether it is struck out. Files
  and regions are their own paths; text is written to a file in the
  stash's directory under the user's runtime directory, named
  `selection-N.txt`, and a region's capture is `region-N.png` there. Anyone
  showing the stash reads those two names back as text and as a region.
- A text item keeps its file name while other items come and go.
- `Hold` hides the card until the caller leaves the bus.
- `AddFile` with a relative path is refused. `Toggle` and `Remove` with an
  index past the end change nothing.

## Constraints & Edge Cases

- **Only the focused Files window answers.** Files is asked which window has
  the keyboard, not which one was used last. A Files window with the keyboard
  and nothing selected answers with nothing, and the add adds nothing: it
  does not fall through to the primary selection, which holds whatever was
  last selected in any app and would pick up another window's selection.
  Files windows without the keyboard refuse the question, so their
  selections are never stashed.
- **Files is given 300 ms to answer.** No answer in time is treated as no
  Files window in front, and the primary selection is used.
- **One input method per seat.** The service reads the focused field's
  selection as an input method. When another input method already holds the
  seat, the service exits rather than fight it.
- **Region capture needs `slurp` and `grim`.** Without them, a region pick
  fails, is logged, and adds nothing.
- **Adds racing an end.** When the primary selection is still being read as
  the stash is sent or cleared, the text that arrives late is dropped
  rather than starting a new stash.
- **Removing while an item shrinks away.** A second removal during the shrink
  finishes the first at once, so the right item goes.
- **The card follows the output.** When the output the floating card is on
  goes away, the card closes; the next change opens it again.
- **An older Otto.** A compositor with `otto-canvas-v1` below version 3
  gets the floating card, since the card needs to show the canvas and to
  never take the keyboard.

## Rationale

- **A stash starts in Ask, unless there is a side canvas.** Most stashes
  are one thing to ask about right away, so without the canvas the first add
  goes where the question is typed. With the canvas, the card shows up
  beside the app without taking the keyboard, and sending is one shortcut
  away. The card is for
  the other case: set aside by closing Ask, it collects more while the person
  keeps working. This makes `otto-launcher --selection` the same as the add
  shortcut, so it isn't bound by default.

- **Default shortcuts ship.** Collecting has to be one key press from any app,
  and a feature nobody can reach until they edit their config goes unused. The
  keys sit on Ctrl+Alt, clear of the app shortcuts on Ctrl alone.

- **Never taking the keyboard.** A selection is often lost when its window
  loses focus, and the point of stashing is to keep working in the app. The
  card is pointer-only for that reason.
- **Focused Files window only, and an empty answer is an answer.** When the
  Files window in front had nothing selected, the add used to fall through to
  the primary selection and stash text selected earlier in another window.
  Treating "nothing selected" as final makes the shortcut add what the person
  is looking at, or nothing at all.
- **Deduplicate, and un-strike on re-add.** Pressing the shortcut twice is a
  common slip and should not double an item. Adding a struck item again is a
  clear sign it is wanted after all.
- **Clear is quiet.** The Clear button is gray and small, at the far end of
  the title, so it is findable but not the obvious thing to press.
- **Items travel as files.** Ask already attaches files, and agents already
  read files with their own tools, so text and regions become files and every
  consumer handles one kind of thing.
- **Fading out.** The card disappearing in one frame read as a glitch; the
  fade matches the launcher's.
- **The side canvas when there is one.** A floating card covers whatever is
  under it and has to be dragged out of the way; the canvas is where
  glanceable panels already live, slides away with a swipe, and comes back
  on the next add. Opening it without the keyboard keeps the one thing the
  stash promises: the app keeps its selection.

## Open Questions

- Where the floating card lives while collecting: at the pointer or caret,
  fixed, or a small indicator that opens for review.
- Whether a stash should survive a lock or a restart.
- Whether the Esc key should throw the stash away, which needs the card to
  take the keyboard at least while it is pointed at.
