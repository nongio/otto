# Emoji Picker

**Status:** draft
**Related specs:** [launcher.md](./launcher.md), [localisation.md](./localisation.md), [accessibility.md](./accessibility.md)

## Summary

A card over the desktop for finding an emoji by name or by category and
having it typed into whichever window had the keyboard. It is the launcher's
kind of thing — the same material, the same entrance, started fresh on a key
and gone once something is picked — for the one job of putting a character
the keyboard does not have where the cursor is.

## Goals

- Opening it, typing a few characters and pressing Enter types the emoji into
  the previously focused window, with no pointer involved and no daemon.
- Browsing without typing shows every emoji by Unicode category, the ones
  picked recently first, and a category is one keystroke or one click away.
- The emoji is delivered as keyboard input, so it lands in any client that
  takes a keyboard — every toolkit, every terminal, X11 included — rather than
  only those speaking a text-input protocol.
- Skin tone is a setting that applies to the whole palette and is kept
  between runs.
- It reads as part of Otto: the launcher's frosted material, corner radius,
  shadow and entrance.
- A screen reader can read the field and the cells, and pick one.

## Non-Goals

- Keywords beyond Unicode's names and categories. Shortcodes (`:tada:`),
  translations and synonyms are a later data source, not a different picker.
- Per-emoji tone choice: a tone is a setting, not a submenu.
- Multi-person sequences with two different tones. There are hundreds and a
  single tone setting cannot express them.
- Rendering emoji the installed font cannot draw. They are left out.
- Any persistent process. The picker starts on a binding and exits.

## Behavior

**Appearing.** On start the picker covers the output and takes the keyboard
exclusively, and must have it from the moment it appears rather than from the
first key pressed — anything typed into a card that is on screen has to land
in it. A card sits above centre showing, top to bottom: a query field, a strip
of tabs (recently used, then one per Unicode category), a grid of ten columns,
and a footer naming the selected emoji with the skin-tone swatches on its
right. Anything on the command line is the query it starts with. Pointer input
is taken over the card alone: a press beside it reaches what is under it, and
the keyboard leaving the picker closes it.

The palette opens on the recent picks when there are at least a full row of
them, and on the first category otherwise: a pane fills the card, and opening
onto two or three emoji in an otherwise empty grid reads as a broken palette.

**Where the card goes.** Beside the desktop's text cursor when its position is
known, and pointing at it: the card is a balloon with a beak on the edge
facing the caret, one shape rather than a panel with something attached, so
the frost, the border and the shadow run round the point as well. It goes
below the caret, flipped above when it would run past the bottom of the usable
screen, and grows into whichever side has room — rightwards from the caret
while that fits, leftwards when it does not — with the beak landing on the
caret and never in a rounded corner. The caret itself is never covered — the point is to keep the text
being typed visible. Where no application has reported a caret, the card is
centred above the middle of the output instead. Reporting a caret is optional
and many applications never do, so both placements are ordinary.

**The palette.** With nothing typed, each category is a *pane* of its own,
filling the width of the grid, with the recent picks — as they were typed, tone
included — as the first pane. Panes sit side by side in Unicode's palette
order. Emoji that take a skin tone are shown in the current tone.

Scrolling works in two directions, as a column browser does: a horizontal
scroll pans between panes, and a vertical scroll moves within the pane under
the pointer. Both fling on and settle when a touchpad gesture ends, and both
rubber-band when pulled past an end. A gesture belongs to whichever axis its
first movement chose, and keeps it until the fingers lift. A notched wheel
scrolls the pane under the pointer one step at a time, with no fling.

A pan that comes to rest between two panes settles onto the nearer one, and
the selection follows onto the pane landed on. The tab strip's marker sits
under the pane the pan is resting on; clicking a tab, or `Tab` / `Shift+Tab`,
pans to that category and selects its first emoji.

**Searching.** Matches are one pane, so there is nothing to pan between while
a query is showing. Each keystroke re-ranks the whole table. Every word typed must
begin a word of the name, or failing that appear in the name, or failing that
in the name, subgroup or category. An exact name ranks first, then a name that
begins with the query, then word-prefix matches, then substring matches, then
category matches; ties keep palette order. Matches are shown as one flat grid
with no headings, the first one selected. A query that matches nothing says so
in the grid. Clearing the query returns to the palette.

**Selecting.** One cell is selected. The arrows move it: left and right walk
the cells when the field is empty, stepping into the neighbouring pane at each
end (and move the caret once something is typed); up and down move a row
within the selection's own pane, keeping the column, stopping at the pane's
first and last rows. A row shorter than the grid catches a column moving onto
it rather than losing the selection. Page keys move a screenful; Home and End
go to the first and last cell of the current pane. Pointer motion over a cell
selects it. The selection is highlighted, and the highlight slides rather than
jumps.

**Tone.** The footer shows six swatches: the unmodified yellow and the five
Fitzpatrick modifiers. Clicking one sets the tone, re-renders the palette in
it, and saves it. The recent list is not re-toned: it records what was typed.

**Picking.** Enter, a click on a cell, or a screen reader's activation picks
the selected emoji. The pick is added to the front of the recent list, the
card animates out, and once it has gone the emoji is delivered:

**Delivery is chosen per pick.** Typed keys reach every application that turns
a keysym into text — terminals, and GTK and Qt apps — but not Chromium, which
truncates a keysym to sixteen bits and so cannot receive any character above
the Basic Multilingual Plane, where the emoji are. So the picker types when it
can and pastes when it cannot: an emoji inside the BMP, or any pick going to a
terminal, is typed; a non-BMP pick going to anything else is put on the
clipboard and delivered with a paste keystroke. The focused application is
identified through the foreign-toplevel list before the picker's own surface
exists. A pasted emoji is left on the clipboard, and the picker stays alive to
serve it until something else is copied — restoring the previous clipboard
would mean reclaiming the selection without focus, which the compositor
refuses.

- By default it is *typed*. After the picker's surfaces are destroyed and the
  compositor has confirmed it, the picker creates a virtual keyboard with a
  keymap of one key per codepoint of the emoji and taps those keys in order.
  The focused client receives the keymap and the keys and produces the
  characters. If the compositor offers no virtual keyboard, the pick is copied
  instead.
- With `--copy` it is put on the clipboard, and the picker stays alive,
  invisible, until the selection is taken from it — a clipboard offer lives
  as long as its client.

**Closing.** Escape, a press beside the card, and the keyboard going elsewhere
close the picker without a pick.

**Accessibility.** The field is a search input with its value; the grid is a
list box whose visible cells are options named after the emoji, the selected
one focused; activating an option picks it.

**Localisation.** The field's placeholder, the empty message and the category
headings are catalogue strings. Emoji names are Unicode's and are shown in
English; translating them is the missing data source above.

## Constraints & Edge Cases

- The parent surface covers the output; its input region and the card's are
  both set to the card's rectangle, or the dock beneath stops answering.
- The usable screen is shorter than the output: the dock sits at the bottom and
  a layer surface anchored to the whole output is told the output's size, not
  the part of it nothing is covering. The card leaves that room itself.
- The picker must not paint faster than the compositor hands it frames. Input
  arrives far faster than frames do, and painting on each event queues commits
  the compositor cannot keep up with until the connection blocks — at which
  point the picker hangs holding an exclusive keyboard grab, and the whole
  session stops accepting input. A burst of input has to become one frame.
- A compositor that only moves the keyboard onto an exclusive layer surface
  when the next key is pressed loses everything typed before it. The focus has
  to follow the surface mapping, and be granted once, so a later frame cannot
  take it back from a window the picker handed off to.
- A client that follows `text-input` to the letter reports no caret at all
  until the compositor has told it that its text input has focus, and holds
  every later report until the compositor has acknowledged the previous one.
  Chromium is one, so the compositor must send those whether or not an input
  method is running — a desktop with no input method is the ordinary case, and
  a caret that never arrives centres the card for a browser that had one.
- Emoji whose first codepoint the emoji font has no glyph for are dropped at
  start, so a font older than the data leaves gaps rather than boxes.
- Sequences are shaped as one run in one font. Split by script or font, a
  joiner sequence comes out as its parts.
- The delivery keymap uses `U+XXXX` keysyms and one keycode per character, so
  any sequence can be typed without a keysym name for each.
- Those keycodes must be ones a standard keyboard calls *printable* — the
  digit row and the letter rows. A key's meaning comes from the keymap, but
  Chromium, and so Electron and every HTML input, decides what a key **is**
  from the raw code through a fixed table before it reads the keymap: a code
  outside that table is dropped, and one that names a non-printable key
  (Escape, a modifier) yields no text for anything but plain ASCII. This is
  why the same technique fails in those applications when the keys are put on
  arbitrary codes.
- A sequence longer than the printable keys available cannot be typed in one
  go; no emoji comes close, but the limit is real.
- The virtual-keyboard keymap is the compositor's to send: it must forward
  the keys to the focused client with the virtual keyboard's own keymap, and
  restore the physical keyboard's afterwards.
- A pick made from the keyboard uses that keystroke's serial to claim the
  clipboard in `--copy` mode; a pick from a screen reader uses the last input
  serial the app saw.

## Rationale

- *Typing over text-input.* An input-method commit reaches only clients that
  bind `text-input`, and only one input method may be active; a keyboard
  reaches everything and conflicts with nothing. This is how `wtype` works.
- *A single tone setting.* Every picker with per-emoji tone menus makes the
  common case — one person, one tone — two clicks. A setting makes it none.
- *Recent picks keep their tone.* They are a record of what was typed, and
  retyping the same thing is what recency is for.
- *No fuzzy matching.* Names are short and made of common words; fuzzy
  matching on them finds things that were not meant.
- *Panes rather than one long list.* Categories as a single scrolling column
  makes reaching Flags a long drag and gives the tab strip nothing precise to
  point at. Side-by-side panes make a category a place, reachable by a swipe
  or a tab, and match the two-axis feel of the file browser's columns.
- *Baked-in data.* The table is Unicode's `emoji-test.txt` slimmed to what the
  palette shows, regenerated by a script, and compiled in: nothing to install,
  nothing to find at runtime.

## Open Questions

- Whether to fall back to typing the emoji through `text-input` when a client
  offers it, for clients that filter virtual-keyboard input.
- A shortcode data source (CLDR annotations) for searching in the desktop's
  language.
