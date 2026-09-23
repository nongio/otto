# Open With

**Status:** draft  
**Related specs:** [file-browser.md](file-browser.md), [context-menus.md](context-menus.md), [file-command-palette.md](file-command-palette.md)

## Summary

Open With lets the user pick which application opens the selected files, either
just this once or from now on. It is a window of its own, built like Get Info,
listing the applications that can open the files and every other one installed.

## Goals

- Open the selection in an application other than the default.
- Change which application a type opens with, from the place the user is when
  they notice.
- Agree with the rest of Files: the default the chooser shows is the one a
  double-click opens, and a remembered choice is written in the user's
  `mimeapps.list`, where every spec-following desktop component reads it.

## Non-Goals

- Choosing applications for URL schemes, or for folders. A folder opened here is
  entered, not handed to another application.
- Editing associations wholesale (a settings page listing every type).
- Opening items in the Trash.

## Behavior

### Opening the chooser

- **Open With…** is on the context menu whenever the selection is one or more
  files and no folders. It is also a command in the palette, offered on the
  same condition.
- It is not offered in the file picker, which answers one request and opens
  nothing.
- In the Trash it is refused with the message a double-click there gives.
- The chooser opens as a separate, fixed-size window carrying the browser's
  `app_id`, with the browser as its parent. It is not modal: the browser keeps
  working behind it. Closing it — its close dot, the compositor's close,
  Escape, Cancel — opens nothing and changes nothing.

### What it shows

- A header with the file's icon, its name (or "N files"), and what its type is
  called in words, from the shared MIME database, in the interface language when
  the database has it ("PDF document"). Files of more than one type say so
  instead of naming one.
- A search field, focused. Typing narrows the list to applications whose name
  matches, by the same subsequence matching the launcher uses, best match
  first; the highlight moves to the first match.
- The list:
  - The applications that can open every selected file, in order: the type's
    default first, tagged **Default** and highlighted; then the applications
    associated with the type itself; then those associated with the types it
    descends from (a Rust file lists editors that register plain text, after
    those that register Rust).
  - A heading, **Other Apps**, folding every other installed application, by
    name. It starts folded, unless nothing could be suggested, in which case it
    starts open. While a query is typed, matches from both groups show and the
    heading does not fold them away.
  - Handler entries (`NoDisplay=true`) are not listed unless one is the
    default, and hidden or unrunnable entries never are.
- The list scrolls the way every other list in Files does: a touchpad
  flings with momentum and stretches past either end before springing back, a
  notched wheel steps, and the overlay bar fades in while scrolling and can be
  dragged.
- A checkbox, **Always use this app for every ⟨type⟩**, unticked. It is only
  offered when every file shares one type: one "always" cannot set two
  defaults. It is not offered for `application/octet-stream`, which every file
  is underneath.
- **Cancel** and **Open**. Open is disabled while no application is
  highlighted.

### Picking

- Up and Down move the highlight, scrolling it into view. Enter on an
  application opens with it; Enter on the heading folds or unfolds it.
- A click highlights a row; a double-click on an application opens with it. A
  click on the heading folds or unfolds it.
- Opening starts the application on the files and closes the chooser:
  - An `Exec=` line with `%F` or `%U` gets every file in one process; one with
    `%f` or `%u` is started once per file; one with no file code is started
    once, with no files. `%i`, `%c`, `%k` and `%%` expand as the Desktop Entry
    specification says; deprecated codes are dropped.
  - Files are passed by absolute path, as the bytes their names are; `%u` and
    `%U` get `file://` URIs.
  - `Terminal=true` applications are started inside a terminal: `$TERMINAL -e`,
    else `xdg-terminal-exec`, else the first of ghostty, alacritty, foot, kitty
    installed, else xterm. The launcher uses the same choice. `Path=` is the
    working directory when it exists.
  - The applications are detached, in a process group of their own, and
    outlive the browser.
- If the application cannot be started, the chooser stays up and says why, so
  another can be picked.
- With the box ticked, the application becomes the type's default: it is set in
  `[Default Applications]` of the user's `$XDG_CONFIG_HOME/mimeapps.list` and
  moved to the front of the type's `[Added Associations]`, leaving the rest of
  the file as it was. When that file is a symlink, the file it points to is
  rewritten and the link kept. A default that cannot be written does not stop
  the files opening; the status line says the choice was not kept.

## Constraints & Edge Cases

- Associations are resolved by the freedesktop MIME Applications Associations
  specification: `mimeapps.list` files in precedence order (config before data,
  user before system, `⟨desktop⟩-mimeapps.list` before `mimeapps.list`, the
  legacy `defaults.list` last in each data directory); a default that is not
  installed falls through to the next; a removed association hides an
  application from lower-precedence files and from its `MimeType=`.
- Desktop entries are read in precedence order by desktop file ID, so a user's
  copy of an entry — including a `Hidden=true` one — replaces the system's.
  Entries filtered out by `OnlyShowIn`/`NotShowIn`, or whose `TryExec` is not
  installed, are not offered.
- A file's type is judged from its name and its first 4 KB together: a real
  signature in the content wins unless the name only narrows it (a `.docx` is
  a zip); content that is merely text, or matches no signature, defers to the
  name; with neither, the content's guess (`#!` makes a shell script), else
  `application/octet-stream`.
- The installed applications are read when the chooser opens, not kept up to
  date: an application installed while it is up appears the next time.

## Rationale

- A window rather than a submenu: the full list of applications is long, needs
  a search field, and the "remember" choice needs somewhere to live. A submenu
  would offer the suggestions only and push everything else into a second step.
- Remembering is a checkbox on the same window, unticked by default, rather
  than a separate "Change All…" flow in Get Info: the decision is made where
  the user is looking at the choice, and the safe answer — just this once — is
  the one that needs no action.
- The plain Open resolves the default with the same associations and the same
  reading of the file's type as the chooser, rather than going through
  `xdg-open`. Outside a desktop `xdg-open` recognizes, it detects types and
  picks defaults its own way (exact type only, first entry in
  `mimeinfo.cache`), so the app tagged Default here would not be the one a
  double-click started. `xdg-open` is still the fallback for a type nothing
  installed claims, and for URLs.

## Open Questions

- Should a double-click on a file with no associated application bring up the
  chooser, instead of falling back to `xdg-open`?
- The chooser could back `org.freedesktop.impl.portal.AppChooser` in
  xdg-desktop-portal-otto, so sandboxed applications asking to open a file get
  the same window.
