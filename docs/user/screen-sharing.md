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

1. A **permission and source dialog** appears in the
   [dynamic island](dynamic-island.md): who is asking, and a list of what you
   can share.
2. You pick a source and grant, or cancel.
3. On grant, Otto starts a PipeWire stream of that source and hands the node to
   the application.

`otto-islands` is what renders that dialog with icons. If it is not running,
Otto falls back to another desktop's standard Access dialog (same choices,
without icons); if that is unreachable either, the request is denied rather
than left hanging — a screenshare that cannot ask fails closed.

### Choosing a source

The dialog lists, in order:

- every **monitor**, by connector name — including virtual outputs, so
  `virtual-1` is shareable like any other screen;
- every open **window**, by its title (or its app id if it has none), with its
  application icon.

The first entry is preselected.

### Sharing one window

Pick a window in that list and Otto captures that window alone: what is shared
is the window's own content, so windows stacked over it do not leak into the
stream, and the size follows the window rather than the screen. A window Otto
decorates shows a sharing indicator in its title bar while it is being
captured.

Only the window's identity is remembered — if it moves, is resized or changes
monitor mid-share, the stream follows it.

### Being asked only once

Apps that ask for persistence get a **restore token**. The next time the same
app starts a session it hands the token back and Otto re-uses the source that
was already approved instead of prompting again. This matters in practice for
Chrome, which creates a session twice — once for the preview inside its own
picker, once for the real capture — and would otherwise ask you twice for the
same share.

A token written by another desktop's portal is rejected, and a token naming a
source that no longer exists (an unplugged monitor, a closed window) falls back
to asking.

### When no dialog is reachable

If neither dialog backend can be reached, Otto shares one monitor without
asking. Which one comes from a file:

```sh
echo "HDMI-A-1" > ~/.config/otto/screencast-output
```

It is read fresh on every share request, so you can change it between sessions
without restarting anything. Use a name from `otto --probe`, or a virtual output
name like `virtual-1`. If the name does not match any available output, Otto
logs a warning and falls back to the first one.

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

Applications that ask through the desktop portal — GTK and Qt screenshot tools,
and sandboxed apps that cannot talk to Wayland directly — are answered too:
Otto captures the whole screen to `~/Pictures/Screenshots/` and returns the file.
That path runs `grim`, so install it if you want portal screenshots to work.

There is no built-in screenshot UI for selecting a region or a window
interactively, and **per-window capture through screencopy is not implemented**
— screencopy captures whole outputs or rectangles of them. Sharing a single
window *is* available through the screen-sharing portal; see
[Sharing one window](#sharing-one-window).

## AirPlay

You can send an Otto output to an Apple TV using
[doubletake](https://github.com/omarroth/doubletake), which captures through
the very portal you just configured:

1. Set up a non-interactive virtual output (AirPlay mirroring has no input
   channel — it is view-only).
2. Run doubletake and pick your receiver.
3. Choose that virtual output in Otto's share dialog — it is listed with the
   physical monitors.

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

**The wrong source is shared.** Pick it in the dialog — see
[Choosing a source](#choosing-a-source). If no dialog appeared at all, Otto took
the fallback path; see
[When no dialog is reachable](#when-no-dialog-is-reachable).

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

- A graphical source picker with window thumbnails and region selection — the
  consent dialog lists sources by name
- Region capture (a rectangle of an output) through the portal
- Multiple simultaneous outputs in one session (the first is used)
- Audio capture
