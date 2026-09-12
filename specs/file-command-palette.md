# File Browser Command Palette

**Status:** draft
**Related specs:** [file-browser.md](./file-browser.md), [launcher.md](./launcher.md), [context-menus.md](./context-menus.md)

## Summary

A keyboard panel inside the file browser, opened with Ctrl+P, that finds any
command the window can carry out by typing a few letters of its name and runs
it — including the commands that need something more said about them, such as
a path to go to or a new name to give a file.

## Goals

- Reach every command the file browser already exposes in its menus, toolbar
  and shortcuts without knowing where it lives or which chord it is bound to.
- Rank commands by a typed fragment, tolerant of gaps and of the word order in
  the command's name.
- Let a command ask for an argument, and let the argument be typed in the same
  field, with completion and a visible prompt saying what is being asked for.
- Show only the commands that can actually run right now, and say why the
  others are absent by simply not offering them.
- Be extensible: the set of commands is data, gathered from providers, so a
  later source of commands (scripts, extensions, another process) is another
  provider and not another panel.

## Non-Goals

- Searching files. The palette finds *commands*; the launcher and type-ahead
  find things on disk. A future provider may add file results, but the first
  version does not.
- Command history, aliases or user-defined bindings.
- A remote or scripted provider. The seams for one are required (see
  Constraints); the provider itself is not built here.
- Replacing any existing chord, menu or context menu. The palette is an
  additional way in, never the only one.

## Behavior

### Opening and closing

- Ctrl+P opens the palette. It opens over the browser window, tucked under the
  lower part of the header and near its right edge, and takes the keyboard
  whole while it is up.
- The card is a surface of its own, not paint on the window. It may therefore
  be dragged clear of the window entirely — off its edge, over the desktop —
  which a card painted into the window's own buffer cannot be, there being no
  pixels out there to draw on.
- The card's top band — the line being typed — is the handle: pressing there
  and dragging moves the panel, including when that band hangs off the
  window. It is clamped to the *display*, so it can never be put somewhere it
  holds the keyboard from out of sight. The next open finds the card where it
  was last dropped — kept for the session, and in
  `$XDG_STATE_HOME/otto/files.toml` across runs, as an offset from where it
  opens so that a window moved or resized in between still gets a sensible
  place — and brought back inside the bounds if the window has shrunk.
- The palette is unavailable in the file picker, whose window is answering a
  request rather than managing files.
- Escape closes it, in one step, without running anything. Ctrl+P while it is
  open closes it too.
- Clicking outside it closes it. Losing keyboard focus closes it.
- Closing restores the selection, cursor and scroll position exactly as they
  were: the palette never changes the browser by being open.

### Searching

- The palette opens with an empty query and a resting list: every currently
  available command, grouped in a fixed order (Go, File, Edit, View),
  each group under its name.
- Typing filters and re-ranks. A query matches a command when its characters
  appear in order in the command's title, its keywords, or its group name; a
  match on the title outranks a match on anything else, a match at the start of
  a word outranks one in the middle, and runs of adjacent characters outrank
  scattered ones. A space in the query separates words rather than having to be
  matched, so "mo tr" finds "Move to Trash".
- While a query is present the list is flat and ranked, with the group shown as
  a dim badge on each row rather than as a heading.
- Up and Down move the highlight; the list scrolls to keep it visible. Home and
  End go to the ends.
- The pointer works the list the way it works a column: the row under it takes
  the highlight as it moves, so Return always does what the pointer is resting
  on; a click on a row picks it, exactly as Return on it would; a scroll over
  the card scrolls the list.
- Each row shows the command's title, its group badge, and — where it has one —
  the keyboard shortcut it is also bound to, right-aligned. A command that
  takes an argument shows its argument label after the title, dimmed, as
  `Go to Path  ›  path`.
- A query matching nothing shows a single dim line saying so, and Return does
  nothing. This is about the *command* list only: an argument with no
  completions — a name, a pattern, any free text — has none by nature, and says
  nothing about it.

### Running a command

- Return on the highlighted command runs it, if it takes no argument, and
  closes the palette. The command's effect is exactly what the same command
  from the menu or its chord would do, including its undo entry, its sound and
  its status line.
- If the highlighted command takes an argument, Return does not run it: it
  enters argument mode, the same as Tab. A command cannot be run half-said.
- Tab on the highlighted command always enters argument mode when it takes an
  argument. On a command that takes none, Tab does nothing.

### Argument mode

- Entering argument mode replaces the query with a prompt: the command's name
  as a non-editable prefix ending in a colon — `Go to path:` — followed by the
  caret. What is typed from here is the argument, not a query.
- The prompt carries the argument's placeholder as dim text when the argument
  is still empty (`Go to path: ~/Documents`).
- Backspace on an empty argument leaves argument mode and restores the query
  that was typed, with the same command still highlighted. The prefix is never
  edited character by character.
- Escape in argument mode leaves argument mode the same way; a second Escape
  closes the palette. Escape never runs anything.
- Return runs the command with the argument as typed. An argument the command
  rejects — a path that does not exist, an empty name — leaves the palette open
  with the reason shown in place of the list, and the text still there to fix.
- The rows below the prompt become the argument's completions rather than
  commands: directory entries for a path, the choices for a choice argument,
  the sidebar's places for a place argument. Up and Down move through them and
  Return takes the highlighted one.
- Tab in argument mode completes the argument against the highlighted
  completion, extending the text to the longest unambiguous prefix when nothing
  is highlighted — the same completion behaviour the location bar has.
- An argument with a fixed set of choices (a view mode, a sort key, a sidebar
  place) opens argument mode with all of them listed and the current one
  highlighted, so it can be answered with one arrow key and Return.

### Showing an argument as it is typed

- A command may declare that its argument can be *shown* while it is being
  written. Only a command whose effect is something displayed — a selection, a
  filter — may do so; a command that touches files may not, because there is no
  half-typed rename.
- Such an argument is applied after every keystroke, answered afresh from the
  state the palette opened on. Deleting a character therefore widens the answer
  again rather than leaving the last, narrower one standing.
- Abandoning the palette — Escape, a click outside, losing focus — puts back
  exactly what was there when it opened. Running the command keeps the answer.
- The palette says what the argument is doing as it is typed, in the line
  where an error would go: "3 of 61 selected" for a pattern, and "Nothing
  matches" when it picks nothing. That is a note, not an error — a half-typed
  pattern is not wrong — and the next key clears it along with the question it
  answered. The restored state stands underneath either way.

### The first set of commands

Every command below is one the window already carries out by some other means;
the palette adds no new capability.

| Group | Command | Argument |
| --- | --- | --- |
| Go | Go Back, Go Forward, Go Up, Go Home | — |
| Go | Go to Path | path (completed against the filesystem) |
| Go | Go to Place | choice of sidebar places |
| Go | Open | — (acts on the cursor entry) |
| File | Get Info | — |
| File | Rename | new name (pre-filled with the current one, whole) |
| File | New Folder | name, optional; empty means the default name |
| File | Move to Trash | — |
| File | Put Back, Delete Immediately, Empty Trash | — (Trash only) |
| Edit | Cut, Copy, Paste, Select All, Undo | — |
| Edit | Select Matching | glob pattern, shown as it is typed |
| File | Move to Folder | path (folders only) |
| File | New Folder with Selection | name, optional; empty means the default |
| File | Rename N Items | pattern, shown as a dry run while it is typed; two or more selected |
| View | List View, Grid View, Column View | — (the one already on is not offered) |
| View | Change View | choice of the three |
| View | Sort By | choice of sort keys |
| View | Show/Hide Hidden Files | — |
| View | Quick Look | — |

- A command whose preconditions are not met is not offered at all: Paste with
  an empty clipboard, Move to Trash with nothing selected, Put Back outside the
  Trash, Undo with an empty stack.
- A command's title reflects what it would do to the current selection where
  the menus already do so — "Move 3 Items to Trash".

### Renaming a selection from a pattern

- Offered when two or more items are selected, outside the Trash: "Rename 3
  Items". One file has the plain Rename, and a pattern beside it would be two
  renames in the list for one thing. The argument is the new name with holes in
  it, filled from each file in turn; it opens as `{name}` so that Return with
  nothing typed changes nothing.
- The holes: `{name}` and `{ext}` are the original name and extension; `{n}`
  is the file's number in the selection from 1, `{n:3}` padded, `{n@10}`
  started at ten; `{1}`, `{2}`, `{-1}` are the words of the original name
  split at spaces, underscores, dashes and dots, and `{2..}`, `{1..3}`,
  `{..-2}` runs of them; `{name:1..4}` picks characters by position the same
  way. Positions count from one, ranges include both ends, negatives count
  from the back — one rule throughout. A brace that spells none of these is
  left as typed, so a half-written hole reads as what it is.
- A pattern that names no extension — no `{ext}` and no dot in its own text —
  keeps each file's extension: `Holiday {n}` on `IMG_001.jpg` is
  `Holiday 1.jpg`.
- The list under the field is the dry run, one line per file — `IMG_001.jpg →
  Holiday 1.jpg` — with the summary beneath it: "Renames 3 of 3", or the first
  thing wrong. A name already taken, by another file in the batch or by
  something in the folder that is not being renamed, is drawn in red and the
  summary says so; Return then refuses and nothing moves. A name another file
  is *giving up* is not taken: `1 → 2, 2 → 3` is fine, because the renames go
  through temporary names.
- Running it is one undo entry. The dry run's lines are read, not picked:
  the arrows do nothing on them and Return runs the command.

### Where the list scrolls

- At most ten rows' worth of list is on screen; a longer list is a scroll view
  under the field, the same one the columns run on — a touchpad fling carries
  momentum, the rows stretch past either end and spring back, a notched wheel
  steps, and the bar fades in while it moves and out when it stops.
- The keyboard scrolls it by the least that brings the highlight back into
  view, so arrowing through a long list moves one row at a time rather than a
  page.
- The card grows downwards as the list grows, up to that cap: the field stays
  where it is, so what is being typed never moves.

### Accessibility

- The palette is announced when it opens, with the number of commands offered.
- The highlighted row is announced as it changes, with its group and shortcut.
- Entering argument mode announces the prompt; the completion list is announced
  as a list of the same shape the location bar's is.

## Constraints & Edge Cases

- **The provider seam must survive going out of process.** Commands are
  gathered from providers and identified by a stable string id; running one is
  a request carrying that id and the argument as text. Nothing in the seam may
  be a closure over browser state, a borrow, or a Rust type that cannot be
  written down as data — otherwise the later extension system needs the seam
  rebuilt rather than reused. The built-in commands are one provider, with no
  privileges the seam does not give every provider.
- **The seam carries a dry run out and the outcome back, both as data.** A
  provider may answer `preview(request, situation)` with lines and a summary
  for the palette to show as the argument is typed, and `run` answers with an
  *effect*: a status line, and the moves and creations the host records for
  undo. The host applies an effect in one place; nothing a provider does
  reaches the window any other way. Rename is built this way, in-process, as
  the proof — it is exactly the shape a script takes.
- **Scripts live in `~/.config/otto/files-scripts/`** (under
  `$XDG_CONFIG_HOME`; `OTTO_FILES_SCRIPTS` overrides the directory). Every
  executable there is one more provider on the same seam, in another process
  (`scripts.rs`, which documents the wire format). At startup, on a thread of
  its own, each is asked once to `describe` its commands together with the
  conditions they need — how many targets, which extensions, files or folders
  — so that Ctrl+P still touches no disk: the host checks the conditions
  against the situation itself. A command's ids are `scripts:<script>.<id>`.
  A script is asked to `preview` (a dry run, under a deadline of well under a
  second, since it runs under the keyboard) and to `run`, each with the
  request, the targets and the whole situation as JSON on stdin; a run's
  changes come back as data and are recorded for undo exactly as an
  in-process provider's are. A run happens on a worker thread and its effect
  lands through the provider's `poll` on the browser's idle tick, so a slow
  script never holds the window; the status line says it is running
  meanwhile. A non-zero exit is a failure, and the last line of stderr is the
  message shown. Scripts speak the window's language: every text in a
  description may be given per locale and the host picks (exact tag, then
  language, then English), keywords in every language match, and the locale
  is handed to the script on every call (`OTTO_LOCALE`, and `locale` in the
  request) for what it says back. A dry-run line may carry only a name, for
  a command whose outcome is one thing (an archive) rather than one per
  target; and when a text argument opens on an initial value with an
  extension, only the stem is selected, so typing replaces the name and
  keeps the suffix. The two shipped samples,
  `components/otto-files/scripts/zip` and `unzip`, are the proof of the
  boundary and the template for the next ones (conversion, OCR).
- **A resize is claimed after its buffer, never before.** The style
  protocol applies a size the moment the request arrives, so telling the
  compositor a new size while the old buffer is still attached has it draw
  the old pixels stretched into the new bounds until the paint lands — the
  card visibly stretches as the list grows and shrinks under typing. A pane
  surface therefore holds a resize as a pending claim and sends the size and
  position right behind the paint that carries the matching buffer, in the
  same flush; a move alone is claimed at once.
- **A dry run's lines can be toggled.** Down from the field moves the
  highlight into the dry run, Space leaves the highlighted file out of the
  run or brings it back, and a click on a line does the same; a line left
  out keeps its name, struck through behind a hollow mark, so it can be
  brought back. The dry run is made again without it — numbers close up,
  conflicts clear — and Return runs on what is left in. Leaving every file
  out leaves nothing: the file under the cursor is not put back in its
  place, and a command that needs files refuses to run. Typing returns to
  the field, where a space is a space, and what was toggled out stays out.
  The provider never learns of the toggle: it is asked about a smaller
  selection, and the host threads its lines back among the names left out.
- **The right-click menu offers the providers' commands too.** After the
  window's own items, the menu asks the same registry the palette does and
  lists whatever applies to the selection — scripts, the pattern rename — so
  a script is never palette-only. A command with an argument is shown with
  an ellipsis and opens the palette in its field, initial value and dry run
  included; one without runs at once.
- **Availability is computed against a snapshot, not the live browser.** A
  provider is asked for its commands with a description of the situation —
  where the window is, what is selected, whether it is the Trash, what the
  clipboard holds, what the undo stack holds — so a provider that does not live
  in the process can answer the same question.
- **Quick Look is the window's, not the browser's.** Its panel and its decode
  belong to the shell around the listing, so that one command is handed back to
  the host to carry out rather than run where the others are.
- **Gathering commands must not do I/O.** The palette opens on a keystroke; a
  provider that needs to look at the disk offers what it already knows.
  Argument completion may touch the disk, and does so the way the location bar
  already does.
- **Gathering the selection into a new folder is one operation.** The folder
  and the moves into it take a single undo entry, recorded so that taking it
  back walks them in the right order: the files come out first, and the folder
  — empty again — goes last. If nothing could be moved in, the folder is
  removed rather than left behind as the only trace of a command that failed.
  It is offered in the context menu as well as the palette; from the menu the
  folder takes the default name and lands in rename.
- **Selecting by pattern matches the listing on screen**, not the directory:
  hidden files stay out of it unless they are being shown, and a filtered
  listing narrows what can be picked. Matching ignores case until the pattern
  itself carries case — `*.png` finds `PHOTO.PNG`, `*.PNG` means only the
  shouty one — which is the rule the file picker's own filters read by.
- **The palette is not modal over the compositor**, only over its window: the
  window can still be moved, resized and closed while it is up, and closing the
  window closes the palette with it. It is drawn over a shadow rather than over
  a dimmed window, because a dim is how this window says *modal* and the
  palette is not.
- **The card is a subsurface, not a popup.** Both escape the window; only one
  can be dragged. A popup's position belongs to the compositor — it is moved by
  handing back a new positioner and waiting for the answer, a round trip per
  motion event, and the compositor's constraint adjustment may put it somewhere
  other than where it was dropped. A subsurface's position is the client's, so
  the card lands where it is put. A subsurface also takes no keyboard focus,
  which is what lets the palette's keys go on arriving at the toplevel exactly
  as they did when it was painted into the window.
- **The pointer is taken on a surface that does not move.** Pointer positions
  arrive relative to the surface under the pointer, so a surface that is being
  dragged is a moving ruler: each motion event re-applies a correction that is
  already in flight, and the card runs away from the hand. The card therefore
  takes no pointer input of its own. An invisible, input-only surface the size
  of the display sits over it while the palette is up and never moves; every
  press and every drag is measured against that, and the card follows exactly.
  A press on it outside the card is the click that dismisses the palette.
- **How far it may be dragged has to be asked for.** A client is never told
  where its own window sits, so the display's edges are not knowable from
  inside; the compositor is asked once per opening, and until it answers the
  window's own edges stand in — the old limit, wrong only in being too strict.
  The answer is relative to the window and so means nothing once the window
  moves, which for the length of one palette session it does not.
- **The material belongs to the compositor.** On its own surface the card is
  frosted: the compositor blurs and tints what is actually behind the *window*,
  and casts the shadow outside the card's bounds. Neither is possible in the
  window's own buffer, where a blur can only sample the listing the card is
  already covering — which is why the card read as the same colour on the same
  colour, and why a hairline was needed to say where its edge was. The hairline
  stays, over the frost rather than in place of it.
- **The card's corners follow the desktop's rounded-corners setting.** Square
  when the desktop squares its chrome, with the fill, the hairline, the painted
  shadow and the compositor's clip all agreeing. The card is kept between
  openings, so a change made while the palette is closed shows on the next
  opening; one made while it is open waits for the next opening too.
- **Under a compositor without Otto's surface style, the card is not
  frosted, even where a standard blur protocol is available.** The card is a
  subsurface of the palette's own window, and a standard blur protocol blurs
  what is behind the *window*, not behind one of its subsurfaces — turning it
  on here would let the window's own listing show sharp through a
  translucent card. The card instead draws the theme's solid popup material,
  which is the same opaque colour whether the compositor offers no blur
  protocol at all or offers the standard one and simply cannot be asked to
  blur behind a subsurface. Otto's own windows are unaffected: this is the
  fallback path only.
- **A click outside the card closes the palette and stops there.** It must not
  also select whatever file was underneath: the click that dismisses something
  is spent on dismissing it.
- **A command that opens something modal** — Get Info, Rename, the delete
  confirmation — closes the palette first, so the two are never stacked.
- **Rename from the palette** always asks for the name, pre-filled with the one
  the file has and selected whole, and renames directly rather than opening the
  in-place field. It takes the same undo entry the in-place rename does — both
  go through one place, so they cannot drift apart. A name left unchanged is
  not a rename.
- **New Folder given a name** creates exactly that folder, and fails if the
  name is taken — unlike the toolbar's New Folder, which picks a free name
  itself. Quietly creating "reports 2" would be answering a different question
  from the one that was asked. Given no name it falls back to the toolbar's
  behaviour, default name and in-place rename included.
- **Ctrl+P is free in the browser today.** It must remain unbound in the
  picker, where the palette does not exist.
- Localisation: titles, keywords and prompts are catalogue strings. Matching
  runs against the localised title, and against the catalogue's keywords for
  that locale.

## Rationale

- **Tab rather than Space to take an argument.** A space is a legal character
  in the middle of a fuzzy query — "move to trash" is typed with spaces — so
  space cannot also mean "commit to this command". Tab is already the
  completion key in the location bar, and means the same thing here: take what
  is offered and carry on typing.
- **Return does not run an argument-taking command.** Running "Go to Path" with
  no path is either an error or a silent no-op, and both are worse than simply
  landing in the field that was going to be needed anyway. Making Return and
  Tab agree here means a user who only ever presses Return still gets the right
  behaviour.
- **A non-editable prompt prefix rather than an inline `>` syntax.** The prefix
  is generated from the command, so it can be localised, can say what kind of
  answer is wanted, and cannot be corrupted by editing. It also keeps the query
  recoverable: Backspace out of the argument and the search is exactly as it
  was.
- **A surface rather than a clamp.** The card was held inside the window
  because it was painted there, and the clamp was presented as a courtesy —
  keeping the panel findable — when it was really the shape of the buffer
  showing through. Giving the card its own surface removes the reason, and the
  courtesy survives on its own terms as a clamp to the display.
- **Grouped when resting, flat when searching.** The resting list is a map of
  what the window can do and is read by eye; a ranked list is read by position
  and headings only get in its way.
- **Unavailable commands are hidden, not greyed.** A greyed row is a row the
  ranking still has to place and the arrow keys still have to skip. The palette
  is a fast path, not a reference.
- **Data-shaped providers from the start.** The extension system is not
  designed yet, but the shape it needs from the palette is knowable now: string
  ids, a situation snapshot, text arguments. Building the built-in commands
  through that seam is what keeps the second provider from being a rewrite.

## Open Questions

- Should the palette remember the last command run and pre-highlight it on the
  next open? Useful for repeats, but it makes the same keystrokes do different
  things on different days.
- Should a path argument accept a bare fragment and search for it, rather than
  requiring a path that resolves? That edges into file search, which is a
  non-goal here.
- Where does a future provider's command go in the group order, and can a
  provider name a group of its own?
