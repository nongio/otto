# Troubleshooting

Otto is in a testing phase. This page covers how to gather evidence, the
failures people hit most often, and what makes a useful bug report.

## Getting logs

Otto logs to stderr, controlled by `RUST_LOG`:

```sh
RUST_LOG=info otto --winit &> /tmp/otto.log
RUST_LOG=debug otto --winit &> /tmp/otto.log
```

Use `&>` (both stdout and stderr), not `2>` — some output goes to stdout and a
half-captured log is a frustrating thing to debug from.

Narrow it down when `debug` is too noisy:

```sh
RUST_LOG="info,otto::udev=debug,otto::workspaces=trace" otto
```

When Otto is started by a display manager, its output goes to the journal:

```sh
journalctl --user -b            # this boot, user services
journalctl -b | grep -i otto
```

Component logs:

| Component | Where |
|-----------|-------|
| `xdg-desktop-portal-otto` | `journalctl --user -u xdg-desktop-portal-otto` |
| `otto-bar`, `otto-islands`, `otto-lock` | stderr — run them by hand to see it |
| `otto-rdp` | stderr; `run-rdp.sh` captures it to `/tmp/otto-rdp.log` |

## Configuration problems

**My config seems to be ignored.** Otto merges several files, later ones
overriding earlier:

1. `/etc/otto/config.toml`
2. `~/.config/otto/config.toml`
3. `./otto_config.toml` (working directory)
4. `./otto_config.{backend}.toml` — highest priority

A stray `otto_config.toml` in the directory you launched from silently wins over
your user config. The log reports which files were loaded.

**A setting has no effect.** Check the TOML parses (`taplo check`, or any TOML
validator), and check the log for a parse error. Otto keeps running with
defaults for a section it cannot read.

**A keyboard shortcut does nothing.** Unparsable triggers and actions are
**skipped with a warning**, not treated as an error:

```sh
grep -i "skipping shortcut" /tmp/otto.log
```

Common causes: a keysym name that does not exist, a modifier misspelled, or the
action name in the wrong case. See
[Keyboard Shortcuts](keyboard-shortcuts.md).

## Startup failures

**Otto exits immediately on a TTY.** It needs seat access — `seatd` or
`systemd-logind` running, and your user in the right group:

```sh
systemctl status seatd
groups        # look for 'seat' or 'video' / 'input' depending on distro
```

**"Permission denied" opening a DRM device.** Same cause. As a last resort Otto
runs as root, but that is a diagnostic, not a setup.

**Nothing appears after login from a display manager.** Check the session file
exists (`/usr/share/wayland-sessions/otto.desktop`) and look at the journal for
the failure.

## Rendering problems

**Missing, misplaced or flickering elements on the `--tty-udev` backend.**
Otto puts parts of the desktop on separate hardware planes instead of
compositing everything into one buffer. This is well tested on Intel GPUs; AMD
and NVIDIA are expected to fall back to full composition when the atomic test
rejects a configuration, but that fallback path is untested.

If you see this on AMD or NVIDIA, it is the first thing to suspect, and a report
is genuinely useful. See
[docs/developer/drm_plane.md](../developer/drm_plane.md).

**Windows are missing content or partially blank.** Try turning off occlusion
culling — Otto skips drawing layers it believes are fully hidden, and a wrong
belief shows up exactly like this:

```toml
occlusion_culling = false
```

**Tearing.** Explicit sync (`wp-linux-drm-syncobj-v1`) is implemented; tearing
usually means a client is not using it. Note which application, and report it.

**Otto crashes when a video plays.** Some dmabuf pixel formats are not handled
and abort the compositor. `mpv --vo=dmabuf-wayland` with NV12 content is a known
trigger. Work around it with `mpv --vo=gpu`, and report the format from the log.

## Applications

**An X11 app does not take keyboard focus.** Otto handles globally-active X11
clients explicitly, but there may be cases it misses. Note the application and
report it.

**An X11 app is the wrong size on a HiDPI screen.** Otto exports the scale via
XSETTINGS; a toolkit that ignores XSETTINGS needs its own env var
(`GDK_SCALE`, `QT_SCALE_FACTOR`).

**The clipboard empties when I close an app.** That is Wayland's design, not a
bug. See [Clipboard](clipboard.md).

**An app's menus do not appear in the top bar.** DBusMenu is opt-in per toolkit.
See [Top Bar](topbar.md#getting-an-app-to-export-its-menu).

## Performance

Otto ships with a [puffin](https://github.com/EmbarkStudios/puffin) profiling
server on port 8585:

```sh
cargo install puffin_viewer
puffin_viewer --url 127.0.0.1:8585
```

Match the versions: Otto's puffin 0.19.x needs puffin_viewer 0.22.0 or later.

For a build with the debugging and profiling tools enabled:

```sh
cargo run --features "dev"
```

**Everything is slow on battery (laptops).** Before blaming Otto, check whether
your firmware is throttling the CPU — some Intel laptops pin the CPU to a few
hundred MHz on battery via `BD_PROCHOT`. `watch -n1 "grep MHz /proc/cpuinfo"`
tells you quickly.

## Getting unstuck

| Situation | Way out |
|-----------|---------|
| Compositor unresponsive | `Ctrl+Alt+F2` to another VT, log in, investigate |
| Need to quit now | `Logo+Q` or `Ctrl+Alt+Backspace` |
| Locked out by a broken locker | `Ctrl+Alt+F2` — VT switching always works while locked |
| Broken config | Move `~/.config/otto/config.toml` aside and restart |

## Reporting a bug

Open an issue at [github.com/nongio/otto](https://github.com/nongio/otto/issues)
with:

1. **What you did, what happened, what you expected.**
2. **Otto version** — `otto --version`, or the commit if you built it.
3. **Backend** — `--tty-udev`, `--winit` or `--x11`.
4. **Hardware** — GPU and driver especially. `lspci -k | grep -A3 VGA`.
5. **Distribution**, and how you installed Otto.
6. **A log** — `RUST_LOG=debug otto &> /tmp/otto.log`, attached.
7. **Your config**, if it is not the shipped default.

For a rendering bug, the scene snapshot helps a lot. Bind these and press them
while the problem is visible:

```toml
[keyboard_shortcuts]
"Ctrl+Alt+J" = "SceneSnapshot"   # writes scene.json
"Ctrl+Alt+K" = "SkpSnapshot"     # writes a Skia picture of the focused screen
```

Attach `scene.json` — it describes exactly what Otto thought it was drawing.

## Getting help

Chat is on [Discord](https://discord.gg/AdXkrYKuz) and Matrix
[`#otto-compositor:matrix.org`](https://matrix.to/#/#otto-compositor:matrix.org).
Questions and "is this expected?" are welcome in either.
