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
query it starts with, so a binding can open it already narrowed. In ask and
agents mode the card is centred on the output instead, and stays centred as the
conversation above the field or the list of sessions under it grows. The card
clips its panes, so neither log nor rows draw past its edge.

**The agent's material.** An agent with a `colour` in `agents.toml` has a
frosted material of its own, and the card wears it whenever it is that agent's:
the composer before the first request (following the agent picked in the list,
or the default), and the conversation once there is one, including a session
opened from the list once the service says whose it is. The list of sessions,
an agent without a colour, and the launcher's other modes keep the plain
material. Only the frost colour changes: the text is coloured from the scheme
as before, and reads the same on every material.

**The agent's mode.** An agent has modes of its own — Claude's Manual, Accept
edits, Plan and Auto, Codex's read-only, agent and full access — and the
service publishes them in the session's `_meta` as `otto.modes`. Once a
conversation is open and the agent has said, the last line of the log names
the agent and the mode it is in, under the status: the agent as a handle,
"@Claude", then the mode in a pill, because the mode is the one thing on that
line that can be changed and so the one thing wearing a control's shape, then
"(Shift+Tab to switch)" when there is more than one. Shift+Tab steps
to the next mode in the agent's order, round the end, and sends `setMode`; the
line changes when the agent has switched, as the service reports it, not
before, and a refused switch leaves it as it was. While the rows under the
field are answers to a question, Shift+Tab walks them instead. An agent
without modes shows no line. The mode is only known once the session opens,
so the composer's agent list carries none.

**Session status.** In agents mode each session row has a small dot where an
icon would go: the theme's accent while the session is working, its yellow
while it waits for input, and its faint text colour (`text_tertiary`) when it
is idle or stopped (a failed session included — the subtitle says it failed).
The colours come from the theme, so the dot follows the user's accent and the
colour scheme.

**In and out of a session.** With nothing typed, Left leaves a conversation
for the list of sessions, and Right opens the highlighted session again; Enter
opens it too. The two arrows walk the same step in both directions, so the
list is never a dead end. Ctrl+L or Cmd+L is the same step without the empty
field: from a request being written or a conversation it shows the list, and
from the list it starts a fresh request, whatever is typed. Neither key is
taken while a request is still on its way to the service, which leaving would
lose.

**Choosing the agent.** The empty field names the agent the request would go
to — "Ask @Otto…" — from the start: the default agent is the first the service
lists, so it is what the field says until another is picked, and the card
wears that agent's material. Until the agents arrive the field names the mode
instead. The list itself stays out of the way: Down opens it under the field,
on the agent the field names, and Up from the first row puts it away again.
While it is open the field names whoever is highlighted, by arrow key or by
pointer, so the choice is legible before it is made rather than only after.
Return takes the highlighted agent and closes the list without sending, so the
choice is on screen with the list gone; leaving the list without picking
leaves the field naming the agent it named before. The first request settles the
session's agent, and the field goes back to asking for a follow-up. A single
agent is no choice at all, and is never listed, but the field still names
it.

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

**Naming a skill.** In ask mode the field is a request rather than a query, and
the agent it goes to has skills — the desktop's own, and any the person
installed — which the agent service publishes with the agent. A request that
opens with `/` names one: while the first word is being typed and it is the
start of a skill's name, the rest of that name is shown after the
caret in the placeholder's colour, and Tab takes it and adds a space. The
completion is a suggestion and nothing more — the request is sent as it reads,
and an agent that makes nothing of the name still gets the words after it. It
fires on the first word only, so an ordinary sentence never sprouts grey text,
and where one skill's name is the start of another's the shorter one is
offered, since the longer is a keystroke further on.

**Allowing a tool.** When the agent asks permission to use a tool, the log
shows who wants to do what and the tool call itself, and under it what the
call would touch as the service reported it: the file, and an edit to it as
`-` and `+` lines, cut to a dozen lines with an ellipsis. The rows under the
field are the agent's options in the agent's order, starting on the one the
service picked (the narrowest allow, or a refusal when the agent asked for no
to be the default); only when the service names none does the launcher fall
back to the narrowest allow itself. Cancelling the turn withdraws the
question rather than refusing it.

**Answering the agent.** When the agent asks the person something — a choice,
a value, a link to open (an input request in its turn) — and no permission
question comes first, the launcher asks the request's questions one at a time.
The log shows the request's message and link, the answers given so far as
"question: answer" lines, "Question n of m" when there are several, and the
question at hand in full, with why the last answer was not taken under it. The
rows are the answers: the options of a single- or multi-select question (a
suggested one says so, and starts selected), Yes and No for a boolean,
Continue for text, numbers and multi-select, Skip this one for a question that
is not required, and Don't answer on every question, which declines the whole
request. Typing answers a text or number question, and stands in for an option
when the question takes an answer of its own; numbers and lengths are checked
before anything is sent, and Enter on an empty field of an optional question
skips it. Picking in a multi-select question ticks and unticks. Left with
nothing typed goes back a question.

Every answer is shared as it is given (`chat/inputAnswerChanged`) — drafts
while ticking, submitted or skipped once a question is settled — and the last
one sends the request (`chat/inputCompleted`) with every answer, unless a
required question further up is still open, which the launcher returns to. A
request with no questions offers Open the link (web links only; any other
scheme is shown and never opened) and Done. Answers given elsewhere show here
as they arrive, and a request settled on another client or refused by the
service falls back to what the chat says. Once settled, a request collapses to
its answers, or to Declined, Dismissed or Not answered.

**Reading an answer.** The agent's answer in the log is Markdown and is drawn
as a document: headings, emphasis, lists, quotes, code and links take the
toolkit's document typography, and the markup itself is not shown. Each
request sits at the right of the log in a rounded gray bubble, in regular
weight and the theme's text colour, wrapped inside the bubble and no wider
than its words need. Everything else — attached files, tool calls, notes, the
status — stays plain text, and is set smaller than the conversation as well as
dimmer, so what was asked and answered outranks the trace of how. Code, in a
fenced block or inline, is set at the size of the prose around it: a monospaced
face is enough to say it is code without shrinking it.

**The tool calls under an answer.** A request's tool calls are one thing in the
log, not a list: closed, the group is the last call and an ellipsis, which is
the call the agent is on. Pointing at it fills it faintly and turns the pointer
to a hand — painted text says nothing about being clickable on its own — and
clicking opens the group to every call in order; clicking again closes it. A
request with a single call shows that call plainly, with nothing to open. Which
groups are open belongs to the conversation on screen and goes when it does. An answer still arriving is drawn as far as it has come, so
an unclosed code fence reads as code until its end lands. Tables are drawn as
code, and an image written in the Markdown itself is drawn as its alt text, as
in Peek.

**Pictures the agent sends.** An agent can answer with a picture rather than a
description of one — a screenshot it read, an image a tool returned — and the log
draws it. A picture
sits where the agent sent it, between what was said before it and what comes
after, so a diagram stays with the paragraph that introduces it. It is drawn at
the left of the log, as wide as the log and no taller than 320 points, keeping
its own proportions and never enlarged past its own size; it wears the card's
corner radius and a hairline edge, so a picture the colour of the card still
reads as one. A picture has no words in it: the selection passes over it, and a
screen reader is told its name. A picture whose file cannot be read — the
service keeps them in a cache it trims — is its name in the log's dimmed text
instead, so the answer still says something was there.

**Selecting what the log says.** The conversation can be picked up and copied.
Pressing on a word in the log starts a selection and dragging extends it,
across lines and through an answer's own formatting; a second press takes the
word under it, a third the line. The selection is painted as the accent behind
the words, faint enough to read through. Ctrl+C or Cmd+C copies it — before the
same key means stop the turn, because a selection on screen says which was
meant — and Ctrl+A with nothing typed selects the whole log. Escape puts the
selection down before it closes the launcher. Copied text keeps the log's line
breaks, and the gaps a layout leaves inside a line, such as after a list's
bullet, come out as spaces. The card is still moved by dragging the field or
the log's background; its words are text now, not a handle.

**Opening a link.** Agents mostly write a URL bare rather than as `[a
link](…)`, so the log treats a bare `https://`, `http://` or `www.` address as
a link like any other: it is drawn as one, the pointer is a hand over it, and
a click opens it in the browser. The sentence's own punctuation is not part of
the address — a trailing full stop, or the bracket around `(https://…)`, stays
text. A link's words are still words: a press on one starts a selection as
anywhere else, and only a release in the same spot opens it, so a link caught
in the middle of a drag is selected rather than followed.

**Copying a code block.** A code block in an answer is usually there to be
run, so it can be taken whole without selecting it. While the pointer is over
a block, a copy button sits in its top-right corner, over the code, and the
pointer is a hand on it; a press puts the block's lines on the clipboard, with
their own line breaks, and the button shows a tick until the pointer leaves
the block. The button is only there on hover: one on every block would make
an answer full of snippets look like a toolbar. A press on it does not start
a selection, and leaves any selection alone.

The log is painted rather than laid out as widgets, so what can be selected is
described separately: one box per run of text, in reading order, rebuilt
whenever the log is laid out again. Those boxes have to agree with what was
painted — the same fonts, the same positions — or the highlight sits off the
words. An answer grows at its end, so a selection made while it is still
arriving survives the rest of it; one whose text has since been laid out
differently is dropped rather than left highlighting whatever now sits there.

**Choosing.** Up/Down move the selection and wrap at both ends. Tab takes the
completion when one is being offered; otherwise Tab and Shift+Tab move the
selection. Page Up/Page Down move by a screenful. Ctrl+N and Ctrl+P
mirror Down and Up. The list scrolls to keep the selection visible; at most
eight rows are shown at once. Enter acts on the selection. Escape closes the
launcher without acting.

**Editing.** The field supports caret movement, selection with Shift, word-wise
motion with Ctrl, select-all with Ctrl+A, deleting the previous word with
Ctrl+W, and clearing the query with Ctrl+U. Ctrl+C, Ctrl+X and Ctrl+V copy, cut
and paste against the system clipboard; a paste keeps only what fits on one
line, and refilters as typing does.

**Pointer.** Over the log's words the pointer is a text cursor, because
nothing else about painted text says it can be picked up. Moving the pointer
over a row selects it. Releasing over a row acts
on it. The row under the pointer must be the row that highlights. A wheel or a
touchpad over the card scrolls the list — a touchpad with momentum and a
stretch past either end, a notched wheel a step at a time — and the row that
comes under the pointer is selected.

The rows scroll on a surface of their own inside the card, and the selection's
highlight slides on another beneath them: scrolling the list or moving the
selection repaints neither the card nor the rows.

**Painting.** The full-output parent surface draws nothing and is painted once
per configure, never per frame: a commit of it tells the compositor the whole
screen changed, and the dock and the bar were re-blurred under a card that never
touches them. A card paint reports only the part of the buffer that changed — the
field for a caret blink or a keystroke — and a pass with nothing changed commits
no frame at all.

The launcher takes pointer input over the card that is drawn — the query field,
plus however many result rows are showing — and nowhere else. Its shadow is not
part of it, and neither is the rest of the output: the pointer over the dock, a
window, or the desktop is the pointer over those, and they hover and click as
they would with no launcher up. A press outside the card therefore goes to what
is under it rather than to the launcher, and the launcher closes because the
keyboard moves on with the press. Escape closes it wherever the pointer is.

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
- **What is drawn and what takes input must be stated separately.** Neither
  surface's geometry describes the card: the card's buffer is allocated at its
  tallest whatever is on it, and the parent surface is anchored to all four
  edges of the output. Both must therefore carry an input region of the card as
  drawn, updated as the list grows and shrinks and again when it empties back
  to the field alone. Otto treats a layer surface's own input region as the
  clickable area of everything beneath it, so a parent that sets none makes the
  whole output the launcher's: the dock under it stops answering the pointer.
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
  as the answer is taken. Something that outlives it has to own the offer —
  `wl-copy`, which forks and stays to serve it. Everything the launcher copies,
  from the log or from the field, is offered here *and* handed to `wl-copy`: the
  first is what makes a paste work while the launcher is still up, the second is
  what makes it work afterwards. Without `wl-copy` installed, a copy lasts only
  as long as the launcher does.
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
  height without reallocating or re-laying-out anything. Nothing about that
  buffer says how tall the card is, though, which is why the shape is stated
  separately as an input region — see the constraint below.
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
- **Names for the agent modes.** Ask mode (`--ask`) is **Ask**, the only new
  product name. The list of agent tasks (`--agents`) is **Sessions**, the word
  people who use agents already use; public text says "agent sessions" on first
  mention, so it is not read as the login session. The flag stays `--agents`
  because `--sessions` is one letter from `--session ID`. The launcher itself
  stays a lower-case noun: one place with several modes, not a product. The
  service behind both is "the agent service" in text, and its binary is
  `otto-agents`, a user service. `otto-ask` is an alias for `--ask`: a symlink
  the launcher recognises by the name it was started under. `--agents` has no
  alias — that name belongs to the service. In Files, the command palette's
  command is **Ask…**; in the islands, a **permission request** has no name.

## Open Questions

- Should the launcher be bound to a key by default, and to which one?
- Should a query that matches nothing offer to run it as a command?
- Should the arithmetic answer offer a second action — opening the expression in
  the calculator application — and on what key?
- Files as a third provider: what is searched, and how is the result acted on?
