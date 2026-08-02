# Configuration

Otto uses TOML configuration files to customize your experience. This page explains how config files are loaded and merged; the individual settings are documented in their own pages.

## Configuration Files

Otto searches for configuration files in the following order (later files override earlier ones):

1. **System config**: `/etc/otto/config.toml`
   - System-wide defaults managed by administrators
   - Lowest priority

2. **User config**: `$XDG_CONFIG_HOME/otto/config.toml`
   - Per-user configuration (defaults to `~/.config/otto/config.toml`)
   - Follows XDG Base Directory specification
   - **Recommended location** for user customization

3. **Local override**: `./otto_config.toml`
   - Config file in the current working directory
   - Useful for development and testing

4. **Backend-specific**: `./otto_config.{backend}.toml`
   - Backend-specific overrides (e.g., `otto_config.winit.toml`, `otto_config.udev.toml`)
   - Highest priority
   - Useful for maintaining different settings per backend during development

Values from higher-priority files are merged recursively into lower-priority ones, so you only need to specify the options you want to override.

Config files are read once, when the session starts: an edit takes effect on the next login.

## Getting Started

```bash
# Create user config directory
mkdir -p ~/.config/otto

# Copy the example config (contains all options with documentation)
cp otto_config.example.toml ~/.config/otto/config.toml

# Edit as needed
$EDITOR ~/.config/otto/config.toml
```

## Configuration Topics

| Topic | Description |
|-------|-------------|
| [Display](display.md) | Scaling, display profiles, monitor arrangement, virtual outputs, layer shell zones |
| [Theming](theming.md) | Theme scheme, accent color, fonts, background, cursors, icons |
| [Input](input.md) | Keyboard layout and repeat, touchpad, pointer acceleration |
| [Keyboard Shortcuts](keyboard-shortcuts.md) | Binding syntax and the complete action list |
| [Dock](dock.md) | Dock appearance, bookmarks, autohide, magnification |
| [Top Bar](topbar.md) | Clock format, tray, global menus (`topbar.toml`) |
| [Audio](audio.md) | Sound effects and sound themes |
| [Power Management](power-management.md) | Lid switch, power button, suspend, clamshell |
| [Lock Screen](lock-screen.md) | Locker command, idle auto-lock, PAM setup |
| [Login Greeter](login-greeter.md) | `--login` mode and the greeter command |
| [Night Shift](night-shift.md) | Color temperature and brightness control |
| [Autostart](autostart.md) | exec_once, XDG autostart, systemd integration |
| [Clipboard](clipboard.md) | Clipboard persistence and managers |

For everything else — how to *use* the desktop rather than configure it — start
from the [User Guide index](README.md).

## Tips

1. **Start with the example** — copy `otto_config.example.toml` to `~/.config/otto/config.toml` and modify as needed.
2. **Use XDG paths** — `~/.config/otto/config.toml` persists across updates.
3. **System-wide defaults** — administrators can set defaults in `/etc/otto/config.toml`.
4. **Backend-specific settings** — use `otto_config.winit.toml` in the current directory for development/testing.
5. **Scaling** — adjust `screen_scale` based on your display DPI (1.0 for 96 DPI, 2.0 for HiDPI).

## Troubleshooting

**Configuration not loading:**
- Verify the TOML syntax (matching brackets, quotes, commas).
- Check Otto's log output for parsing errors and which config files were loaded.
- Ensure the config file is in one of the searched locations listed above.

**An edit to `/etc/otto/config.toml` does nothing:**
- The user config wins. Check whether `~/.config/otto/config.toml` sets the same key — a key present there shadows the system file even if you never typed it, since older builds copied the whole `[dock]` table into it (see [Dock](dock.md)).
- Restart the session: config is only read at startup.

**Icon/cursor theme not found:**
- Verify the theme is installed: `ls /usr/share/icons/ ~/.local/share/icons/`
- Theme names are case-sensitive.
- Some themes may require additional packages.

**Keyboard shortcuts not working:**
- Modifiers are `Ctrl`, `Alt`, `Shift` and `Logo` (aliases accepted, case-insensitive).
- An unparsable trigger or action is **skipped with a warning**, not an error —
  grep the log for `skipping shortcut`.
- Some shortcuts may conflict with an application's shortcut inhibitor.

**Touchpad settings ignored:**
- Settings only apply to touchpad devices, not mice.
- Some hardware may not support all features.
- Check `libinput` capabilities for your device.

For a fuller list, see [Troubleshooting](troubleshooting.md).
