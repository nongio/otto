# Otto
A visually-focused desktop system designed around smooth animations, thoughtful gestures, and careful attention to detail, inspired by familiar macOS interactions. 

This system aims to be visually refined and pleasant to use, while at the same time serving as an experimental platform to push the Linux desktop environment forward.

Otto is a Wayland compositor and stacking window manager, built in Rust on top of [LayersEngine](https://github.com/nongio/layers), with Skia used for rendering.

## :information_source: Testing phase
While many features are ready for daily use, there is still some work required for full stability. 
Testing is valuable, so you are invited to play around with Otto.

Feedback and questions are welcome in the Matrix chat: 
[`#otto-compositor:matrix.org`](https://matrix.to/#/#otto-compositor:matrix.org).


 ## :framed_picture: What does Otto look like?

<video src="https://github.com/user-attachments/assets/9abad978-319d-4699-a5a4-f34f8b3e3560" autoplay muted loop playsinline></video>

*Dock task manager showing running applications.*

<video src="https://github.com/user-attachments/assets/014df942-4a79-43f5-9562-73f1858152ba" autoplay muted loop playsinline></video>

*Minimizing windows to the Dock with an animated genie effect.*

<video src="https://github.com/user-attachments/assets/dedfed16-6713-4a70-b5aa-e0057c6d4aad" autoplay muted loop playsinline></video>

*Moving windows between workspaces with drag and drop.*

<video src="https://github.com/user-attachments/assets/eef1a894-b80e-4db0-b638-341bca321fb0" autoplay muted loop playsinline></video>

*Navigating between applications from the Dock.*

<video src="https://github.com/user-attachments/assets/5a2a9cab-8e25-4c69-aeec-d21bed02542f" autoplay muted loop playsinline></video>

*Workspace selector with visual previews.*

<video src="https://github.com/user-attachments/assets/eb631d10-8417-4124-9472-52a9eef9a856" autoplay muted loop playsinline></video>

*Exposé view showing all open windows with smooth animations.*

<video src="https://github.com/user-attachments/assets/62b745c4-f873-4961-91e4-5a1679155fdf" autoplay muted loop playsinline></video>

*Application switcher with icons, names and background blur.*

## Is Otto usable?
Otto is in an early but functional state. You can install it from pre-built packages or build it from source. Many features are still missing.

Testing and issue reports are welcome. Development follows a draft roadmap of planned features and improvements.

## Features and roadmap
- **Window management:** move/resize, fullscreen/maximize (animated), minimize to the Dock (animated), snap to left/right halves, new windows placed where they overlap the least.
- **Workspaces:** multiple workspaces, animated switching, drag windows between workspaces, configurable background; each monitor has its own independent set.
- **Multi-monitor:** per-output rendering, workspaces, fullscreen, Exposé and workspace selector; hotplug, display arrangement and modes from the config, and virtual outputs created on demand.
- **Dock (task manager):** shows running apps, minimized windows and pinned/bookmarked apps, bounces an icon while a launch is in progress.
- **App switcher** (default: `Ctrl+Tab`): searches app metadata/icons (XDG), can close apps, cycles between windows of the same app, appears on the monitor under the pointer.
- **Exposé / overview** (default: `PageDown`, gesture: three-finger swipe up): shows all windows, shows window previews with names, includes “show desktop”.
- **Topbar:** clock, system tray, and application menus exported over DBusMenu.
- **Dynamic island:** notifications, ongoing activities, and system UI (brightness, volume, keyboard backlight) in a floating panel.
- **Session lock:** `ext-session-lock-v1` locking with a PAM-backed locker (`otto-lock`), lock on `Ctrl+Alt+Escape`, on the power button, on lid close, or after an idle timeout — respecting `idle-inhibit` clients.
- **Login greeter:** `otto --login` hosts a login screen (`otto-greeter`) against greetd, with password and fingerprint prompts.
- **Power management:** Otto-owned lid-close suspend with clamshell and remote-session awareness, configurable power-button and lid actions.
- **Input:** natural scrolling, two-finger scrolling, keyboard remapping, configurable shortcuts.
- **Theming:** dark/light.
- **Screen sharing:** XDG Desktop Portal backend + PipeWire, with a permission dialog, dmabuf modifier negotiation, and AirPlay receivers as a target.
- **Remote desktop:** `otto-rdp` serves a virtual output over RDP with TLS, remote input, and hardware H.264 encoding where available.
- **XWayland:** X11 apps including fullscreen games — keyboard focus for globally-active clients, output scale via XSETTINGS, direct scanout.
- **Rendering:** Skia pipeline with KMS multi-plane scanout (dock, app switcher, popups and topmost windows on their own planes) and cross-plane backdrop blur.

### Still to come
 - **Screen capture:** per-window capture and screenshots.
 - **Multi-monitor:** display mirroring.
 - **Dock improvements:** favorite locations; move Dock code out of compositor core.
 - **Input polish:** scroll acceleration.

 ### Experimentation
- **Scene graph protocol:** WIP protocol ([otto-surface-style-unstable-v1](protocols/otto-surface-style-unstable-v1.xml)) exposing the scene graph and its animations to clients — size, position, corner radius, blur and shadow driven by compositor-side springs, a Core Animation-like model. The topbar and the dynamic island are built on it.

## Supported Wayland Protocols
Otto implements a comprehensive set of Wayland protocols, including:
- Core: `wl_compositor`, `wl_subcompositor`, `wl_shm`, `wl_seat`, `wl_data_device_manager`
- Shells: `xdg_wm_base` (XDG shell), `xdg_decoration_manager_v1`, `wlr_layer_shell_v1` (Layer shell 1.0), `xwayland_shell_v1`
- Output management: `wl_output`, `xdg_output`, `wp_presentation`, `wp_fractional_scale_v1`, `wp_viewporter`
- Rendering and DRM: `zwp_linux_dmabuf_v1`, `wp_linux_drm_syncobj_v1` (explicit sync), `wp_drm_lease_device_v1`
- Input: pointer gestures, relative pointer, pointer constraints, tablet, `wp_cursor_shape_v1`, keyboard shortcuts inhibit, text input, input method, virtual keyboard, `zwlr_virtual_pointer_v1`, XWayland keyboard grab
- Selection: primary selection, data control (wlr-data-control)
- Session: `ext_session_lock_v1`, `zwp_idle_inhibit_manager_v1`, `xdg_activation_v1`, security context
- Window listing: `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_management_v1`
- Capture: `zwlr_screencopy_v1`
- XDG foreign: cross-client surface identification
- Display control: `zwlr_gamma_control_v1` (color temperature/night shift with hardware gamma tables)
- Otto extensions: [`otto-surface-style-unstable-v1`](protocols/otto-surface-style-unstable-v1.xml), [`otto-dock-v1`](protocols/otto-dock-v1.xml)

For where each one is implemented and how to trace it through the code, see [docs/developer/wayland.md](./docs/developer/wayland.md).

## Development

Otto consists of the main compositor and additional components:

| Component | Description |
|-----------|-------------|
| `otto` | Main compositor binary |
| `otto-bar` | Topbar: clock, tray and application menus |
| `otto-islands` | Dynamic island: notifications, activities and dialogs |
| `otto-lock` | PAM-backed screen locker (`ext-session-lock-v1`) |
| `otto-greeter` | Login screen client speaking greetd's IPC |
| `otto-auth-ui` | Authentication panel shared by the locker and the greeter |
| `otto-rdp` | RDP bridge serving a virtual output to a remote client |
| `otto-kit` | UI toolkit the Otto clients are built on |
| `apps-manager` | Application launcher |
| `xdg-desktop-portal-otto` | XDG Desktop Portal backend for screen sharing |

Each lives under `components/`, and can be built on its own with `cargo build -p <name>`.

## How can you contribute?
Both this project and the LayersEngine are open to contributions. Contribute by testing the compositor, reporting bugs, implementing new features, or bringing new ideas. If you have any questions, open an issue on the repository.

The repository provides [AGENTS.md](AGENTS.md), automated code review instructions, and developer documentation to support both human contributors and coding agents.

## Installation

### Download Pre-built Packages

Pre-built packages are available from the [GitHub Releases](https://github.com/nongio/otto/releases) page.

#### Debian/Ubuntu (`.deb`)

```bash
# Download the .deb package from releases, then:
sudo dpkg -i otto_*.deb
sudo apt-get install -f  # Install dependencies if needed
```

#### Fedora/RHEL (`.rpm`)

```bash
# Download the .rpm package from releases, then:
sudo dnf install otto-*.rpm
# or
sudo rpm -i otto-*.rpm
```

#### Arch Linux

```bash
# Download PKGBUILD and let makepkg fetch the tarball automatically:
curl -O https://raw.githubusercontent.com/nongio/otto/main/PKGBUILD
makepkg -si
```

If you already downloaded the tarball from GitHub Releases, put the PKGBUILD in the same directory — `makepkg` will use it without re-downloading:
```bash
cd ~/Downloads  # wherever your otto-*-x86_64.tar.gz is
curl -O https://raw.githubusercontent.com/nongio/otto/main/PKGBUILD
makepkg -si
```

### After Installation

Once installed, Otto will appear in your login manager (GDM, SDDM, LightDM, etc.) as "Otto" in the session selection menu. Simply select it and log in.

**Note:** Screen sharing functionality requires `xdg-desktop-portal` to be installed on your system.

**Note:** Using Otto as the login screen (`otto --login` with `otto-greeter`) requires `greetd`. On Debian/Ubuntu, copy the shipped `otto-lock.pam` example to `/etc/pam.d/otto-lock` before using the screen locker — the packages for Arch and Fedora install it for you.

**Note:** Otto handles the lid switch and the power button itself. For those to work, set `HandleLidSwitch=ignore` and `HandlePowerKey=ignore` in `logind.conf`.

## Building Otto

### Prerequisites
You'll need to install the following dependencies (note that these package
names may vary depending on your OS and Linux distribution):
- `libwayland`
- `libxkbcommon`
- `libudev`
- `libinput`
- `libgbm`
- [`libseat`](https://git.sr.ht/~kennylevinsen/seatd)

If you want to enable X11 support (to run X11 applications within Otto),
you'll need to install the `xwayland` package as well.

### Build and run

You can run Otto with cargo after having cloned this repository:

```bash
cd otto

# Run Otto (auto-detects backend)
cargo run

# Run with development features (debugger, profiler)
cargo run --features "dev"

# Release build
cargo build --release
cargo run --release
```

Otto automatically detects the best backend for your environment:
- If running inside a Wayland session, it uses `--winit` (runs as a window)
- If running in a TTY, it uses `--tty-udev` (bare metal display)

**You can force a specific backend** by passing it as an argument:

- `--tty-udev`: start Otto in a tty with `udev` support. This is the "traditional" launch of a Wayland
  compositor. Note that this might require you to start Otto as root if your system does not have `logind`
  available.
- `--winit`: start Otto as a [Winit](https://github.com/tomaka/winit) application. This allows you to run it
  inside another X11 or Wayland session — useful for development.
- `--x11`: start Otto as an X11 client. This allows you to run the compositor inside an X11 session or any
  compositor supporting XWayland. This implementation is quite basic and is not really maintained.


## Configure Otto

Otto uses TOML configuration files and follows standard Linux configuration paths.

### Configuration Locations

Otto searches for configuration files in the following order (later files override earlier ones):

1. **System config**: `/etc/otto/config.toml`
2. **User config**: `$XDG_CONFIG_HOME/otto/config.toml` (defaults to `~/.config/otto/config.toml`)
3. **Local override**: `./otto_config.toml` (current directory, for development)
4. **Backend-specific**: `./otto_config.{backend}.toml` (highest priority)

A complete example configuration is provided in `otto_config.example.toml`. To get started:

```bash
# Create user config directory
mkdir -p ~/.config/otto

# Copy example config
cp otto_config.example.toml ~/.config/otto/config.toml

# Edit as needed
$EDITOR ~/.config/otto/config.toml
```

### Backend-specific Configuration

You can create backend-specific configuration files for development using the naming convention `otto_config.{backend}.toml` in the current directory:

- `otto_config.winit.toml` - Configuration for the winit backend
- `otto_config.udev.toml` - Configuration for the tty-udev/DRM backend

Backend-specific configs have the highest priority and override all other configuration files. This allows you to maintain different display settings, keyboard shortcuts, or other preferences for each backend. For instance, you might want different `screen_scale` values when running in a window (winit/X11) versus on bare metal (tty-udev).

For detailed configuration options, see the [configuration documentation](./docs/user/configuration.md).

### Keyboard Shortcuts
Hotkeys are now fully configurable via the `otto_config.toml` file. See the `[keyboard_shortcuts]` section to customize keybindings for your setup. Example:

```toml
[keyboard_shortcuts]
"Ctrl+Alt+BackSpace" = "Quit"
"Ctrl+Shift+Q" = "Quit"
"Ctrl+Return" = { run = { cmd = "terminator", args = [] } }
"Logo+Space" = { open_default = "file_manager" }
"Logo+B" = { open_default = "browser" }
"Ctrl+1" = { builtin = "Workspace", index = 0 }
```

## Profiling

Otto includes built-in support for profiling using [puffin](https://github.com/EmbarkStudios/puffin). The profiler is enabled by default via the `profile` feature.

### Using the Profiler

1. **Run the compositor** — The puffin HTTP server starts automatically on port 8585:
   ```bash
   cargo run -- --winit
   ```

2. **Install `puffin_viewer`** (if you haven't already):
   ```bash
   cargo install puffin_viewer
   ```

3. **Connect to the profiler**:
   - Launch `puffin_viewer`
   - Connect to `127.0.0.1:8585`

The profiler will show frame timing, render performance and other metrics to help identify performance bottlenecks.

**Note:** Make sure your `puffin_viewer` version matches the puffin version used by Otto (0.19.x requires puffin_viewer 0.22.0 or later).


### Credits
- Icons used: [Fluent Icon Theme](https://github.com/vinceliuice/Fluent-icon-theme)
- Font used: [Inter Font](https://rsms.me/inter/)
- Background used: Zach Lieberman Soft Circle Study #6 2024 [zach.li](http://zach.li/)
