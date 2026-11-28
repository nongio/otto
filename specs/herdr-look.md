# Otto's look for herdr-app

**Status:** draft  
**Related specs:** first-run-setup.md

## Summary

Otto ships a layout, two themes and a sessions mod for herdr-app, so the
agent workspace looks and reads like Otto's own Ask.

## Goals

- The package carries the look; `otto-agents plugins install` copies it into
  herdr-app's config folder.
- The sessions list shows each agent session as Ask does: a status dot, the
  session's title, and `@agent · status · folder` beneath, grouped into
  needs input, working, done and idle. Each group folds.
- The spaces list shows each space as a Solar folder in its color (home for
  `~`), with Solar glyphs for machines, worktrees, folding and adding.
- The themes take the desktop's accent and follow it when it changes.
- A file the person edited is never overwritten.

## Non-Goals

- Selecting the layout or theme for the person.
- Installing herdr-app.

## Behavior

- The look is copied only once herdr-app's config folder exists
  (`$HERDR_APP_HOME`, else `~/.config/herdr-app`). Before then, install and
  status say it is skipped and why.
- Each shipped file is one of: not there (written), current (left as is),
  stale (ours and unedited, replaced by the newer copy) or taken (edited, or
  someone else's file of the same name: left alone).
- A record in the folder (`.otto-look`) remembers what was written, so an
  edit is told apart from an older copy of ours.
- The accent keys of a theme (`cursor`, `colors.accent`, `colors.solid`) are
  ignored when deciding whether it was edited, and are carried over when a
  newer theme replaces it.
- The sessions mod reads the accent from the desktop portal
  (`org.freedesktop.appearance accent-color`) at start and on every
  `SettingChanged` for it, writing it into both themes only when it differs.
- `otto-agents plugins status` lists every file with its state.

## Constraints & Edge Cases

- herdr-app reads only its own folder; there is no system-wide location.
- Without a portal, or with no accent set, the themes keep Otto's blue.
- Without `gdbus` or `dbus-monitor` the accent stays as shipped.

## Rationale

- Copy-and-track rather than symlinks: herdr rewrites theme files through the
  mod, and a link into `/usr/share` would make that fail.
- The layout is offered, not chosen, because herdr-app is the person's tool
  and may be set up the way they like.
