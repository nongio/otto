# Otto Helper Scripts

This directory contains helper scripts for working with Otto compositor.

## Keyboard Configuration Scripts

### `show-keys.sh` - Real-time Keyboard Event Viewer

Shows what keys you're pressing in real-time, useful for:
- Verifying XKB remapping is working (e.g., Caps Lock → Escape)
- Testing keyboard shortcuts
- Understanding modifier key behavior
- Debugging input issues

**Usage:**
```bash
./scripts/show-keys.sh
```

Then press keys to see them displayed. Example output:
```
Ctrl+Alt+f                     
Shift+a                        (prints: 'A')
Escape                         
Logo+Space                     
```

Press `Ctrl+C` to exit.

**Requirements:** `wev` (recommended) or `xkbcli` (libxkbcommon-tools)

---

### `check-xkb-config.sh` - XKB Configuration Inspector

Check your current XKB keyboard configuration and learn about available options.

**Usage:**
```bash
./scripts/check-xkb-config.sh
```

Shows:
- Current system XKB configuration
- Active Wayland keymap (if Otto is running)
- Available XKB tools and commands
- Example configurations

**What it tells you:**
- Which layout you're using (us, dvorak, etc.)
- Which XKB options are active
- How to list all available options
- Where to configure keyboard in Otto

---

## Other Scripts

### `start_session.sh`
Start Otto with a full session environment (D-Bus, pipewire, etc.)

### `test-screenshare.sh`
Test Otto's screen sharing functionality

**Usage:**
```bash
# Default output: eDP-1, opens GStreamer playback automatically
./scripts/test-screenshare.sh

# Select an explicit output from ListOutputs (example: virtual-1)
./scripts/test-screenshare.sh virtual-1

# Optionally attempt OpenPipeWireRemote too
OPEN_PIPEWIRE=1 ./scripts/test-screenshare.sh

# Open stream live with ffplay (requires pipewire input support in ffmpeg)
PLAYER=ffplay ./scripts/test-screenshare.sh eDP-1

# Keep stream session alive without opening a player
PLAYER=none ./scripts/test-screenshare.sh eDP-1
```

### Session helper scripts
- `dbus.sh` - D-Bus session management
- `pipewire.sh` - PipeWire audio setup
- `portal.sh` - XDG Desktop Portal setup
- `kwallet.sh` - KWallet integration
- `wifi.sh` - WiFi management

---

## Tips

**Testing keyboard remapping:**
1. Edit `otto_config.toml` under `[input]` section
2. Set your `xkb_options` (e.g., `["caps:escape"]`)
3. Restart Otto to apply changes
4. Run `./scripts/show-keys.sh` to verify
5. Press Caps Lock - it should show as "Escape"

**Finding XKB options:**
```bash
# List all available options
xkbcli list

# Search for specific options
cat /usr/share/X11/xkb/rules/base.lst | grep -A5 "ctrl:"
cat /usr/share/X11/xkb/rules/base.lst | grep -A5 "caps:"

# Read detailed documentation
man xkeyboard-config
```

**Common XKB options:**
- `caps:escape` - Caps Lock becomes Escape
- `ctrl:swapcaps` - Swap Ctrl and Caps Lock
- `altwin:ctrl_win` - Win/Super key acts as Ctrl (Mac-like)
- `compose:ralt` - Right Alt as Compose key for accents
- `grp:win_space_toggle` - Win+Space to switch layouts
