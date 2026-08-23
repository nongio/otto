# Clipboard Management

Why clipboard contents vanish when you close an app on Wayland, and which
clipboard manager to run so they do not.

## Clipboard Persistence Issue

By default in Wayland compositors (including Otto), when you copy text from an application and then close or crash that application, the clipboard content is lost. This is because Wayland requires the source client to remain active to serve clipboard data on paste.

## Solution: External Clipboard Manager

Otto implements the **wlr-data-control** protocol, which allows external clipboard managers to monitor and cache clipboard content. This ensures clipboard data persists even after the source application closes.

## Recommended Clipboard Managers

### 1. wl-clip-persist (Recommended for basic persistence)

Purpose-built for clipboard persistence without additional features.

**Installation:**
```sh
cargo install wl-clip-persist
# OR
yay -S wl-clip-persist  # Arch AUR
```

**Usage:**
```sh
wl-clip-persist --clipboard regular &
```

### 2. cliphist (Recommended for clipboard history)

Lightweight clipboard manager with history support.

**Installation:**
```sh
yay -S cliphist  # Arch AUR
# OR build from https://github.com/sentriz/cliphist
```

**Usage:**
```sh
# Start the daemon to watch clipboard
wl-paste --watch cliphist store &

# Later, select from history (requires fuzzy finder like wofi/rofi)
cliphist list | wofi --dmenu | cliphist decode | wl-copy
```

### 3. clipman (Alternative with history)

Another clipboard manager with history support.

**Installation:**
```sh
yay -S clipman  # Arch AUR
go install github.com/yory8/clipman@latest
```

**Usage:**
```sh
wl-paste -t text --watch clipman store &
```

## Auto-start with Otto

Add your chosen clipboard manager to your shell startup script or compositor autostart configuration:

```sh
# ~/.bashrc or startup script
if [ "$XDG_SESSION_TYPE" = "wayland" ]; then
    wl-clip-persist --clipboard regular &
fi
```
