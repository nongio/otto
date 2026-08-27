# Launcher

**Status:** draft
**Related specs:** [context-menus.md](./context-menus.md), [topbar.md](./topbar.md)

## Summary

A keyboard-driven overlay that appears over the desktop, filters a list of things
as the user types, and acts on the one they pick. It ships knowing about two
kinds of thing — installed applications and open windows — and is built so a
third kind (files, clipboard history, a calculator) is another provider rather
than another launcher.

## Goals

- Opening it, typing a few characters and pressing Enter starts an application
  or focuses a window, with no pointer involved.
- Ranking puts what the user meant first: a prefix beats a match in the middle,
  a run of adjacent characters beats scattered ones, and a short name beats a
  long one that merely contains the same letters.
- It reads as part of Otto: the same frosted material, corner radius and shadow
  as the bar's menus and the islands.
- It is disposable — started fresh on a keystroke, gone as soon as something is
  chosen. No daemon, and nothing to keep warm.
- Adding a new kind of result requires implementing one provider interface and
  registering it; nothing in the filtering, layout or input handling changes.

## Non-Goals

- Running arbitrary shell commands typed into the field.
- Searching file contents, or anything that would need an index.
- Remembering what was picked before, or ranking by frequency.
- Being configurable by theme file. It follows the desktop's colour scheme and
  icon theme, and nothing else.

## Behavior

**Appearing.** On start the launcher covers the output and takes the keyboard
exclusively: every keystroke belongs to it while it is up, including ones the
previously focused window would have wanted. A card sits above centre, showing
a query field and the first results. Anything given on the command line is the
query it starts with, so a binding can open it already narrowed.

**Modes.** A run offers applications, windows, or both, chosen when it starts.
Applications is the default: the two bindings mean "launch something" and
"switch to a window", and a mode that quietly does both is neither. The empty
field names the mode it is in — it is the only thing on screen that says which
one is up.

**At rest.** With nothing typed, the launcher does not list everything it
knows. It shows the last three applications launched from it, most recent
first, and nothing else — nothing at all until something has been launched, in
which case the card is the query field alone. The window switcher is the
exception: browsing is its purpose, so an empty query there lists every window,
most-recently-focused first.

A query that matches nothing says so. An empty query does not: a launcher that
has just opened has not failed to find anything.

**Typing.** Each keystroke re-ranks the whole list; results and the card's
height follow immediately. Text typed into the field must be visible in the
field. Everything a provider has is searchable, whether or not it is shown at
rest.

**Arithmetic.** A query that is a complete arithmetic expression containing at
least one operator is answered: the result appears as the first row, above the
matches, and acting on it copies the result. A bare number is a search, not a
sum. A comma is a decimal separator; when a number carries both a comma and a
dot, the last of the two is the decimal separator and the other is grouping.
The answer is written with whichever separator the question used.

**Choosing.** Up/Down move the selection and wrap at both ends. Tab and
Shift+Tab do the same. Page Up/Page Down move by a screenful. Ctrl+N and Ctrl+P
mirror Down and Up. The list scrolls to keep the selection visible; at most
eight rows are shown at once. Enter acts on the selection. Escape closes the
launcher without acting.

**Editing.** The field supports caret movement, selection with Shift, word-wise
motion with Ctrl, select-all with Ctrl+A, deleting the previous word with
Ctrl+W, and clearing the query with Ctrl+U.

**Pointer.** Moving the pointer over a row selects it. Releasing over a row acts
on it. Pressing anywhere outside the card closes the launcher, as Escape does.
The row under the pointer must be the row that highlights.

**Acting.** Choosing an application starts it detached, in its own process
group, with desktop-entry field codes stripped and `Terminal=true` entries
wrapped in a terminal, and records it at the front of the launch history. An
application that failed to start is not recorded. Choosing a window focuses it, un-minimising it first if
it was minimised. The launcher exits once the action has been carried out; if
the action fails it stays up and reports the failure rather than vanishing
having done nothing.

**Losing focus.** If the keyboard is taken away after the user has interacted
with the launcher, it closes. Before the first interaction it does not — that
would be closing on the way up.

**Changing state.** A window opening or closing while the launcher is up updates
the list, keeping the selection on the same item where that item still exists.

## Constraints & Edge Cases

- **The card must not dim what it frosts.** A scrim painted by the launcher
  covers the desktop that the compositor's blur samples, so the frost becomes a
  blur of flat grey. Any dimming has to come from the compositor, not from a
  client-painted layer behind the card.
- **Drawn position and Wayland position must agree.** The card is a subsurface
  whose material the compositor supplies. Moving it by surface style alone moves
  only where it is drawn: the compositor still hit-tests the pointer against the
  subsurface's own position and reports coordinates relative to it, so hover
  lands on the wrong row. Both must be set.
- **The colour scheme arrives after the card does.** The scheme comes from the
  settings portal, which answers asynchronously — normally after the launcher
  has built its surfaces and drawn its first frame from the default (light)
  scheme. Everything that was coloured from it — row text, field text, the
  divider and highlight, and the card's frost colour — must be rebuilt when the
  answer lands, or the launcher shows dark-theme text on a dark card. Rebuilding
  the frost must not also rewind the card's entrance state.
- **The query field must be laid out before it is drawn.** A field with no width
  scrolls its own text out of its clip, and the launcher then looks like it is
  ignoring the keyboard while the list filters correctly.
- **Icon lookup is on the typing path.** Resolving an icon must be cached and
  must not re-scan the icon theme directories per lookup; a screenful of
  uncached icons is otherwise seconds of work between keystrokes.
- Desktop entries that are hidden, that say `NoDisplay`, or that have no `Exec`
  are not offered. A user-level entry shadows the system entry of the same name
  rather than appearing twice.
- Windows with no title are not offered: there is nothing to type against.
- The launch history is state, not configuration: it is written by the program,
  and losing it costs nothing but the resting list. It must be written whole and
  moved into place, so a launcher killed mid-write leaves the previous history
  rather than half of a new one. Entries naming an application that is no longer
  installed are skipped rather than shown.
- An answer cannot be put on the clipboard by the launcher itself: a Wayland
  selection dies with the client that offered it, and the launcher exits as soon
  as the answer is taken. Something that outlives it has to own the offer.
- If the compositor does not offer foreign-toplevel management, the launcher
  runs with applications only.

## Rationale

- **Exclusive keyboard, not on-demand.** The launcher is modal for as long as it
  is up. Anything less means a keystroke can go to the window behind it, which
  for a tool whose whole interface is typing is the one unacceptable failure.
- **A subsurface for the card.** The blur belongs to the card, not to the whole
  screen, and a surface's material applies to the whole surface — so the card
  needs a surface of its own. It also puts the launcher on the same material as
  the rest of the desktop for free.
- **Buffer allocated at full height, clipped shorter.** A shorter list is the
  same buffer with the compositor showing less of it, so the card can change
  height without reallocating or re-laying-out anything.
- **No frecency, but a resting list.** Ranking that changes with history makes
  the same query mean different things on different days, and the muscle memory
  of "type three letters, press Enter" is worth more than a better first guess.
  History earns its place only where nothing has been typed, where the
  alternative is a wall of names nobody reads.
- **A material that still frosts.** The card is opaque enough for small text to
  read against a busy desktop, and no more. Past roughly 85% the frost stops
  reading as frost and the card may as well be opaque, which throws away the
  blur the compositor is doing anyway.
- **Three, and nothing when there are none.** A resting list long enough to
  scan is a list worth reading instead of typing. Three is small enough to take
  in at a glance, and an empty one is honest: a launcher that has never been
  used knows nothing about what you want.
- **Wrapping selection.** A list that stops at the end makes the user check
  where the end was.

## Open Questions

- Should the launcher be bound to a key by default, and to which one?
- Should a query that matches nothing offer to run it as a command?
- Should the arithmetic answer offer a second action — opening the expression in
  the calculator application — and on what key?
- Files as a third provider: what is searched, and how is the result acted on?
