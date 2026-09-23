# First Run

**Status:** draft  
**Related specs:** settings-app.md, dock-places.md, login-mode.md

## Summary

A new user's first session starts with an icon theme that matches their other desktops, and a dock that already holds the everyday apps. The icon theme is detected once and written to the user's configuration. The dock comes from a fixed list in the system configuration.

## Goals

- A new user's first session has a complete icon theme, not `hicolor` alone.
- A new user's dock holds a file browser, settings, a terminal, a web browser, a text editor and a calculator, as far as the system has them.
- The detected icon theme is written to the user's configuration file, where Settings shows it and the user can change it.

## Non-Goals

- Following later changes made in another desktop. Once written, the icon theme is the user's.
- Installing anything. Only apps and themes already on the system are used.
- Picking one app per job. On a system with both GNOME and KDE apps, the dock shows both.

## Behavior

- **Dock:** the system configuration's `dock.bookmarks` lists Otto's Files and Settings, then the usual GNOME and KDE terminal, Firefox (also `firefox-esr`), Chromium, and the GNOME and KDE text editor and calculator. A listed app with no desktop file installed is skipped with a warning, so each system shows whichever of them it has. The skipped entry stays in the configuration and appears in the next session after the app is installed.
- **Icon theme trigger:** a session starts and the user's configuration file (`$XDG_CONFIG_HOME/otto/config.toml`) does not exist. A greeter session and a test session never trigger it.
- **Icon theme result:** the file is created holding `icon_theme`. When no theme is found, nothing is written and the system configuration's value applies.
- **Icon theme source:** the first installed theme found in:
  1. the running desktop's own setting, when started from inside another desktop;
  2. a theme the user picked in any desktop: Plasma, GNOME, Cinnamon, MATE, Xfce, GTK's `settings.ini`, qt6ct, qt5ct;
  3. the distribution's defaults: GSettings overrides, then system-wide Plasma, Xfce and GTK settings;
  4. Breeze, then Adwaita.
- **Failure:** if the file cannot be written, the session starts on the system configuration and a warning is logged. The next session tries again.

## Constraints & Edge Cases

- Detection runs before anything reads the configuration, so the first session already uses the result.
- A user who deletes their configuration file gets a new detection.
- A user with no configuration file of their own, including one who used Otto before this existed, gets a detected icon theme in their next session.

## Rationale

- A fixed dock list is predictable and easy to change, and a missing app costs nothing. Guessing apps from MIME defaults and menu categories took a lot of code to cover the same common cases.
- No freedesktop standard records the user's icon theme, so each desktop's own settings are read. Distribution defaults are read from the files distributions use to brand their desktops, rather than from a per-distribution table.
- Writing the detected theme once keeps it predictable. A theme that changes whenever another desktop's settings change would be confusing, and Settings would have nothing to show.

## Open Questions

- Should first run also offer a short welcome, or suggest the launcher shortcut?
