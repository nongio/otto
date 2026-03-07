# Autostarting Applications

Otto supports three complementary approaches for launching applications automatically at startup. Choose the one that best fits your workflow — or combine them.

---

## 1. Otto Config: `exec_once`

The simplest option. Define a list of commands in your Otto config file and they will be spawned once, in order, when the compositor is ready.

```toml
# ~/.config/otto/config.toml

[[exec_once]]
cmd = "waybar"
args = []

[[exec_once]]
cmd = "dunst"
args = []

[[exec_once]]
cmd = "wlsunset"
args = ["-l", "48.8", "-L", "2.3"]
```

Each entry is spawned non-blocking (fire-and-forget) in the listed order — meaning `spawn()` calls are made in sequence, but Otto does not wait for any process to be ready before launching the next. There is no startup ordering or readiness guarantee between entries.

**Best for:** personal dotfiles, simple setups, or apps you always want running.

**Limitation:** Otto does not restart crashed processes. Use a supervisor (systemd, s6, runit) if you need that.

---

## 2. XDG Autostart Entries

Otto can respect the [XDG Autostart specification](https://specifications.freedesktop.org/autostart-spec/latest/), reading `.desktop` files from the standard autostart directories.

Enable it in your config:

```toml
# ~/.config/otto/config.toml
xdg_autostart = true
```

Otto will scan:
- `~/.config/autostart/*.desktop` — per-user entries (override system entries of the same name)
- `/etc/xdg/autostart/*.desktop` — system-wide entries

**Filtering rules** (full XDG spec compliance):
- `Hidden=true` — entry is skipped
- `OnlyShowIn=Otto` — only launched when running under Otto
- `NotShowIn=Otto` — skipped when running under Otto
- Entries with no `OnlyShowIn` / `NotShowIn` are always launched

**Example** `~/.config/autostart/mako.desktop`:
```ini
[Desktop Entry]
Type=Application
Name=Mako
Exec=mako
Hidden=false
```

**Best for:** distro-managed or shared configurations, apps that ship with their own `.desktop` autostart file.

---

## 3. Systemd User Services

The most robust approach for production setups. Otto can integrate with `systemd --user` to signal readiness and activate the `graphical-session.target`, allowing other services to start in the correct order.

### Enable systemd notify

Either add to your config:

```toml
# ~/.config/otto/config.toml
systemd_notify = true
```

Or pass a CLI flag (useful in the unit file itself):

```sh
otto --tty-udev --systemd-notify
```

When enabled, Otto will:
1. Send `READY=1` via `sd_notify` after the Wayland socket is listening
2. Run `systemctl --user start graphical-session.target`

### Example unit file

Create `~/.config/systemd/user/otto.service`:

```ini
[Unit]
Description=Otto Wayland Compositor
After=systemd-user-sessions.service

[Service]
Type=notify
ExecStart=/usr/bin/otto --tty-udev --systemd-notify
Restart=on-failure

[Install]
WantedBy=default.target
```

Enable and start it:

```sh
systemctl --user enable otto.service
systemctl --user start otto.service
```

### Depending on Otto from other services

Once Otto declares readiness via `graphical-session.target`, other user services can wait for it:

```ini
# ~/.config/systemd/user/waybar.service
[Unit]
Description=Waybar status bar
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=/usr/bin/waybar

[Install]
WantedBy=graphical-session.target
```

**Best for:** production installations, multi-user systems, services that need strict startup ordering or automatic restart.

---

## Combining Approaches

The three methods are not mutually exclusive. A common setup:

- **systemd** manages Otto itself + critical services (portals, pipewire)
- **`exec_once`** handles personal lightweight utilities (clipboard manager, notification daemon)
- **`xdg_autostart`** picks up any desktop-environment apps that ship their own autostart entries
