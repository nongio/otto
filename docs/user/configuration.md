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
| [Display](display.md) | Scaling, display profiles, layer shell zones |
| [Theming](theming.md) | Theme scheme, accent color, fonts, background, cursors, icons |
| [Input](input.md) | Keyboard repeat, touchpad, pointer acceleration |
| [Keyboard Shortcuts](keyboard-shortcuts.md) | Key remapping, shortcut bindings, available actions |
| [Dock](dock.md) | Dock appearance, bookmarks, autohide, magnification |
| [Audio](audio.md) | Sound effects and sound themes |
| [Power Management](power-management.md) | Lid switch behavior |
| [Night Shift](night-shift.md) | Color temperature and brightness control |
| [Autostart](autostart.md) | exec_once, XDG autostart, systemd integration |
| [Clipboard](clipboard.md) | Clipboard persistence and managers |

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

**Icon/cursor theme not found:**
- Verify the theme is installed: `ls /usr/share/icons/ ~/.local/share/icons/`
- Theme names are case-sensitive.
- Some themes may require additional packages.

**Keyboard shortcuts not working:**
- Modifier names are `Logo`, `Ctrl`, `Alt`, `Shift` (case-sensitive).
- Some shortcuts may conflict with system bindings.

**Touchpad settings ignored:**
- Settings only apply to touchpad devices, not mice.
- Some hardware may not support all features.
- Check `libinput` capabilities for your device.
