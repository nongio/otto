# Sorting the desktop

For "tidy my windows", "sort this lot out", "put these somewhere sensible",
"make me a music room" — anything that moves the person's windows around to
put the desktop in order.

Look, plan, **ask**, then open exposé, move things while they watch, and close
exposé when the last window has landed. Do not touch a window before they have
agreed to the plan.

## Step 1 — look, before anything else

Nothing can be planned or asked until you know what is open and where it is.

```sh
otto-msg -t get_workspaces -r | jq -r '.[] | "\(.num)\t\(.name)\(if .focused then "  <- here" else "" end)"'
```

```sh
otto-msg -t get_tree -r | jq -r '.. | objects
    | select(.layout == "none" and .name != null)
    | "\(.app_id // .window_properties.class // "?")\t\(.name)\(if .focused then "  <- focused" else "" end)"'
```

Read the **titles**, not just the app ids. A title carries the folder, the
file, the page — which is where the grouping actually comes from. Note which
workspace the person is on now: they go back there at the end.

## Step 2 — group them

This is the part that needs judgement rather than a rule. Two ways to cut it:

- **By kind** — terminals together, documents together, browsing together,
  music together. Stable, obvious, and the right default when the windows have
  nothing much to do with each other.
- **By project** — everything whose title shares a folder or a filename root.
  Better when the person is working on one thing across several apps, and much
  better than kind when it fits.

Either way:

- **Several terminals go in one workspace, tiled.** Terminals are the one kind
  that is genuinely better side by side, so when there are three or more, put
  them together and turn tiling on for that workspace. Two is a judgement
  call; one is not a group. This is the only place sorting turns tiling on.
- Keep a pair that works together in the same place — the editor and the
  terminal running its build belong side by side, whatever the other rule says.
- Two to four groups. Five workspaces holding one window each is not tidier
  than one workspace holding five.
- A window that fits nowhere stays where it is. Say so; do not invent a room
  for it.

**Name each group** in one word, in the person's language: Terminals,
Documents, Internet, Music, or the project's own name — Lantern, Plate. Reuse
a workspace that already holds that kind of thing rather than making another.
Do not rename a workspace the person named themselves unless they asked, and
leave workspace 1's name alone — it is usually where they were.

## Step 3 — ask, before touching anything

Put the plan to them **with the question tool** (`AskUserQuestion` on Claude,
`request_user_input` on Codex, `elicitation/create` over MCP), which on Otto
arrives as a dialog they click. One question, the plan as the options:

- `Sort them this way` — and the description names the rooms: "Terminals (3,
  tiled), Documents, Internet".
- `Leave my terminals alone` or whichever single change they are most likely
  to object to.
- `Don't sort anything`.

Moving somebody's windows is not a thing to do and then report. Ask even when
the plan looks obvious; a wrong guess costs them the arrangement they had.

If the harness has no question tool, write the plan out — one line per room —
and wait for a yes.

## Step 4 — open exposé

```sh
otto-msg 'expose show'
```

**After they agree, before you move anything.** Exposé is the window overview:
every window as a preview under the workspace strip, which is exactly what the
person needs on screen to follow what you are about to do. Windows sorted
behind an ordinary desktop just vanish from under them.

It stays open while you work — the grid re-flows as each window leaves and the
strip's thumbnails update, so the sorting is the thing they watch. **Close it
when the last move lands**: `otto-msg 'expose hide'`, the final step of the
plan. Exposé is a view of the sorting, not where the person lives, and leaving
it up makes them dismiss it themselves before they can use the desktop you just
tidied. `Ctrl+Up` is their own way back in if they want another look.

## Step 5 — run the plan, paced

Sent in one go, every move lands within a frame and the windows simply
teleport. Run them through `sort-run` instead, which pauses between steps so
the person can follow each window and stop you:

```sh
<this skill>/scripts/sort-run 0.4 \
    'rename workspace 2 to Terminals' \
    'rename workspace 3 to Documents' \
    '[title="build"] focus; move container to workspace 2' \
    '[title="notes.md"] focus; move container to workspace 2' \
    'workspace 2' \
    'tiling enable' \
    'workspace 1' \
    'expose hide'
```

`<this skill>` is the folder this skill's `SKILL.md` is in; write it as an
absolute path. The first argument is the pause in seconds — **0.4 is right**;
below about 0.2 it is a teleport again, above 1 it drags. Each step prints as
it runs, and a step that fails stops the rest rather than carrying on past the
thing it depended on.

| Command | What it does |
|---|---|
| `rename workspace [<n>] to <name>` | name the focused workspace, or number `<n>`, making it if it is not there. The name is the rest of the command, spaces and all |
| `[app_id="…"] focus` / `[title="…"] focus` | focus that window wherever it is. Case-insensitive substring |
| `move container to workspace <n>` | send the focused window there, making it if needed |
| `workspace <n>` | switch; needed before `tiling enable`, which acts on the workspace you are on |
| `tiling enable` | tile the current workspace — for the terminals room |
| `expose show\|hide\|toggle` | the window overview |

Rename before moving: a number that does not exist yet is created by the
rename, so the rooms are laid out before anything travels.

**Tiling is per workspace**, so `tiling enable` needs `workspace <n>` in front
of it, and a `workspace <back>` after it to return. Put those in the plan, in
that order — that is why `workspace 2`, `tiling enable`, `workspace 1` end the
example.

**Telling several windows of one app apart.** A criteria takes the **first**
match, so `[app_id="ghostty"] focus` finds the same terminal every time —
repeat it and you will move one window and then shuffle it about. Match on
**`title`** instead, one line per terminal, taken from Step 1's listing. If two
windows share an app id *and* a title there is no way to tell them apart:
move what you can, say which one you left, and leave it.

Finish on the workspace they started on, then **close exposé** — those two are
the last two steps of every plan, in that order. The desktop should not end up
somewhere they did not ask to be, nor behind an overview they have to dismiss
themselves.

## Step 6 — check and report

```sh
otto-msg -t get_tree -r | jq -r '.nodes[]?.nodes[]?
  | "\(.name): " + ([.. | objects | select(.app_id? != null) | .app_id] | join(", "))'
```

One line per room, in plain words: "Terminals has your three shells, tiled.
Documents has Files. Internet has the Wikipedia page." Name anything you left
where it was, and why.

## Rules

1. **Ask before you move anything.** Step 3 is not optional, and it comes
   before exposé opens.
2. **Exposé before the first move, closed after the last.** Windows that move
   behind a plain desktop just disappear as far as the person can tell — and
   an overview left up is one more thing for them to clear away.
3. **Pace the moves.** Through `sort-run`, not as one `otto-msg` call.
4. **Never close a window while tidying.** Sorting moves things; it never
   removes them.
5. **Leave what you are unsure about.** An unsorted window is better than one
   filed somewhere wrong.
6. **Do not make a workspace you are not going to fill.** Workspaces are made
   and never destroyed, so an empty one stays on the strip.
7. **Tiling only for the terminals room.** Sorting and tiling are otherwise
   different requests; anywhere else, `tiling enable` only if they asked.
