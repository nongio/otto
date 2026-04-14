# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```sh
# Development build (lean default, add dev features on demand)
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

## Building Components

The workspace includes standalone components built with `-p`:
```sh
cargo build -p otto-kit
cargo build -p topbar
cargo build -p apps-manager
cargo build -p xdg-desktop-portal-otto
```

To test a component together with the compositor:
```sh
# First, run Otto in one terminal
cargo run -- --winit &
# Then, in another terminal, run the component
WAYLAND_DISPLAY=wayland-1 cargo run -p apps-manager
```

**Note:** Minimum supported Rust version is 1.85.0.

## Architecture Overview

Otto is a Wayland compositor built on Smithay with a Skia-based rendering pipeline and the `lay-rs` engine (from `github.com/nongio/layers`) for scene graph/layout management.

### Backend System

Interchangeable backends implement the same compositor logic:
- `src/udev.rs` — Production backend using DRM/GBM/libinput for bare-metal display
- `src/winit.rs` — Development backend running as a window inside another compositor
- `src/headless.rs` — Headless backend for integration tests (no GPU, feature-gated `headless`)
- `src/x11.rs` — X11 client backend (basic, not actively maintained)

Each backend sets up its display/input subsystem, creates `Otto<BackendData>` state, runs the event loop with calloop, and calls the shared rendering pipeline.

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
3. **Skia Renderer**: `src/skia_renderer.rs` with sub-components:
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

### Components

- `components/otto-kit/` — UI toolkit for building Otto apps (menu bars, context menus, popups)
- `components/otto-bar/` — Top menu bar component
- `components/apps-manager/` — Application launcher/manager
- `components/xdg-desktop-portal-otto/` — Portal backend bridging xdg-desktop-portal to compositor

### Screenshare System

Located in `src/screenshare/`. See [docs/developer/screenshare.md](./docs/developer/screenshare.md) for detailed architecture.

## Coordinate Systems & Naming Conventions

Otto has two coordinate spaces — mixing them causes subtle scale-dependent bugs.

- **Physical pixels** — raw hardware pixels. Used for layer positions (`set_position`, `change_position`) and `output.current_mode().size`.
- **Logical pixels (points)** — physical ÷ scale. `output_geometry(output).size` returns logical pixels — **do not use this for layer positions**.

Always use the **per-output scale**: `output.current_scale().fractional_scale() as f32`.
`WorkspacesModel.scale` is a global fallback only — avoid it in geometry code.

**Naming convention:** suffix physical-pixel variables with `_px` (e.g. `width_px`, `offset_px`) to make the space explicit.

## Configuration

TOML-based config at runtime:
- `otto_config.toml` — Default configuration
- `otto_config.{backend}.toml` — Backend-specific overrides (e.g., `otto_config.winit.toml`)

See `otto_config.example.toml` for all options.

## Git Commit Messages

Commits are parsed by [git-cliff](https://git-cliff.org) to generate `CHANGELOG.md`, so follow [Conventional Commits](https://www.conventionalcommits.org):

```
<type>[optional scope]: <short description>
```

Common types: `feat`, `fix`, `refactor`, `doc`, `perf`, `style`, `test`, `chore`, `ci`.
Keep the subject line short (50 chars or fewer). Omit a body unless the change genuinely needs explanation.

## Spec Sync

After implementing a behavior change, check if a spec exists in `specs/` for the affected feature. If so, update it to match. If none exists and the feature is non-trivial, create one from `specs/SPEC-TEMPLATE.md`. See `.github/instructions/spec-sync.instructions.md` for details.

## Testing

Otto has two test systems. See [docs/developer/testing.md](./docs/developer/testing.md) for the full guide.

### Headless Integration Tests

Tests compositor behavior (gestures, expose, workspaces, window lifecycle, scene graph) using a real compositor instance without GPU.

```sh
cargo test --features headless --test headless_basic
```

- **Backend**: `src/headless.rs` — `HeadlessHandle` starts compositor on a background thread
- **Test client**: `components/otto-kit/src/testing.rs` — lightweight Wayland client with SHM buffers
- **Tests**: `tests/headless_basic.rs`
- **Key APIs**: `handle.swipe_begin/update/end()`, `handle.settle(frames)`, `handle.query(|state| ...)`, `handle.scene_snapshot()`
- Tests must use `#[serial]` attribute

### WLCS Protocol Conformance

Tests Wayland protocol compliance (surface roles, pointer/touch routing, xdg_shell).

```sh
# One-time: build the WLCS test runner
./compile_wlcs.sh

# Build the Otto WLCS adapter
cargo build -p wlcs_otto

# Run specific test groups
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='SelfTest*:FrameSubmission*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='*/SurfacePointerMotionTest*'
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_filter='XdgToplevelStableTest.*'

# List all tests
./wlcs/wlcs target/debug/libwlcs_otto.so --gtest_list_tests
```

- **Adapter**: `wlcs_otto/` — cdylib that WLCS loads, runs a headless Otto instance
- **Key files**: `wlcs_otto/src/main_loop.rs` (event handling), `wlcs_otto/src/ffi_wrappers.rs` (C FFI)

## Key Dependencies

- **smithay** — Wayland compositor library (pinned to specific git rev)
- **lay-rs** — Scene graph and layout engine (`github.com/nongio/layers`)
- **zbus** — D-Bus implementation for screenshare
- **pipewire** — Video streaming for screenshare
- **tokio** — Async runtime for D-Bus service

## Documentation

Two tiers:
- `docs/user/` — End-user configuration and usage guides
- `docs/developer/` — Architecture, design docs, and implementation details

Key developer docs: `rendering.md`, `render_loop.md`, `wayland.md`, `screenshare.md`, `expose.md`, `dock-design.md`, `sc-layer-protocol-design.md`.

Review and documentation guidelines: `.github/instructions/review.instructions.md`, `.github/instructions/documentation.instructions.md`.
