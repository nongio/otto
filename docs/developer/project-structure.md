## Project Structure

Otto is a Cargo workspace. The root crate is the compositor itself; everything
under `components/` is a separate binary that talks to the compositor over
Wayland and D-Bus like any other client.

That split is deliberate and worth internalising early: **the top bar, the
launcher, the lock screen and the greeter are not part of the compositor.**
They are ordinary Wayland clients built with `otto-kit`. Only the dock,
exposé, the app switcher and window decorations are drawn server-side.

### The compositor crate

```
src/
├── main.rs              Entry point, backend selection
├── lib.rs               Library exports
├── state/               The big compositor state + most protocol handlers
├── shell/               Window management: xdg-shell, layer-shell, XWayland, decorations
├── workspaces/          Workspaces, window views, dock, app switcher, exposé
│   ├── dock/            The compositor-drawn dock
│   ├── app_switcher/    Cmd-Tab switcher
│   └── window_view/     Per-window rendering and effects (genie minimize)
├── input/               Keyboard and pointer event handling
├── input_handler.rs     Seat wiring, hit-testing, focus routing
├── focus.rs             What currently has keyboard/pointer focus
├── config/              TOML parsing, layered config, shortcuts
├── theme/               Colour palettes and text styles
├── render.rs            Builds the per-output render element list
├── render_elements/     Render element types handed to the damage tracker
├── skia_renderer.rs     Skia wrapper over Smithay's GlesRenderer
├── renderer/            EGL surfaces, textures, GPU sync
├── udev/                Production backend: DRM/GBM/libinput (see below)
├── winit.rs             Development backend: run inside another compositor
├── x11.rs               X11 client backend (basic, not actively maintained)
├── headless.rs          Headless backend used by the integration tests
├── screenshare/         PipeWire screen capture + org.otto.ScreenCast
├── virtual_output/      Virtual (non-physical) outputs
├── settings/            Setting schema and storage
├── settings_service.rs  org.otto.Settings D-Bus service
├── surface_style/       otto-surface-style-v1: client-driven layer styling
├── otto_dock/           otto-dock-v1: lets clients contribute dock items
├── audio/               Volume and output device handling
├── lock.rs              ext-session-lock-v1
├── login.rs             greetd session handoff
└── utils/               Shared helpers (natural layout, geometry, …)
```

`src/udev/` is a directory, not a file — older docs and comments still say
`src/udev.rs`. The interesting parts are `render.rs` (the frame path, by far
the largest), `planes.rs` and `backdrop.rs` (hardware plane scanout and
cross-plane blur), `init.rs` and `device.rs` (DRM setup and hotplug).

### Components

Each is a standalone binary in `components/`:

| Component | What it is |
|-----------|------------|
| `otto-kit` | The UI toolkit every other component is built on — window/surface plumbing, theme, typography, icons, widgets |
| `otto-bar` | The top bar: clock, tray, global menus |
| `otto-islands` | The dynamic island: notifications, HUD, permission dialogs |
| `otto-launcher` | Keyboard-driven app and window launcher |
| `otto-settings` | Settings application |
| `otto-lock` | Lock screen, backed by PAM |
| `otto-greeter` | Login screen, backed by greetd |
| `otto-auth-ui` | The password panel shared by the greeter and the lock screen |
| `otto-rdp` | Serves a virtual output over RDP — see [rdp-virtual-output.md](rdp-virtual-output.md) |
| `xdg-desktop-portal-otto` | Portal backend: screencast, screenshot, settings, access dialogs |
| `apps-manager` | Application launcher/manager |

### Other top-level directories

```
protocols/       Wayland protocol XML (otto-surface-style, otto-dock, wlr-*)
docs/            user/ and developer/ guides
specs/           Behavioural specs — the contract each feature must meet
tests/           Integration tests that drive the headless backend
sample-clients/  Minimal Wayland clients for testing protocol behaviour
assets/          Icons and images
resources/       Runtime resources (cursors, …)
website/         Hugo site generated from docs/
```

### Build & run

```sh
cargo run -- --winit          # windowed, inside an existing session — the dev path
cargo run -- --tty-udev       # bare metal DRM/GBM — needs root or libseat
cargo run -- --x11            # as an X11 client
cargo build --release
```

Build a single component with `-p`:

```sh
cargo build -p otto-kit
cargo run -p otto-settings
```

To run a component against a development compositor, point it at the
compositor's socket (Otto takes the next free one, usually `wayland-1` when
you are already inside a session):

```sh
cargo run -- --winit &
WAYLAND_DISPLAY=wayland-1 cargo run -p otto-bar
```

Minimum supported Rust is 1.87.0 for the compositor; building the whole
workspace needs 1.96.0, which `otto-rdp` pins through GStreamer.

### Feature flags

The canonical list is `[features]` in the workspace `Cargo.toml`. The default
build is deliberately lean — enable developer tooling on demand:

```sh
cargo run --features dev -- --winit
```

| Feature | Effect |
|---------|--------|
| `dev` | Convenience: `debug` + `profile` + `debugger` |
| `debug` | Debug-only functionality, including RenderDoc capture |
| `debugger` | lay-rs scene debugger on `localhost:8000` — needed to inspect the live scene graph |
| `debug-kms` | Extra KMS/plane logging; opt in explicitly |
| `profile` | Puffin profiling (compositor + lay-rs) |
| `profile-with-tracy`, `profile-with-tracy-mem` | Tracy profiling, optionally with memory tracking |
| `perf-counters` | Extra per-frame statistics logging |
| `ticker` | On-screen FPS counter |
| `metrics` | Render metrics collection |

Backends are features too — `udev`, `winit`, `x11`, plus `xwayland` and `egl`.
`default` enables everything except `x11`.
