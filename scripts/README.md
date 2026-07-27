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

### `test-login-mode.sh`
Test Otto's login mode (`otto --login`) and the otto-greeter client.
See [`specs/login-mode.md`](../specs/login-mode.md).

Tests are grouped by what they need from the environment — most of the feature
can be exercised without root or a spare VT, using a bundled fake greetd daemon
that speaks the real wire protocol.

**Usage:**
```bash
# Everything that runs headlessly: fmt, clippy, unit tests, build, IPC conversation
./scripts/test-login-mode.sh

# Individual groups
./scripts/test-login-mode.sh check     # static checks + unit tests
./scripts/test-login-mode.sh ipc       # greetd wire protocol, no display needed
./scripts/test-login-mode.sh mock      # greeter UI, built-in mock backend
./scripts/test-login-mode.sh greeter   # greeter UI against a fake greetd
./scripts/test-login-mode.sh nested    # full login mode inside a window
./scripts/test-login-mode.sh tty       # full login mode on the console
./scripts/test-login-mode.sh greetd    # generate a real greetd config

# Exercise a different PAM conversation shape
FAKE_GREETD_SCENARIO=two-factor ./scripts/test-login-mode.sh greeter
FAKE_GREETD_SCENARIO=locked ./scripts/test-login-mode.sh greeter
```

Scenarios: `simple`, `fingerprint` (default — sends an unanswerable info message
first, as `pam_fprintd` does), `two-factor`, `locked`.

### `otto-session`
Wrapper for the `Exec` line of a `wayland-sessions` entry, so Otto can be
launched by a display manager without printing to the console.

greetd hands the session the VT as its stdio, and the VT is in text mode until
Otto modesets — a compositor logging at `info` therefore flashes a terminal
between the greeter and the desktop. This redirects the log to
`$XDG_STATE_HOME/otto/session.log` (one generation kept) instead.

```bash
sudo install -Dm755 scripts/otto-session /usr/local/bin/otto-session
# then point Exec= at it:
#   /usr/share/wayland-sessions/otto-current.desktop
```

`$OTTO_BIN` overrides which binary is run (default `/usr/local/bin/otto`).

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
