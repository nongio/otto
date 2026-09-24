# Getting Started

Installing Otto, launching it on your hardware, and the handful of things
worth checking on first run.

## Installing

Each block below installs the
[latest release](https://github.com/nongio/otto/releases/latest). Copy it as it
is: there is no version to fill in and the package manager pulls in the
dependencies. Packages are built for x86_64.

### Debian / Ubuntu

```sh
curl -fLO https://github.com/nongio/otto/releases/latest/download/otto-amd64.deb
sudo apt install ./otto-amd64.deb
```

### Fedora / RHEL

```sh
sudo dnf install https://github.com/nongio/otto/releases/latest/download/otto-x86_64.rpm
```

### Arch Linux

```sh
curl -fsSLO https://raw.githubusercontent.com/nongio/otto/main/PKGBUILD
makepkg -si
```

`makepkg` fetches the release tarball itself. If you already downloaded it,
drop the `PKGBUILD` next to it and it will be used without downloading again.

Once it is installed, Otto appears in your login manager's session menu. Pick
it there and log in, then run through the
[first-run checklist](#first-run-checklist).

### Nightly builds

Every commit to `main` is packaged as a
[nightly build](https://github.com/nongio/otto/releases/tag/nightly). It is
what is being worked on rather than what has been tested, so expect rough
edges and keep a release installed if you need the machine to work.

```sh
# Debian / Ubuntu
curl -fLO https://github.com/nongio/otto/releases/download/nightly/otto-nightly-amd64.deb
sudo apt install ./otto-nightly-amd64.deb

# Fedora / RHEL
sudo dnf install https://github.com/nongio/otto/releases/download/nightly/otto-nightly-x86_64.rpm

# Arch Linux
curl -fLO https://github.com/nongio/otto/releases/download/nightly/PKGBUILD-nightly-bin
makepkg -p PKGBUILD-nightly-bin -si
```

The URLs never change, so running the same commands again is how you update.
Each nightly is versioned as the release it was built from plus the commits
since (`1.4.1+nightly.r566.gcb851c3`, say), so it installs over that release
and the next nightly installs over it; `PKGBUILD-nightly-bin` is pinned to
the night's tarball, so on Arch it fetches the new one rather than reusing
one downloaded before. The nightly package replaces `otto-bin` on Arch.

Going back to a release is a downgrade, which the package manager wants
told: `sudo apt install --allow-downgrades ./otto-amd64.deb`,
`sudo dnf downgrade ./otto-x86_64.rpm`, or `makepkg -si` from the release
`PKGBUILD` above on Arch.

### Building from source

```sh
git clone https://github.com/nongio/otto
cd otto
cargo build --release
```

You need `libwayland`, `libxkbcommon`, `libudev`, `libinput`, `libgbm` and
[`libseat`](https://git.sr.ht/~kennylevinsen/seatd). Add `xwayland` if you want
to run X11 applications. Otto runs without it: build with
`--no-default-features --features "egl,winit,udev,renderer_sync"` to leave
XWayland support out, and X11-only applications will not start. On Arch,
`OTTO_XWAYLAND=0 makepkg -p PKGBUILD-git -si` does the same for the package.
Minimum supported Rust version is 1.87.0 for the
compositor itself; building the whole workspace (`otto-rdp` pulls in GStreamer)
needs 1.96.0.

## What gets installed

| Binary | Role |
|--------|------|
| `otto` | The compositor itself |
| `otto-bar` | [Top bar](topbar.md): clock, tray, application menus |
| `otto-islands` | [Dynamic island](dynamic-island.md): notifications, activities, dialogs |
| `otto-lock` | [Screen locker](lock-screen.md) |
| `otto-greeter` | [Login screen](login-greeter.md) client for greetd |
| `otto-rdp` | [Remote desktop](remote-desktop.md) bridge |
| `xdg-desktop-portal-otto` | [Screen sharing](screen-sharing.md) portal backend |

Packages also install `/etc/otto/config.toml` (a copy of
`otto_config.example.toml`), a `wayland-sessions` entry so Otto appears in your
display manager, the portal service files, and (on Arch and Fedora) the PAM
service for `otto-lock`.

## Launching

After installing, "Otto" appears in the session list of GDM, SDDM, LightDM and
friends. Pick it and log in.

To start it by hand:

```sh
otto                # auto-detects the backend
```

Otto chooses `--winit` when it finds an existing Wayland or X11 session
(it runs as a window, which is what you want for development), and `--tty-udev`
when started from a bare TTY.

### Backends

| Flag | Use |
|------|-----|
| `--tty-udev` | Bare metal: DRM/KMS + libinput. The real session. Needs logind/seatd, or root. |
| `--winit` | Runs as a window inside another Wayland or X11 session. Best for trying Otto out. |
| `--x11` | Runs as an X11 client. Basic and not actively maintained. |
| `--headless` | No display output; used by the test harness. |

### Other flags

| Flag | Effect |
|------|--------|
| `--login` | Run as a greeter host; see [Login Greeter](login-greeter.md) |
| `--probe` | Print the connectors, resolutions and refresh rates Otto can see, then exit |
| `--systemd-notify` | Send `READY=1` and activate `graphical-session.target` (for `Type=notify` user units) |
| `--version`, `--help` | As expected |

`--login` and `--systemd-notify` are orthogonal to the backend flag and can be
combined with it.

Use `--probe` first if you are writing display profiles — it tells you the exact
connector names (`eDP-1`, `HDMI-A-1`, …) and modes to put in your config.

## First-run checklist

Otto ships lean: the top bar and the dynamic island are separate programs, and
nothing is started for you unless you ask. A useful starting configuration in
`~/.config/otto/config.toml`:

```toml
[[exec_once]]
cmd = "otto-bar"

[[exec_once]]
cmd = "otto-islands"
```

See [Autostart](autostart.md) for XDG autostart and systemd integration.

Then, in rough order of how much you will miss them:

1. **Keyboard shortcuts.** The shipped `/etc/otto/config.toml` defines them.
   If you write your own config from scratch, Otto starts with *no* bindings at
   all. See [Keyboard Shortcuts](keyboard-shortcuts.md).
2. **Screen sharing.** This needs `xdg-desktop-portal` installed and a
   `portals.conf` pointing at Otto. See [Screen Sharing](screen-sharing.md).
3. **Screen locking.** This needs `/etc/pam.d/otto-lock`. Debian and Ubuntu
   users must install it manually. See [Lock Screen](lock-screen.md).
4. **Lid and power button.** Otto handles these itself, which requires
   `HandleLidSwitch=ignore` and `HandlePowerKey=ignore` in `logind.conf`.
   See [Power Management](power-management.md).
5. **Clipboard persistence.** Wayland loses clipboard contents when the source
   app exits. See [Clipboard](clipboard.md).

## Quitting

`Logo+Q` and `Ctrl+Alt+Backspace` always quit Otto. These two are wired into the
compositor ahead of the config file and cannot be rebound or removed, so there
is always a way out of a session whose config is broken.

## Where to go next

Take the [Desktop Tour](desktop-tour.md) to learn what everything on screen is,
then [Configuration](configuration.md) to start making it yours.
