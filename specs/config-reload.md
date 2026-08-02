# Live Configuration Reload

**Status:** draft
**Related specs:** [multi-output](multi-output.md), [topbar](topbar.md)

## Summary

Editing an Otto configuration file applies to the running session. The
compositor notices the edit on its own and re-applies the settings; restarting
the session is not required to see a change take effect.

## Goals

- An edit to any config file in the load order (system, user, local override,
  backend override) is picked up while Otto runs.
- Creating a config file that did not exist before counts as an edit.
- Re-applied settings take effect on the live session, not only on things
  created afterwards: dock appearance and bookmarks, wallpaper and background
  colour, keyboard layout and repeat, cursor theme and size, touchpad and
  pointer settings, global scale, theme colours and fonts.
- A reload never resets unrelated session state: open windows keep their
  position and stacking, the current workspace stays current, and clients are
  not disconnected.
- Settings Otto itself writes back to the config file (the dock's autohide,
  magnification and bookmarks) do not cause the dock to visibly reset when the
  write is read back.

## Non-Goals

- Watching config files of separate components (the top bar reads
  `otto_topbar.toml` itself).
- Applying settings that only exist at process start: the wayland socket,
  `exec_once` and XDG autostart entries (already-launched programs are not
  restarted or killed), `systemd_notify`, and the login-mode greeter command.
- Reverting runtime changes made through the UI that were never persisted.
- Reacting to an output scale change by re-reading the config files — output
  scale is applied from config to outputs, not the other way round.

## Behavior

- When a watched config file's content changes on disk, Otto re-reads the whole
  load order and merges it exactly as it does at startup.
- When the merged result is identical to the running configuration, nothing is
  applied and nothing is redrawn.
- When the merged result differs:
  - Touchpad and pointer settings are re-applied to every input device present,
    including devices plugged in after startup.
  - Keyboard layout, variant, options, repeat delay and repeat rate are
    re-applied to the seat. If the new layout fails to compile the previous one
    stays in effect and a warning is logged.
  - A changed cursor theme or size is loaded and the cursor is drawn with it.
  - A changed `[dock]` section is adopted by the dock: size, magnification,
    autohide, icon colorisation and bookmarks all take effect. Bookmarks are
    re-resolved only when the bookmark list itself changed.
  - A changed background image or colour replaces the wallpaper on every
    workspace of every output. A background image that fails to load leaves the
    background colour showing and logs a warning.
  - A changed global scale is applied to every output, and window and layer
    positions are fixed up as they are when the scale keybinding is used.
  - Everything read while drawing (theme scheme, accent colour, font family,
    layer-shell zones, shortcut bindings, sound settings) applies on the next
    frame.
- A malformed config file is reported and ignored: the values that were parsed
  successfully still apply, and the session keeps running.

## Constraints & Edge Cases

- Detection is polled, not instantaneous: an edit applies within a couple of
  seconds.
- A file that is rewritten in place and a file that is replaced by a rename
  must both be detected, as editors do both.
- Config files are also read from the working directory. A relative path is
  resolved the same way at reload as at startup.
- Shortcut bindings are compiled from the config; a reload recompiles them, so
  a rebound key works on the next press without a restart.
- Reload runs on the compositor's event loop, so applying it must not block:
  work that can be deferred to the next frame is.

## Rationale

- Polling file metadata rather than subscribing to filesystem events keeps the
  feature dependency-free and behaves the same for every way an editor writes a
  file; a stat of a handful of paths every couple of seconds is far cheaper than
  the reload it guards.
- Only the sections that changed are re-applied, because re-applying everything
  would, for example, re-resolve dock bookmarks (an async lookup per bookmark)
  every time the dock persists its own settings.
- Global scale reuses the existing runtime scale-change path rather than a
  second one, so a config-driven scale change behaves exactly like the
  keybinding.

## Open Questions

- Should a reload be requestable explicitly (a keybinding or a D-Bus call) in
  addition to being detected?
- Should `exec_once` entries added to the config while Otto runs be launched?
