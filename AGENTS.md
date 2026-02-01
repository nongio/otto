# AGENTS.md

This file provides guidance to agents when working with code in this repository.

## Build & Run Commands for Otto

```sh
# Development build
cargo build
cargo run -- --winit   # Run in windowed mode (Wayland/X11 session)

# Development with debugging/profiling tools
cargo build --features "dev"
cargo run --features "dev" -- --winit

# Run on different backends
cargo run -- --x11     # Run as X11 client
cargo run -- --tty-udev # Run on bare metal (DRM/GBM, requires root or libseat)

# Release build (optimized)
cargo build --release
cargo run --release -- --winit

# Release build with optional features
cargo build --release --features "metrics,ticker"

# Linting and formatting
cargo fmt --all -- --check   # Check formatting
cargo fmt --all              # Auto-format
cargo clippy --features "default" -- -D warnings

# Run with tracing
RUST_LOG=debug cargo run -- --winit

# Run logging into a file
RUST_LOG=debug cargo run -- --winit 2> winit.log
```

## Build apps and tools
when working with the apps/tools in `components/apps-manager` and `components/xdg-desktop-portal-otto`, use these commands:
```sh
cargo build -p apps-manager

cargo run -p apps-manager

cargo build -p xdg-desktop-portal-otto
cargo run -p xdg-desktop-portal-otto
```

sometimes we need to test a component together with Otto, in that case use:
```sh
# First, run Otto in one terminal
cargo run -- --winit &
# Then, in another terminal, run the app/tool
WAYLAND_DISPLAY=wayland-1 cargo run -p apps-manager
```

**Note:** No test suite exists yet. The project uses Rust 1.83.0 minimum.

## Architecture Overview

Otto is a Wayland compositor built on Smithay with a Skia-based rendering pipeline and the `lay-rs` engine for scene graph/layout management.

### Backend System

Three interchangeable backends implement the same compositor logic:
- `src/udev.rs` — Production backend using DRM/GBM/libinput for bare-metal display
- `src/winit.rs` — Development backend running as a window inside another compositor

Each backend:
1. Sets up its display/input subsystem
2. Creates `Otto<BackendData>` state
3. Runs the event loop with calloop
4. Calls the shared rendering pipeline

### Core State (`src/state/mod.rs`)

`Otto<BackendData>` is the central compositor state containing:
- Wayland protocol handlers (via Smithay delegates)
- `Workspaces` — multi-workspace window management with dock, app switcher, expose mode
- `PopupManager` — popup surface management
- Seat/input state, output management, layer shell surfaces

The state module also contains protocol handler implementations (`*_handler.rs` files).

### Rendering Pipeline

1. **Scene Graph**: `lay-rs` engine manages the scene tree and Taffy-based layout
2. **Element Building**: `src/render.rs` produces `OutputRenderElements` per output
3. **Skia Renderer**: `src/skia_renderer.rs` with modular sub-components:
   - `src/renderer/skia_surface.rs` — Skia surface creation and management
   - `src/renderer/textures.rs` — Texture types combining OpenGL and Skia
   - `src/renderer/sync.rs` — GPU synchronization using EGL fences
   - `src/renderer/egl_context.rs` — EGL surface wrappers
4. **Damage Tracking**: `OutputDamageTracker` from Smithay renders only damaged regions
5. **Frame Submission**: Backend submits the composed buffer (dmabuf on DRM, presented on winit/x11)

### Window Management

- `src/shell/` — Protocol implementations for XDG shell, layer shell, XWayland
- `src/workspaces/` — Workspace logic, window views, dock, app switcher, expose mode
- `src/workspaces/window_view/` — Individual window rendering and effects (genie minimize)

### Screenshare System

Located in `src/screenshare/`:
- `mod.rs` — Session state management and command handlers
- `dbus_service.rs` — D-Bus API (`org.otto.ScreenCast`)
- `frame_tap.rs` — Frame capture hooks with damage tracking
- `pipewire_stream.rs` — PipeWire stream with SHM buffer handling
- `session_tap.rs` — Per-session frame filtering

Portal backend: `components/xdg-desktop-portal-otto/` — separate binary that bridges xdg-desktop-portal to compositor

See [docs/developer/screenshare.md](./docs/developer/screenshare.md) for detailed architecture documentation.

## Configuration

TOML-based config at runtime:
- `otto_config.toml` — Default configuration
- `otto_config.{backend}.toml` — Backend-specific overrides (e.g., `otto_config.winit.toml`)

See `otto_config.example.toml` for all options.

## Key Dependencies

- **smithay** — Wayland compositor library (pinned to specific git rev)
- **lay-rs** — Scene graph and layout engine (from `github.com/nongio/layers`)
- **zbus** — D-Bus implementation for screenshare
- **pipewire** — Video streaming for screenshare
- **tokio** — Async runtime for D-Bus service

## Documentation

Otto maintains two documentation tiers:
- `docs/user/` — End-user configuration and usage guides
- `docs/developer/` — Architecture, design docs, and implementation details
- `website/` — Hugo static site generator (auto-builds from markdown)

**For detailed guidance on writing and updating documentation, see:**
- [`.github/instructions/documentation.instructions.md`](./.github/instructions/documentation.instructions.md)

**For code review guidelines, see:**
- [`.github/instructions/review.instructions.md`](./.github/instructions/review.instructions.md)

Key developer docs:
- `rendering.md`, `render_loop.md` — Rendering pipeline
- `wayland.md` — Protocol implementation details
- `screenshare.md` — Screen sharing architecture and D-Bus API
- `expose.md`, `dock-design.md` — UI component design
- `sc-layer-protocol-design.md` — Scene graph protocol design

User documentation in `docs/user/`:
- `configuration.md` — Configuration options
- `clipboard.md` — Clipboard usage
- `project-structure.md` — Codebase organization
