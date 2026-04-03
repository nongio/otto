# Autostarting Applications

Otto supports three complementary approaches for launching applications automatically at startup. Choose the one that best fits your workflow — or combine them.

## Config Reference

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `exec_once` | array of commands | `[]` | Commands to run once at startup |
| `xdg_autostart` | bool | `false` | Scan XDG autostart directories for `.desktop` entries |
| `systemd_notify` | bool | `false` | Send `sd_notify(READY=1)` and activate `graphical-session.target` |

---

## 1. `exec_once` — Run Commands at Startup

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

Each entry takes:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `cmd` | string | yes | Executable name (must be on `$PATH`) or absolute path |
| `args` | array of strings | no | Command-line arguments (defaults to `[]`) |

Entries are spawned non-blocking (fire-and-forget) in listed order. Otto calls `spawn()` sequentially but does not wait for any process to become ready before launching the next — there is no startup ordering or readiness guarantee between entries.

### Environment

Each spawned process inherits the compositor's environment with the following additions:

| Variable | Value | When |
|----------|-------|------|
| `WAYLAND_DISPLAY` | Socket name (e.g., `wayland-1`) | Always |
| `DISPLAY` | X11 display (e.g., `:0`) | When XWayland is active |
| `XDG_SESSION_TYPE` | `wayland` | Always |
| `XDG_CURRENT_DESKTOP` | `otto` | Always |

**Best for:** personal dotfiles, simple setups, or apps you always want running.

**Limitation:** Otto does not restart crashed processes. Use a supervisor (systemd, s6, runit) if you need that.

---

## 2. XDG Autostart Entries

Otto can respect the [XDG Autostart specification](https://specifications.freedesktop.org/autostart-spec/latest/), reading `.desktop` files from standard autostart directories.

Enable it in your config:

```toml
xdg_autostart = true
```

### Directory Scan Order

Otto scans directories in this order — later entries override earlier ones by filename:

1. **System dirs** — each directory in `$XDG_CONFIG_DIRS` (defaults to `/etc/xdg`), with `/autostart` appended
   ```
   /etc/xdg/autostart/*.desktop
   ```
2. **User dir** — `$XDG_CONFIG_HOME/autostart` (defaults to `~/.config/autostart`)
   ```
   ~/.config/autostart/*.desktop
   ```

If a user entry has the same filename as a system entry, the user entry takes precedence.

### Filtering Rules

Otto implements the full XDG autostart filtering spec:

| Field | Effect |
|-------|--------|
| `Hidden=true` | Entry is skipped |
| `OnlyShowIn=Otto` | Only launched when running under Otto |
| `NotShowIn=Otto` | Skipped when running under Otto |
| _(neither set)_ | Always launched |

The desktop environment name matched is `Otto` (case-insensitive).

### Example

`~/.config/autostart/mako.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Mako
Exec=mako
Hidden=false
```

**Best for:** distro-managed or shared configurations, apps that ship their own `.desktop` autostart file.

---

## 3. Systemd User Services

The most robust approach for production setups. Otto integrates with `systemd --user` to signal readiness and activate the `graphical-session.target`, allowing dependent services to start in the correct order.

### Enabling systemd Integration

Any of these three methods will enable systemd notify:

```toml
# Option A: config file
systemd_notify = true
```

```sh
# Option B: CLI flag
otto --tty-udev --systemd-notify
```

```sh
# Option C: environment variable
OTTO_SYSTEMD_NOTIFY=1 otto --tty-udev
```

### What Happens at Startup

When systemd notify is enabled, Otto performs these steps after the Wayland socket is listening:

1. Exports `WAYLAND_DISPLAY` to the systemd user session (`systemctl --user set-environment`)
2. Sends `READY=1` via `sd_notify`
3. Runs `systemctl --user start graphical-session.target`

### Example: Otto as a Systemd Service

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

Enable and start:

```sh
systemctl --user enable otto.service
systemctl --user start otto.service
```

### Depending on Otto from Other Services

Once Otto activates `graphical-session.target`, other user services can depend on it:

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

| Method | Use for |
|--------|---------|
| **systemd** | Otto itself + critical infrastructure (portals, PipeWire) |
| **`exec_once`** | Personal lightweight utilities (clipboard manager, notification daemon) |
| **`xdg_autostart`** | Desktop apps that ship their own autostart `.desktop` file |

### Execution Order

1. Wayland socket becomes ready
2. `WAYLAND_DISPLAY` is exported to systemd
3. `sd_notify(READY=1)` is sent (if enabled)
4. `graphical-session.target` is activated (if enabled)
5. `exec_once` entries are spawned in order
6. XDG autostart entries are launched (if enabled)
