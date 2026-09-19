# XDG autostart entries

The freedesktop standard for "start this program when the desktop starts": a
`.desktop` file in an autostart directory. Applications install their own — a
password manager, a cloud-sync client, a notification daemon.

**Otto ignores these unless it is told to read them.** For programs listed in
Otto's own configuration instead, see
[autostart-commands.md](autostart-commands.md).

## Turning it on

This is a top-level key in the configuration file, not a D-Bus setting. Find the
file:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings ConfigPath
```

Add:

```toml
xdg_autostart = true
```

It is `false` by default, and takes effect at the next login.

## Where the entries come from

Otto reads, in this order:

1. `/etc/xdg/autostart/*.desktop` — every directory in `$XDG_CONFIG_DIRS`, with
   `/autostart` appended. This is where packages install their entries.
2. `~/.config/autostart/*.desktop` — `$XDG_CONFIG_HOME/autostart`, the person's
   own.

**A user entry with the same filename as a system one replaces it.** That is how
a system-wide entry is overridden or switched off.

List what is there:

```sh
ls /etc/xdg/autostart/ ~/.config/autostart/
```

## Writing one

`~/.config/autostart/mako.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Mako
Exec=mako
Hidden=false
```

| Key | Required | What it does |
|---|---|---|
| `Type` | yes | Always `Application` |
| `Name` | yes | What it is called |
| `Exec` | yes | The command, with its arguments |
| `Hidden` | no | `true` means skip this entry entirely |
| `OnlyShowIn` | no | Start only in the listed desktops |
| `NotShowIn` | no | Skip in the listed desktops |

Otto's desktop name is **`Otto`**, matched without regard to case.

## Turning one entry off

To stop a system entry from starting, copy it and hide it — do not edit the file
in `/etc`, which an upgrade will replace:

```sh
mkdir -p ~/.config/autostart
cp /etc/xdg/autostart/thing.desktop ~/.config/autostart/
printf 'Hidden=true\n' >> ~/.config/autostart/thing.desktop
```

The user copy shadows the system one, and `Hidden=true` skips it.

Alternatively, to keep it everywhere except Otto, add `NotShowIn=Otto;` instead.

## To start only under Otto

```ini
OnlyShowIn=Otto;
```

Useful for a helper that makes sense on this desktop and nowhere else.

## How they run

- **After the `[[exec_once]]` entries.** The order among the `.desktop` files
  themselves is not defined.
- **Fire and forget.** Otto does not restart one that exits.
- With the same environment `[[exec_once]]` entries get, including
  `XDG_CURRENT_DESKTOP=otto`.

## Rules

1. **Check `xdg_autostart` is on** before telling anyone their entry will run.
   With it off, nothing in those directories starts and the entry is not
   broken.
2. **Write to `~/.config/autostart/`, never to `/etc/xdg/autostart/`** — the
   second needs root and is overwritten by package upgrades.
3. **Say it needs a restart** to take effect.
4. `Exec` is not a shell line: no pipes, no `&&`, no environment assignments. If
   they need those, write a small script and point `Exec` at it.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/autostart.md
- The specification: https://specifications.freedesktop.org/autostart-spec/latest/
