---
name: otto
description: Use and extend the Otto desktop. Use when someone wants to change how their desktop looks or behaves ("make the dock bigger", "dark mode", "change the wallpaper", "swap caps lock for escape", "rebind a shortcut", "set up my second monitor", "lock after 5 minutes", "what can I configure?"), asks where a setting lives, or wants Otto Files to do something it does not do yet ("add a Convert to PNG command", "batch rename from Files", "why doesn't my Files script show up?").
allowed-tools: Read(//**/skills/otto/**) Bash(busctl --user list) Bash(busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings *) Bash(busctl --user --json=short call org.otto.Settings /org/otto/Settings org.otto.Settings *) Bash(*/skills/otto/scripts/files-command new *) Bash(*/skills/otto/scripts/files-command edit *) Bash(*/skills/otto/scripts/files-command publish *) Bash(*/skills/otto/scripts/files-command list) Bash(*/skills/otto/scripts/files-command review *) Write(~/.local/state/otto/files-drafts/**) Edit(~/.local/state/otto/files-drafts/**)
---

# Otto

Otto is a Wayland desktop. This skill is a set of pages; this one only says
which to read. **Read the one page the request is about, then follow it.** Do
not read the others.

| The request is about | Read |
|---|---|
| Changing a setting: appearance, wallpaper, displays, dock, keyboard, shortcuts, trackpad, tiling, sound, power, lock screen, login screen, autostart, virtual outputs — or where a setting lives | [references/configure.md](references/configure.md) |
| A new command for Otto Files' command palette (`Ctrl+P`, right-click menu), or one that does not show up | [references/files.md](references/files.md) |

If the request fits neither, this skill does not cover it. Say so rather than
guessing at Otto's commands.

## Rules for every page

1. **Do not improvise commands.** Every command you need is written out in the
   pages, ready to copy. If it is not there, it does not exist.
2. **Do not change anything the person did not ask for.**
3. **Do not run `otto --probe`.** It takes over the session.

## How to write

- Short sentences. British spelling: colour, behaviour, minimise.
- Say what happened, plainly: "The dock is on the left now."
- No exclamation marks. No emoji. Do not congratulate anyone.
- Do not compare Otto to other desktops.
