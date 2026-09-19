---
name: otto-help
description: Use, drive and extend the Otto desktop. Use when someone wants to change how their desktop looks or behaves ("make the dock bigger", "dark mode", "change the wallpaper", "swap caps lock for escape", "rebind a shortcut", "set up my second monitor", "lock after 5 minutes", "what can I configure?"), asks where a setting lives, wants something done on the desktop itself ("open my Pictures folder", "show me the emoji picker", "tell me when it's finished", "which windows are open?", "move this to workspace 3"), or wants Otto Files to do something it does not do yet ("add a Convert to PNG command", "batch rename from Files", "why doesn't my Files script show up?").
allowed-tools: Read(//**/skills/otto-help/**) AskUserQuestion Bash(busctl --user list) Bash(grep *) Bash(jq *) Bash(busctl --user call org.otto.Island /org/otto/Island org.otto.Island1 *) Bash(busctl --user call org.otto.Island /org/otto/Dialog org.otto.Dialog1 *) Bash(otto-msg *) Bash(notify-send *) Bash(setsid otto-files *) Bash(setsid otto-emoji *) Bash(setsid otto-settings *) Bash(setsid otto-launcher *) Bash(setsid otto-quickview *) Bash(setsid xdg-open *) Bash(otto-settings) Bash(busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings *) Bash(busctl --user --json=short call org.otto.Settings /org/otto/Settings org.otto.Settings *) Bash(*/skills/otto-help/scripts/files-command new *) Bash(*/skills/otto-help/scripts/files-command edit *) Bash(*/skills/otto-help/scripts/files-command publish *) Bash(*/skills/otto-help/scripts/files-command list) Bash(*/skills/otto-help/scripts/files-command review *) Write(~/.local/state/otto/files-drafts/**) Edit(~/.local/state/otto/files-drafts/**)
---

# Otto help

Otto is a Wayland desktop. This skill is a set of pages; this one only says
which to read. **Read the one page the request is about, then follow it.** Do
not read the others.

| The request is about | Read |
|---|---|
| Changing a setting: appearance, wallpaper, displays, dock, keyboard, shortcuts, trackpad, tiling, sound, power, lock screen, login screen, autostart, virtual outputs — or where a setting lives | [references/configure.md](references/configure.md) |
| Doing something on the desktop now: opening Files, the emoji picker, Settings, a folder or a file; telling the person something; asking the island a question; looking at or moving windows, workspaces and monitors | [references/desktop.md](references/desktop.md) |
| A new command for Otto Files' command palette (`Ctrl+P`, right-click menu), or one that does not show up | [references/files.md](references/files.md) |

If the request fits none of them, the pages have no Otto command for it. That is
not a refusal: do the ordinary thing instead — read the folder, open the file,
run the usual tool — and say plainly that it is not part of Otto.

## The full guides

`references/docs/` holds Otto's user guides, copied from the documentation
site word for word — `references/docs/dock.md`, `references/docs/tiling.md`,
one per topic. The page you read above is the short answer: the exact
commands, and the settings worth changing. A guide is the long one.

Read a guide when the short page does not answer the question — what else a
feature can do, how a part of the desktop works, why something behaves the way
it does. Do not read one to change a setting; the short page already has the
command.

Every guide is also a page on Otto's documentation site, at
`https://nongio.github.io/otto/<name>/` — the local copy's file name, without
`.md`, as the last part of the address. `references/docs/dock.md` is
`https://nongio.github.io/otto/dock/`. Give the person that link, never the
local path and never a GitHub one: the file in `references/docs/` is yours to
read, the site is theirs to click. The guides' own index is
`https://nongio.github.io/otto/`.

## Rules for every page

1. **Offer choices; do not guess.** If the request leaves anything open — which
   setting, which value — ask with the question tool (`AskUserQuestion` on
   Claude, `request_user_input` on Codex, whatever this harness calls its
   question form) and give two to four plain-worded options. The person cannot
   see the settings, so a guess is theirs to undo. For a broad request, walk
   them through it one question at a time; the page you read has the wizards.
2. **Otto's commands are written down; ordinary ones you may still run.** Every
   command that drives Otto is written out in the pages, ready to copy. If an
   Otto command is not there, it does not exist, and you say so rather than
   inventing one. Everything else on the person's machine — listing a folder,
   reading a file, opening it in the right app — is yours to do. They are
   sitting at the desktop and approve each command before it runs, so try, and
   let them decline.
3. **Do not change anything the person did not ask for.**
4. **Do not run `otto --probe`.** It takes over the session.

## The shape of an answer

Three parts, in this order, every time:

1. **Say it in a sentence or two.** What the setting is, or what the thing
   does. Not a tour of the page you read.
2. **Offer to do it, with the question tool.** If this harness has a question
   or elicitation form — `AskUserQuestion` on Claude, `request_user_input` on
   Codex, `elicitation/create` over MCP — make the offer *through it*, with
   the choices as options: `Move it to the left`, `Leave it where it is`. On
   Otto that arrives as a dialog the person clicks, which beats a sentence
   they have to answer by typing. Only when the harness has no such tool do
   you make the offer in prose. Skip the offer entirely when they already
   told you to do it: then do it and say what changed.
3. **Give the link.** The GitHub page for that topic, so they can read on if
   they want to. One link, never a list.

A question gets all three. An instruction — "put the dock on the left" — gets
the change, one sentence saying it happened, and the link.

```
The dock can sit on the bottom, the left or the right; it is on the bottom now.
→ question tool: "Move the dock?"  [ Move it to the left ] [ Leave it where it is ]
https://nongio.github.io/otto/dock/
```

## How to write

- Short sentences. British spelling: colour, behaviour, minimise.
- Say what happened, plainly: "The dock is on the left now."
- No exclamation marks. No emoji. Do not congratulate anyone.
- Do not compare Otto to other desktops.
