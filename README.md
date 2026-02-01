# Otto
A visually-focused desktop system designed around smooth animations, thoughtful gestures and careful attention to detail, inspired by familiar macOS interactions. 

This system aims to be visually refined and pleasant to use, while at the same time, serving as an experimental platform to push the Linux desktop environment forward.

Otto is a Wayland compositor and stacking window manager, built in Rust on top of [LayersEngine](https://github.com/nongio/layers) and uses Skia for rendering.

## :information_source: Testing phase
While many features are ready for daily use, there is still some work required for full stability. 
Testing is valuable therefore you are invited to play around with Otto.

Feedback and questions are welcome in the Matrix chat: 
[`#otto-compositor:matrix.org`](https://matrix.to/#/#otto-compositor:matrix.org).


 ## :framed_picture: What does Otto look like?

<video src="https://github.com/user-attachments/assets/9abad978-319d-4699-a5a4-f34f8b3e3560" autoplay muted loop playsinline></video>

*Dock task manager showing running applications.*

<video src="https://github.com/user-attachments/assets/014df942-4a79-43f5-9562-73f1858152ba" autoplay muted loop playsinline></video>

*Minimising windows with animated genie effect to the Dock.*

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
Otto is in an early but functional state and can be tested by building it from source. Many features are still missing, and the project is not yet packaged for any distribution.

Testing and issue reports are welcome. Development follows a draft roadmap of planned features and improvements.

## Features and roadmap

- **Window management:** move/resize, fullscreen/maximize (animated), minimise to the Dock (animated).
- **Workspaces:** multiple workspaces, animated switching, drag windows between workspaces, configurable background.
- **Dock (task manager):** shows running apps, minimised windows and pinned/bookmarked apps.
- **App switcher** (default: `Ctrl+Tab`): searches app metadata/icons (XDG), can close apps, cycles between windows of the same app.
- **Exposé / overview** (default: `PageDown`, gesture: three-finger swipe up): shows all windows, shows window previews with names, includes “show desktop”.
- **Input:** natural scrolling, two-finger scrolling, keyboard remapping.
- **Theming:** dark/light.
- **Screen sharing:** works through an XDG Desktop Portal backend + PipeWire (full-screen capture via GPU blit + dmabuf).

### Still to come
 - **Multi-monitor:** multiple screens.
 - **Screen capture:** per-window capture, screenshots, and a permission dialog UI.
 - **Session management:** lockscreen / login with libseat integration.
 - **Topbar:** application menus and system integration.
 - **Dock improvements:** favorite locations; move Dock code out of compositor core.
 - **System UI:** brightness, volume, keyboard backlight widgets; notifications.
 - **Input polish:** scroll acceleration.

 ### Experimentation
- **Scene graph protocol:** WIP protocol ([sc-layer-v1](protocols/sc-layer-v1.xml)) to expose the scene graph and animations to external clients for advanced UI customisation and effects.
- **Ideas:** remote "virtual screens" (VNC/RDP).

## Supported Wayland Protocols
Otto implements a comprehensive set of Wayland protocols, including:
- Core: `wl_compositor`, `wl_shm`, `wl_seat`, `wl_data_device_manager`
- Shells: `xdg_wm_base` (XDG shell), `wlr_layer_shell_v1` (Layer shell 1.0)
- Output management: `wl_output`, `xdg_output`, `wp_presentation`
- Rendering: `zwp_linux_dmabuf_v1`, `wp_viewporter`
- Input: pointer gestures, relative pointer, keyboard shortcuts inhibit, text input, input method
- Selection: primary selection, data control (wlr-data-control)
- XDG foreign: cross-client surface identification

For a complete and up-to-date list, see [docs/developer/wayland.md](./docs/developer/wayland.md).

## Development

Otto consists of the main compositor and additional components:

| Component | Description |
|-----------|-------------|
| `otto` | Main compositor binary |
| `xdg-desktop-portal-otto` | XDG Desktop Portal backend for screen sharing |

The portal backend is located in `components/xdg-desktop-portal-otto/`.

## How can you contribute?
Both this project and the LayersEngine are open to contributions. Contribute by testing the compositor, reporting bugs, by implementing new features or by bringing new ideas. If you have any questions, open an issue on the repository.

The repository provides AGENTS.md, automated code review instructions and developer documentation to support both human contributors and coding agents.

## Building Otto

### Prerequisites
You'll need to install the following dependencies (note, that those package
names may vary depending on your OS and linux distribution):
- `libwayland`
- `libxkbcommon`
- `libudev`
- `libinput`
- `libgbm`
- [`libseat`](https://git.sr.ht/~kennylevinsen/seatd)

If you want to enable X11 support (to run X11 applications within Otto),
then you'll need to install the following packages as well:
    - `xwayland`

### Build and run

You can run Otto with cargo after having cloned this repository:

```bash
cd otto

# Run with default features (lean)
cargo run -- --winit

# Run with development features (debugger, profiler)
cargo run --features "dev" -- --winit

# Release build
cargo build --release
cargo run --release -- --winit
```

Current available backends:

- `--tty-udev`: start Otto in a tty with udev support. This is the "traditional" launch of a Wayland
  compositor. Note that this might require you to start Otto as root if your system does not have logind
  available.
- `--winit`: start Otto as a [Winit](https://github.com/tomaka/winit) application. This allows you to run it
  inside of an other X11 or Wayland session, useful for developemnt.
- `--x11`: start Otto as an X11 client. This allows you to run the compositor inside an X11 session or any compositor supporting XWayland. This implementation is quite basic and not really maintaned.


## Configure Otto

Otto uses TOML configuration files. A complete example configuration is provided in `otto_config.example.toml` which you can copy and modify:

```bash
cp otto_config.example.toml otto_config.toml
```

### Backend-specific configuration

You can create backend-specific configuration files using the naming convention `otto_config.{backend}.toml`. For example:

- `otto_config.winit.toml` - Configuration for the winit backend
- `otto_config.udev.toml` - Configuration for the tty-udev/DRM backend

When running with a specific backend, Otto will automatically load the corresponding configuration file if it exists, falling back to `otto_config.toml` otherwise.

This allows you to maintain different display settings, keyboard shortcuts or other preferences for each backend. For instance, you might want different `screen_scale` values or display resolutions when running in a window (winit/X11) versus on bare metal (tty-udev).

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

Otto includes built-in support for profiling using [puffin](https://github.com/EmbarkStudios/puffin). The profiler is enabled by default through the `profile` feature.

### Using the Profiler

1. **Run the compositor** - The puffin HTTP server starts automatically on port 8585:
   ```bash
   cargo run -- --winit
   ```

2. **Install puffin_viewer** (if you haven't already):
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
