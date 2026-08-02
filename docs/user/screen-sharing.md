# Screen Sharing

Otto shares your screen through the standard XDG Desktop Portal plus PipeWire —
the same mechanism GNOME and KDE use. Browsers, video-call apps, OBS and
Flatpak'd applications all work through it without knowing anything about Otto.

## Portal setup

Screen sharing needs three things running: `xdg-desktop-portal` (the frontend),
`xdg-desktop-portal-otto` (Otto's backend), and PipeWire.

### 1. Install the portal frontend

```sh
sudo pacman -S xdg-desktop-portal    # Arch
sudo dnf install xdg-desktop-portal  # Fedora
sudo apt install xdg-desktop-portal  # Debian/Ubuntu
```

Otto's own packages install the backend, its D-Bus service file and
`/usr/share/xdg-desktop-portal/portals/otto.portal`.

### 2. Tell the frontend to use Otto

Create `~/.config/xdg-desktop-portal/portals.conf`:

```ini
[preferred]
default=gtk

org.freedesktop.impl.portal.ScreenCast=otto
org.freedesktop.impl.portal.Settings=otto
```

This routes screen capture and the appearance settings (light/dark) to Otto, and
leaves everything else — file chooser, printing, notifications — to the GTK
backend. A copy of this file ships as
`/usr/share/doc/otto/portals.conf.example`.

`org.freedesktop.impl.portal.Settings` is what makes applications follow your
`theme_scheme` — see [Theming](theming.md).

### 3. Restart the portal

```sh
systemctl --user restart xdg-desktop-portal
```

### Verify

```sh
gdbus call --session \
  --dest org.freedesktop.portal.Desktop \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.ScreenCast.CreateSession \
  "{'handle_token':<'t1'>,'session_handle_token':<'s1'>}"
```

A successful call returns a request object path immediately.

## Sharing your screen

In a browser, click "Share screen"; in a video call, pick screen sharing. The
portal takes over from there:

1. A **permission dialog** appears in the
   [dynamic island](dynamic-island.md) — who is asking, and what for, with a
   list of everything you can share.
2. You pick a source and grant, or deny.
3. On grant, Otto starts a PipeWire stream of the chosen source and hands the
   node to the application.

`otto-islands` is what renders that dialog. If it is not running, Otto asks
another desktop's portal dialog instead (GTK, GNOME or KDE, in that order) —
the same list, without app icons.

### Choosing what to share

The dialog lists every connected monitor and, when the application asks for
windows too, every open window — one list, one choice. With a single monitor
and no windows on offer there is nothing to choose, so the dialog is just the
grant/deny prompt.

If no dialog renderer answers at all, monitor sharing still works: Otto shares
the first monitor it enumerates rather than failing. Windows are never picked
for you on that path.

To choose which monitor that fallback lands on, write the connector name into a
file:

```sh
echo "HDMI-A-1" > ~/.config/otto/screencast-output
```

It is read fresh on every share request, so you can change it between sessions
without restarting anything. Use a name from `otto --probe`, or a virtual output
name like `virtual-1`.

If the name does not match any available output, Otto logs a warning and falls
back to the first one. The file is only consulted when no dialog renderer
answers — when the dialog appears, your choice in it wins.

### Cursor

The portal's three cursor modes are supported: hidden, embedded (drawn into the
video) and metadata (sent alongside, for the receiver to draw). The application
picks; there is nothing to configure.

## Recording with OBS

OBS uses the portal like any other client:

1. Add a source → **Screen Capture (PipeWire)**.
2. Grant the permission dialog.

For a dedicated recording surface that is not one of your real monitors, set up
a [virtual output](display.md#virtual-outputs) and point OBS at its PipeWire
node directly:

```toml
[[virtual_outputs]]
name = "virtual-1"
resolution = { width = 1920, height = 1080 }
refresh_hz = 60.0
position = { x = 3840, y = 0 }
interactive = false
```

Otto logs the node id at startup:

```
Virtual output 'virtual-1' started (PipeWire node 42)
```

```sh
gst-launch-1.0 pipewiresrc path=42 ! videoconvert ! autovideosink
```

A virtual output is a real workspace you can drag windows onto — a "recording
stage" separate from what you are looking at.

## Screenshots

Otto implements `zwlr-screencopy-v1`, so the usual wlroots screenshot tools
work:

```sh
grim ~/Pictures/shot.png                      # whole screen
grim -o eDP-1 ~/Pictures/shot.png             # one monitor
grim -g "$(slurp)" ~/Pictures/shot.png        # a region you drag out
```

Bind one to a key:

```toml
[keyboard_shortcuts]
"Print" = { run = { cmd = "sh", args = ["-c", "grim ~/Pictures/$(date +%s).png"] } }
```

There is no built-in screenshot UI, and **per-window capture is not implemented**
— screencopy captures whole outputs or rectangles of them.

## AirPlay

You can send an Otto output to an Apple TV using
[doubletake](https://github.com/omarroth/doubletake), which captures through
the very portal you just configured:

1. Set up a non-interactive virtual output (AirPlay mirroring has no input
   channel — it is view-only).
2. Run doubletake and pick your receiver.
3. Otto's share dialog lists every monitor, the virtual output included — pick
   the one you want to cast. With a single monitor there is nothing to choose
   and casting starts on it.

Otto does not implement the AirPlay protocol itself. The sender side requires
Apple's FairPlay handshake, which no clean-room implementation exists for.

Receiving *from* an Apple device (an iPhone screen appearing as a window in
Otto) is not implemented either; UxPlay works as an independent application.

## Remote control

Screen sharing is view-only. To *control* Otto remotely, use the RDP bridge —
see [Remote Desktop](remote-desktop.md).

## Screen sharing and power

A lid close does not suspend the machine while a screenshare session is
actively streaming, so a remote viewer is not cut off. See
[Power Management](power-management.md).

## Troubleshooting

**The screen-share picker in my browser is empty, or sharing fails silently.**
Check the backend is running and reachable:

```sh
busctl --user status org.freedesktop.impl.portal.desktop.otto
tail -f /tmp/portal-otto.log
```

That log records every method call, the compositor interaction and the PipeWire
node ids.

**The permission dialog never appears.** `otto-islands` is not running. Start
it (`[[exec_once]]`) and try again.

**The wrong monitor is shared.** See
[Choosing which monitor](#choosing-which-monitor) above.

**The video is mangled — torn, sheared or wrongly coloured.** This is a dmabuf
modifier negotiation problem between Otto and the consumer. Capture a log with:

```sh
GST_DEBUG=3 <your app>
```

and open an issue with it.

**Sharing works in Firefox but not in a Flatpak app.** Flatpak apps use the
portal from inside the sandbox; make sure `xdg-desktop-portal` and the Otto
backend are both on the session bus the sandbox sees.

## Not yet implemented

- A graphical source picker (window thumbnails, region selection)
- Per-window capture
- Restore tokens — every share asks permission again
- Multiple simultaneous outputs in one session (the first is used)
- Audio capture
