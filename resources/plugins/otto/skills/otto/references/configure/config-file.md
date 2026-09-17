# The configuration file

Where a setting is written, which file wins, and what it takes for a change to
show up. The keys themselves are in the topic files — this is the mechanics.

## Which file

Otto merges these in order, later over earlier:

1. `/etc/otto/config.toml` — system defaults, installed by the package, never
   written by Otto
2. `~/.config/otto/config.toml` — **where a person's own configuration belongs**
3. `./otto_config.toml` — next to the working directory, for development
4. `./otto_config.{backend}.toml` — `otto_config.winit.toml`, `otto_config.udev.toml`

Ask the compositor rather than guessing:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings ConfigPath
```

That is the highest-priority file that exists, and the one the Settings app and
the desktop itself write to. With no `otto_config.toml` about, it is
`~/.config/otto/config.toml`, created if absent.

**A stray `otto_config.toml` shadows everything below it** for every key it
sets — including `~/.config/otto/config.toml` — and quietly becomes the file
that gets written. The current directory is the home directory for a normal
login and the checkout for `cargo run`, so this is the usual reason a setting
"does not stick". Otto logs a warning at startup when the writable file is not
the user's own config.

`otto_config.example.toml` in the source tree documents every option with
comments; the Arch packages install an untouched copy at
`/etc/otto/config.example.toml` to diff against.

## What it takes to take effect

- A `live` setting applies the moment it is set, from the bus or the Settings
  app.
- A `restart` setting is persisted and read at the next start. The running
  session is left exactly as it was, so nothing half-applies.
- **Every hand edit is a restart**, whatever the setting's `apply` says: the
  file is read once, at startup.

Restarting means logging out and back in. `Logo+Q` quits Otto, which ends the
session — say so before suggesting it, because anything unsaved goes with it.

The Settings app has no file watcher: an edit made while it is open will not
show up until it is reopened.

## Editing it by hand

Read the file before changing it, keep the edit to the keys that were asked
for, and leave the comments alone. TOML errors are reported in the log at
startup — a file that will not parse means the layer is skipped, not that Otto
refuses to start, which is easy to miss.

## What is file-only

Lists and tables have no identifier and no `Set`:

| What | Where it is written up |
|---|---|
| Shortcuts | [shortcuts.md](shortcuts.md) |
| Display profiles, panel zones | [displays.md](displays.md) |
| Virtual outputs | [virtual-outputs.md](virtual-outputs.md) |
| Dock bookmarks, places, the Trash | [dock.md](dock.md) |
| `mac_style_modifiers` | [keyboard.md](keyboard.md) |
| Autostart, workspaces, accessibility, the top bar's own file | [system.md](system.md) |

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/configuration.md
- https://github.com/nongio/otto/blob/main/docs/user/troubleshooting.md
