## Configuration

Otto uses a TOML configuration file to customize your experience. The configuration file allows you to control display settings, theming, input behavior, keyboard shortcuts, and more.

### Configuration Files

Otto looks for configuration files in the following order:

`otto_config.toml` - Main configuration file

**Overrides** 

These are mainly useful for development

`otto_config.{backend}.toml` - Backend-specific overrides (e.g., `otto_config.winit.toml`, `otto_config.udev.toml`)

Backend-specific configurations override settings from the main file.

Place your configuration files in the same directory as the Otto binary, or in the working directory from which you launch Otto.

A reference configuration with all available options is provided in `otto_config.example.toml`.

---

### Display

```toml
# Overall scaling factor for all displays
screen_scale = 2.0

# Compositor backend mode: "drm" for bare metal, auto-detected otherwise
compositor_mode = "drm"
```

**Named Displays**

Configure specific displays by name (useful for development backends like winit):

```toml
[displays.named."winit"]
resolution = { width = 1280, height = 1000 }
refresh_hz = 60.0
position = { x = 0, y = 0 }
```

**Generic Display Matching**

Configure displays based on connector type:

```toml
[[displays.generic]]
match = { connector_prefix = "HDMI" }
resolution = { width = 1920, height = 1080 }
refresh_hz = 60.0
position = { x = 1920, y = 0 }
```

---

### Theme Configuration

```toml
# UI theme scheme
theme_scheme = "Light"  # or "Dark"

# Font configuration
font_family = "Inter"

# Background image (path relative to config file or absolute)
background_image = "./resources/background.jpg"
```

### Cursor Settings

```toml
# Cursor size in pixels
cursor_size = 24

# Cursor theme name (must be installed in standard icon directories)
cursor_theme = "Notwaita-Black"
```

Available cursor themes can be found in:
- `/usr/share/icons/`
- `~/.local/share/icons/`
- `~/.icons/`

### Icon Theme

```toml
# Icon theme (auto-detected if not specified)
icon_theme = "GNUstep"
```

Popular icon themes include:
- **Notwaita** - Dark cursor theme
- **Papirus** - Modern, colorful icon set
- **Adwaita** - GNOME default icons
- **Breeze** - KDE default icons
- **WhiteSur** - macOS-inspired icons
- **Yaru** - Ubuntu default icons
- **Fluent** - Windows-inspired icons

Icon themes are searched in the same directories as cursor themes.

---

### Input Configuration

**Keyboard Settings**

```toml
# Key repeat delay in milliseconds (time before repeat starts)
keyboard_repeat_delay = 300

# Key repeat rate (keys per second)
keyboard_repeat_rate = 30
```

**Touchpad Settings**

```toml
[input]
# Tap-to-click (1-finger = left, 2-finger = right, 3-finger = middle)
tap_enabled = true

# Tap-and-drag (tap, hold, and drag)
tap_drag_enabled = true

# Tap drag lock (lift finger during drag without releasing)
tap_drag_lock_enabled = false

# Click method:
#   "clickfinger" = 1-finger=left, 2-finger=right, 3-finger=middle
#   "buttonareas" = traditional (top-right corner = right click)
touchpad_click_method = "clickfinger"

# Disable touchpad while typing
touchpad_dwt_enabled = true

# Natural (reverse) scrolling
touchpad_natural_scroll_enabled = true

# Left-handed mode (swap left/right buttons)
touchpad_left_handed = false

# Middle mouse button emulation (left+right click simultaneously)
touchpad_middle_emulation_enabled = false
```

---

### Layer Shell Configuration

Control the maximum space that panels, bars, and docks can occupy:

```toml
[layer_shell]
# Maximum exclusive zone per edge in logical points (0 = unlimited)
# These values are multiplied by screen_scale to get physical pixels
max_top = 100       # Top panels/bars
max_bottom = 100    # Bottom panels/docks
max_left = 50       # Left side panels
max_right = 50      # Right side panels
```

---

### Application Defaults

---

### Keyboard Shortcuts

Remap individual keys:

```toml
[key_remap]
"BackSpace" = "Delete"
```

**Modifier Remapping**

Remap modifier keys (currently commented out in example):

```toml
# [modifier_remap]
# logo = "ctrl"  # Remap Logo/Super key to Control
```

Define custom keyboard shortcuts:

```toml
[keyboard_shortcuts]
# System controls
"Ctrl+Alt+BackSpace" = "Quit"
"Logo+Q" = "Quit"

# Launch applications
"Logo+Return" = { run = { cmd = "terminator", args = [] } }
"Logo+Space" = { run = { cmd = "dolphin", args = [] } }
"Logo+Shift+B" = { open_default = "browser" }

# Workspace/screen switching
"Logo+1" = { builtin = "Screen", index = 0 }
"Logo+2" = { builtin = "Screen", index = 1 }
# ... (up to Logo+9)

# Display controls
"Logo+Shift+M" = "ScaleDown"
"Logo+Shift+P" = "ScaleUp"
"Logo+Shift+R" = "RotateOutput"

# Window management
"Logo+ArrowUp" = "ToggleMaximizeWindow"
"Alt+W" = "CloseWindow"
"Alt+Shift+W" = "CloseWindow"

# Application switcher
"Alt+Tab" = "ApplicationSwitchNext"
"Alt+Shift+Tab" = "ApplicationSwitchPrev"
"Alt+`" = "ApplicationSwitchNextWindow"
"Alt+Q" = "ApplicationSwitchQuit"

# Expose mode
"Alt+D" = "ExposeShowDesktop"
"Alt+F" = "ExposeShowAll"

# Debug
"Logo+Shift+I" = { run = { cmd = "layers_debug", args = [] } }
"Alt+J" = "SceneSnapshot"
```

### **Available Shortcut Actions**

- `"Quit"` - Exit Otto

**Window Management:**
- `"ToggleMaximizeWindow"` - Maximize/restore focused window
- `"CloseWindow"` - Close focused window

**Application Switching:**
- `"ApplicationSwitchNext"` - Switch to next application
- `"ApplicationSwitchPrev"` - Switch to previous application
- `"ApplicationSwitchNextWindow"` - Switch to next window of current app
- `"ApplicationSwitchQuit"` - Close application switcher

**Expose Mode:**
- `"ExposeShowDesktop"` - Show desktop (minimize all)
- `"ExposeShowAll"` - Show all windows (expose mode)

**Display Controls:**
- `"ScaleDown"` - Decrease display scale
- `"ScaleUp"` - Increase display scale
- `"RotateOutput"` - Rotate display
- `"BrightnessUp"` - Increase screen brightness (requires sysfs access)
- `"BrightnessDown"` - Decrease screen brightness (requires sysfs access)
- `{ builtin = "Screen", index = N }` - Switch to screen N

**Launch Application:**
```toml
{ run = { cmd = "app-name", args = ["arg1", "arg2"] } }
```

**Open Default Application:**
```toml
{ open_default = "browser" }  # or "terminal", "file_manager"
```

---

### Dock Configuration

Customize the dock appearance and bookmarked applications:

```toml
[dock]
# Dock size multiplier (0.5 - 2.0, default: 1.0)
size = 1.0

# Genie effect parameters for minimize animation
genie_scale = 0.5
genie_span = 10.0

# Bookmarked applications
bookmarks = [
  { desktop_id = "org.kde.dolphin.desktop" },
  { desktop_id = "org.mozilla.firefox.desktop", label = "Web", exec_args = ["--private-window"] },
  { desktop_id = "code.desktop" }
]
```

**Dock Bookmark Options**

Each bookmark entry can have:
- `desktop_id` - Desktop file ID (required)
- `label` - Custom display name (optional)
- `exec_args` - Additional command-line arguments (optional)

Desktop files are typically found in:
- `/usr/share/applications/`
- `~/.local/share/applications/`

---

### Night Shift / Color Temperature

Otto supports color temperature adjustment (night shift/blue light reduction) through the `wlr-gamma-control-v1` Wayland protocol. This uses **hardware DRM gamma tables** for efficient, battery-friendly color adjustment with smooth 500ms animated transitions.

**Recommended Tools:**

- **[wlsunset](https://sr.ht/~kennylevinsen/wlsunset/)** - Automatic day/night color temperature adjustment based on time and location
- **[gammastep](https://gitlab.com/chinstrap/gammastep)** - Fork of Redshift with Wayland support
- **[wl-gammactl](https://github.com/mischw/wl-gammactl)** - Manual gamma control tool

**Example: Using wlsunset**

```bash
# Auto-adjust based on location (Oslo, Norway)
wlsunset -l 59.9 -L 10.8

# Manual temperature (warm, 3000K)
wlsunset -T 3000

# Different day/night temperatures
wlsunset -t 3400 -T 6500 -l 59.9 -L 10.8
```

Color changes animate smoothly over 500ms. When the client disconnects, the display smoothly fades back to neutral (6500K).

**Brightness Control:**

Otto also supports screen brightness control via keyboard shortcuts (see example configuration). Brightness is adjusted through system interfaces and works independently of color temperature.

Example shortcuts:
```toml
[keyboard_shortcuts]
"XF86MonBrightnessUp" = "BrightnessUp"
"XF86MonBrightnessDown" = "BrightnessDown"
```

**Battery Efficiency:**

Unlike GPU shader-based implementations, Otto's gamma control uses hardware lookup tables at the display controller level, resulting in negligible power consumption (~0.5-1% vs 3-8% for shader-based approaches).

---

## Tips

1. **Start with the example**: Copy `otto_config.example.toml` to `otto_config.toml` and modify as needed
2. **Backend-specific settings**: Use `otto_config.winit.toml` for development/testing and `otto_config.udev.toml` for production
3. **Icon/cursor themes**: List available themes with `ls /usr/share/icons` and `ls ~/.local/share/icons`
4. **Test shortcuts**: Use simple shortcuts first to verify your configuration is loading correctly
5. **Scaling**: Adjust `screen_scale` based on your display DPI (1.0 for 96 DPI, 2.0 for 192 DPI/HiDPI)

---

## Troubleshooting

**Configuration not loading:**
- Verify the TOML syntax is correct (matching brackets, quotes, commas)
- Check Otto's log output for parsing errors
- Ensure the config file is in the working directory

**Icon/cursor theme not found:**
- Verify the theme is installed: `ls /usr/share/icons/ ~/.local/share/icons/`
- Theme names are case-sensitive
- Some themes may require additional packages

**Keyboard shortcuts not working:**
- Ensure modifier keys use correct names: `Logo`, `Ctrl`, `Alt`, `Shift`
- Key names are case-sensitive
- Some shortcuts may conflict with system bindings

**Touchpad settings ignored:**
- Settings only apply to touchpad devices, not mice
- Some hardware may not support all features
- Check `libinput` capabilities for your device
