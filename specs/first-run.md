# First Run

**Status:** draft  
**Related specs:** settings-app.md, dock-places.md, login-mode.md

## Summary

The first time a user starts an Otto session, Otto writes them a configuration that fits the system it finds: the icon theme their other desktops use, and a dock of the applications that are installed. After that, the configuration belongs to the user and Otto never guesses again.

## Goals

- A new user's first session has a complete icon theme, not `hicolor` alone.
- A new user's dock holds a file browser, settings, a terminal, a web browser, a text editor and a calculator, as far as the system has them.
- The choices are written to the user's configuration file, where Settings shows them and the user can change them.
- It happens exactly once per user.

## Non-Goals

- Following later changes made in another desktop. Once written, the values are the user's.
- Installing anything. Only apps and themes already on the system are used.
- Keyboard shortcuts. They come from the system configuration.

## Behavior

- **Trigger:** a session starts and the user's configuration file (`$XDG_CONFIG_HOME/otto/config.toml`) does not exist. A greeter session and a test session never trigger it.
- **Result:** the file is created with `icon_theme` and `dock.bookmarks`. A value is left out when nothing suitable is found, so the system configuration's value applies instead.
- **Icon theme:** the first installed theme found in:
  1. the running desktop's own setting, when started from inside another desktop;
  2. a theme the user picked in any desktop: Plasma, GNOME, Cinnamon, MATE, Xfce, GTK's `settings.ini`, qt6ct, qt5ct;
  3. the distribution's defaults: GSettings overrides, then system-wide Plasma, Xfce and GTK settings;
  4. Breeze, then Adwaita.
- **Dock:** one app per role, in this order: files, settings, terminal, browser, text editor, calculator. For each role, the first of:
  1. Otto's own app (Files, Settings), when installed;
  2. the user's default handler for the role's MIME type (`inode/directory`, `x-scheme-handler/terminal`, `x-scheme-handler/https`, `text/plain`);
  3. a well-known app for the role;
  4. any app in the role's freedesktop menu category (`FileManager`, `TerminalEmulator`, `WebBrowser`, `TextEditor`, `Calculator`).
  
  An app is eligible only when it opens its own window and is meant to show in menus (not `NoDisplay`, `Hidden` or `Terminal=true`), and is not restricted to other desktops through `OnlyShowIn`. A role nothing fills is left out.
- **Failure:** if the file cannot be written, the session starts on the system configuration and a warning is logged. The next session tries again.

## Constraints & Edge Cases

- It runs before anything reads the configuration, so the first session already uses the result.
- A user who deletes their configuration file gets a new first run.
- The dock written here replaces the system configuration's bookmarks for that user. Later changes to the system configuration's dock don't reach them.

## Rationale

- No freedesktop standard records the user's icon theme, so each desktop's own settings are read. Distribution defaults are read from the files distributions use to brand their desktops, rather than from a per-distribution table.
- Guessing once and writing the result keeps behaviour predictable. A theme that changes whenever another desktop's settings change would be confusing, and Settings would have nothing to show.
- MIME defaults come before well-known apps, so a user who already chose a browser or editor elsewhere gets that one.

## Open Questions

- Should first run also offer a short welcome, or suggest the launcher shortcut?
