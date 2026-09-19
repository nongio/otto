# Starting programs with the session

Programs Otto itself launches at startup, written in the configuration file.
**There is no D-Bus setting for this** — it is a list, so it is edited by hand
and takes effect at the next login.

For `.desktop` files that other applications install, see
[autostart-xdg.md](autostart-xdg.md).

## What to write

Add one block per program to the writable config file. Find that file first:

```sh
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings ConfigPath
```

Then append:

```toml
[[exec_once]]
cmd = "wlsunset"
args = ["-l", "48.8", "-L", "2.3"]
```

| Field | Required | What it is |
|---|---|---|
| `cmd` | yes | An executable on `$PATH`, or an absolute path |
| `args` | no | Arguments, one per array entry. Defaults to `[]` |

Each argument is a separate string. `args = ["-l 48.8"]` is **wrong** — write
`args = ["-l", "48.8"]`.

## The two Otto expects

The top bar and the dynamic island are separate programs. **Nothing starts them
unless they are listed here**, so a config with no `exec_once` at all is a
desktop with no bar and no notifications:

```toml
[[exec_once]]
cmd = "otto-bar"
args = []

[[exec_once]]
cmd = "otto-islands"
args = []
```

If someone reports a missing top bar or missing notifications, check this first.

## Useful examples

```toml
# A wallpaper daemon, for animated or per-monitor wallpaper
[[exec_once]]
cmd = "swaybg"
args = ["-i", "/home/me/Pictures/wall.jpg", "-m", "fill"]

# Night-time colour temperature, by latitude and longitude
[[exec_once]]
cmd = "wlsunset"
args = ["-l", "48.8", "-L", "2.3"]

# A clipboard manager
[[exec_once]]
cmd = "wl-paste"
args = ["--watch", "cliphist", "store"]

# A network applet in the tray
[[exec_once]]
cmd = "nm-applet"
args = []
```

## How they run

- **In order, without waiting.** Otto spawns each one and moves straight to the
  next. There is no readiness guarantee and no way to say "start this after
  that".
- **Fire and forget.** Otto does not restart a program that crashes or exits. If
  it must stay up, use a systemd user service instead (below).
- **After the Wayland socket is ready**, and before the XDG autostart entries.
- **With the session's environment**, plus these:

| Variable | Value |
|---|---|
| `WAYLAND_DISPLAY` | the socket name, such as `wayland-1` |
| `DISPLAY` | the X11 display, when XWayland is running |
| `XDG_SESSION_TYPE` | `wayland` |
| `XDG_CURRENT_DESKTOP` | `otto` |

## When it should restart on failure: systemd

For anything that must stay running, a systemd user service is the right tool.
Turn on Otto's systemd integration first:

```toml
systemd_notify = true
```

This is a top-level key, not inside a table. It makes Otto export
`WAYLAND_DISPLAY` to the systemd user session, send `READY=1`, and start
`graphical-session.target` once the socket is listening. The same can be done
with the `--systemd-notify` flag or `OTTO_SYSTEMD_NOTIFY=1`.

Then a service can depend on the session being up:

```ini
# ~/.config/systemd/user/waybar.service
[Unit]
Description=Waybar status bar
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=/usr/bin/waybar
Restart=on-failure

[Install]
WantedBy=graphical-session.target
```

```sh
systemctl --user enable --now waybar.service
```

**A helper started by systemd will not find the session if `WAYLAND_DISPLAY` in
the systemd environment is stale** — from an earlier session, say. That is the
usual cause of a bus-activated helper reporting no compositor.

## Which to use

| Use | For |
|---|---|
| `[[exec_once]]` | Personal utilities: wallpaper, clipboard, tray applets, the bar and the islands |
| systemd user service | Anything that must restart on failure, or start in a strict order |
| XDG autostart | Applications that ship their own `.desktop` file — see [autostart-xdg.md](autostart-xdg.md) |

## Rules

1. **Ask before editing the config file**, and read it before you write to it.
2. **Add only what was asked for.** Leave the rest of the file alone, comments
   included.
3. **Say it needs a restart** — logging out and back in — because the file is
   read once, at startup.
4. **Check the program exists** (`command -v swaybg`) before writing it in. A
   `cmd` that is not on `$PATH` fails silently at the next login.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/autostart.md
- [autostart-xdg.md](autostart-xdg.md) — `.desktop` autostart entries
