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

- Ctrl+P opens the palette. It opens over the browser window, just below the
  header and near its right edge, and takes the keyboard whole while it is up.
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
- A partial argument that matches nothing leaves the restored state standing
  and says nothing: it is half-typed, not wrong.

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

### Where the list scrolls

- At most ten rows are on screen. A longer list scrolls under the highlight, by
  the least that brings the highlight back into view — so arrowing through it
  moves one row at a time rather than a page.
- The card grows downwards as the list grows: the field stays where it is, so
  what is being typed never moves.

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
