# The Launcher

The launcher is how Otto is driven from the keyboard. A key opens a card over the
desktop, typing narrows a list, and Enter starts an application, brings a window
forward, or copies the answer to a sum. The same card is also Ask, where what is
typed is a request for an agent, and the list of agent sessions. It is an
ordinary Wayland client in a process of its own, started fresh by a key binding
(or by the side canvas, the stash and the agent service) and gone as soon as
something is chosen. There is no daemon and nothing to keep warm.

## The model

Three ideas carry the design.

**One run, one choice.** Each press of the key is a new process. Startup is the
whole cost, so the only work done then is reading the installed desktop entries
and asking the compositor for its windows. The process ends once the choice has
been carried out. What has to outlive a run (the last few apps launched, Ask
drafts not yet sent) goes in small state files, and losing them costs nothing
but convenience.

**Sources, not features.** The launcher knows how to filter and draw a list of
items. Where the items come from, and what picking one means, belongs to a
*source*. A source hands over its items when the launcher opens, can offer a
shorter resting list for the empty field, can answer the query itself, and
carries out the choice. Apps, open windows and the calculator are three sources.
A new kind of result is a new source; the filtering, the layout and the keys
stay as they are.

**Ask is a mode of the same card, not a source.** In Ask the field is a request,
so it filters nothing. The rows are whatever the agent service needs picked
next: the agent to send to, the session to open, the answer to a permission or
input question. The connection to the service is the one piece that is not a
source, because it talks back.

```
           key binding / otto-canvas / otto-stash / agent service
                                   |
                                   v
  +------------------ otto-launcher (one process per run) -----------------+
  |                                                                        |
  |   Apps --------+                                                       |
  |   Calculator --+--> items --> rank(query) --> rows --> card (lay-rs)   |
  |   Windows -----+                               ^                       |
  |                                                |                       |
  |   Ask (thread) --- agents, sessions, questions-+                       |
  +------------------------------------------------------------------------+
        |                |                    |                  |
  desktop entries   foreign-toplevel     agent service      otto-stash
  (XDG dirs)        (2nd Wayland conn)   (socket, AHP)      (D-Bus)
```

## Start to finish: a key press to a running app

1. **The key.** Otto's config binds a key to run `otto-launcher`, and the
   compositor spawns it like any other command. The mode comes from the command
   line: apps by default, or windows, both, Ask, or the list of sessions.
   Started under the name `otto-ask` it opens in Ask.

2. **Sources load.** Before any surface exists, the launcher scans the desktop
   entries in XDG order, keeping the first entry of each id so a user's override
   shadows the system copy. Hidden entries, `NoDisplay` ones and those with
   nothing to run are dropped. In window mode it opens a second Wayland
   connection and does two roundtrips to collect every toplevel the compositor
   announces. From here on nothing on the typing path touches the disk.

3. **Surfaces.** The launcher creates a layer-shell surface on the overlay layer,
   anchored to all four edges, with exclusive keyboard interactivity. That
   surface draws nothing. The card is a subsurface of it, allocated once at its
   tallest, and the compositor supplies its material (frost, corner radius,
   border, shadow) through `otto-surface-style`. Both surfaces carry an input
   region of the card as drawn, so the pointer over the dock or a window still
   reaches them. The card arrives with a quick fade and a scale that springs a
   little past full size.

4. **At rest.** With nothing typed, each source gives its resting list. For apps
   that is the last three launched from the launcher, most recent first, and
   nothing at all before the first launch. Windows list every window, most
   recently focused first, but only in window mode. A query that matches nothing
   says so; an empty field never does.

5. **Typing.** Every keystroke ranks all items again. First come the sources'
   answers to the query itself (a complete arithmetic expression gets its result
   pinned as the first row), then the ranked matches. Ranking is the desktop's
   shared subsequence scorer: every typed character must appear in order, and
   the score favours a match at the start of a word, a run of adjacent
   characters, and a shorter name. Spaces separate words, so `fire dev` finds
   Firefox Developer Edition. The title is what someone aims at. Keywords, the
   program name and a window's app id still find the item, but score lower and
   never outrank a title hit. Ties keep source order. The card's height follows
   the list on a transition the scene engine runs.

6. **Enter.** The selected row's origin names its source and its index there, and
   that source carries out the choice. Apps start the entry detached, in a
   process group of their own, through otto-kit's desktop-entry launcher (field
   codes expanded, terminal apps wrapped in a terminal). Only once it has started
   does the app move to the front of the launch history. Windows ask the
   compositor to unminimise and activate the toplevel. The calculator hands its
   answer to `wl-copy`.

7. **Gone.** On success the card fades and shrinks, and the process exits once
   that has run. If the action fails, the launcher stays up rather than vanishing
   having done nothing.

The launcher also closes when it loses the keyboard after it has had it. That is
how a click outside the card works: the click goes to whatever is under it, the
keyboard follows, and the launcher sees itself lose focus. Losing the keyboard
before ever having it is ignored, since that is the launcher still arriving.

## How Ask plugs in

In Ask and the sessions mode the launcher loads no sources. It opens a
connection to the agent service instead, through the shared Ask client in
`otto-agents-kit`. That client lives on a thread of its own, because the
launcher's loop has no async runtime, and wakes the loop through a socket the
loop polls, the same way the window list does. It connects as the launcher
starts, so the agents and sessions have arrived by the time anyone looks.

Each pass of the loop asks the client what changed and rebuilds the rows from
its state. A permission question the agent is waiting on comes first, then a
request for input, then the session list while one is being picked, then the
agent list while it is open under the field. The log above the field is the
conversation, laid out and painted by code the side canvas shares.

Sending queues the request on the session's chat and returns. The card can then
close at any time; on the way out the process waits a few seconds at most for a
request still on its way to the service, so Escape right after Enter does not
lose it. Leaving a conversation for the list, or starting a fresh request,
drops the connection and opens a new one, and costs the session nothing: the
service owns it. Picking a session that is open in a terminal brings that
terminal forward through the window list instead of opening the conversation,
and a session an app started about one of its files opens in that app.

The protocol, the session model, the terminal handover and where permission
questions go are on [Agents](agents.md#the-launcher-as-a-client). Things stashed
to go with the next request are followed from `otto-stash` over D-Bus. The
launcher lists them and can strike or remove them, but the stash owns them.

## Boundaries

| Across | What crosses | Who owns the state |
|---|---|---|
| Compositor, main connection | `wlr-layer-shell` overlay surface, the card subsurface and its `otto-surface-style` material, keyboard and pointer, input regions | Otto owns focus and the card's material; the launcher owns what is painted on it |
| Compositor, second connection | `wlr-foreign-toplevel-management`: the window list, activate, unminimise | Otto owns the windows; the launcher keeps a copy for one run |
| Agent service | AHP over its Unix socket | The service owns sessions, requests and questions; the launcher shows and answers them |
| `otto-stash` | D-Bus, with stashed items as files in its runtime directory | The stash, until the items are sent |
| State files | Launch history and Ask drafts, each written whole and moved into place | The launcher |
| Child processes | Apps, `wl-copy`, a session's terminal, the sandboxed thumbnail worker (this same binary started again) | Apps, `wl-copy` and the terminal outlive the launcher |

Inside the process there is one loop. The connections that need async (the
agent service, the stash) each run on a helper thread and report back over a
channel. Everything visible is built and changed on the loop.

While the launcher holds the keyboard, Otto still fires the shortcuts that leave
the windows under it alone: the app switcher, volume, brightness, media keys,
the person's own commands, locking. Window and workspace shortcuts do not. That
rule lives in the compositor's keyboard handling, not here.

## Decisions

**No frecency.** The ranking never learns from what was picked. A query that
means different things on different days breaks the habit of typing three
letters and pressing Enter. History only fills the empty field, where the
alternative is a wall of names nobody reads.

**Exclusive keyboard.** The launcher is modal for as long as it is up. For a tool
whose whole interface is typing, a keystroke landing in the window behind it is
the one failure that cannot be allowed.

**The card is its own surface, and the parent draws nothing.** Only the
compositor can blur the desktop, and a surface's material covers the whole
surface, so the frost needs a surface the size of the card. A dim behind the
card, painted by the client, would be exactly what the blur samples, turning the
frost into blurred grey, so there is none. The parent is painted once per
configure and never per frame, because a commit of a full-output surface tells
the compositor the whole screen changed.

**One buffer at full height.** The card's buffer is allocated at its tallest and
the compositor shows less of it, so the card changes height without reallocating
or laying text out again. The cost is that nothing about the buffer says how tall
the card is, which is why the input region is stated separately.

**A second Wayland connection for windows.** otto-kit's runner owns the main
event queue and the protocols it dispatches, and the launcher cannot add a
handler to a type it does not own. Rather than widen the toolkit for one source,
the window list has its own connection, whose socket the loop polls next to its
own.

**Rows on a scroll pane.** The rows and the selection highlight sit on
subsurfaces inside the card that are moved rather than repainted, so scrolling
the list or moving the selection repaints neither the card nor the rows. See
[Scroll Panes](scroll-pane.md).

**Copy through `wl-copy`.** A Wayland selection dies with the client that offered
it, and the launcher exits as soon as an answer is taken. A copy is offered by
the launcher, so a paste works while it is up, and also handed to `wl-copy`,
which stays behind to serve it.

**Rows and the log are shared, not the launcher's.** Rows, list keys, ranking
and the chat log live in `otto-agents-kit` and otto-kit's item list, because the
side canvas and the settings search show the same kinds of thing. A change to
how a session row looks is made there and shows in both places.

## Limits and direction

- Results come from apps, open windows and arithmetic only. Files as a source,
  and whether a query that matches nothing should offer to run it as a command,
  are open questions in the [spec](../../specs/launcher.md#open-questions).
- Whether the launcher gets a default key binding is open too; the example
  config binds Ctrl+Space.
- Without the foreign-toplevel protocol the launcher runs with apps only. On
  another compositor the card's material and placement depend on
  `otto-surface-style`, which only Otto offers.
- Theming follows the colour scheme, the accent, an agent's own colour and the
  icon theme. There is no theme file, by design.

## See also

- [Agents](agents.md): the agent service, AHP, and the launcher as its client.
- [Side Canvas](side-canvas.md): the other agent UI, sharing the launcher's rows and log.
- [otto-agents-kit](../../components/otto-agents-kit/AGENTS.md) and [otto-kit](otto-kit.md): the shared pieces.
- [Surface Style Protocol](surface-style-protocol.md), [Scroll Panes](scroll-pane.md),
  [Foreign Toplevel](foreign-toplevel.md), [Accessibility](accessibility.md).
- Specs: [Launcher](../../specs/launcher.md), [Stash](../../specs/stash.md),
  [Dictation](../../specs/dictation.md).
- User guide: [Launcher](../user/launcher.md).
- Working notes for contributors: [AGENTS.md](../../components/otto-launcher/AGENTS.md).
