# AGENTS.md

Guidance for coding agents working in this repository. `CLAUDE.md` is a link to this file.

Every crate under `components/`, `crates/` and `tools/` has its own `AGENTS.md` with what is specific to it. Read it before working in that folder.

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
cargo build -p otto-render   # the compositor's render stack (crates/otto-render)
cargo build -p otto-bar
```

To test a component together with the compositor:
```sh
# First, run Otto in one terminal
cargo run -- --winit &
# Then, in another terminal, run the component
WAYLAND_DISPLAY=wayland-1 cargo run -p otto-bar
```

**Tests:** unit tests live in the lib target — run with `cargo test --lib` (e.g. `cargo test --lib backdrop` for the backdrop rebuild-gating regression tests). End-to-end tests live in `tests/` and drive a real compositor with real Wayland clients behind the `headless` feature — run with `cargo test --features headless --test <name>` (e.g. `headless_basic`). A crate's own tests: `cargo test -p <crate>`; the architecture rules: `cargo run -p otto-arch`. To test only the crates your branch affects, as CI does: `cargo run -q -p otto-arch -- affected --base origin/main --test-args | xargs -r cargo test` (`--explain` instead of `--test-args` says why each crate is in). Minimum supported Rust version is 1.87.0 (1.96.0 to build the whole workspace, which `otto-rdp` pins through GStreamer). CI pins 1.97.0.

## Architecture Overview

Otto is a Wayland compositor built on Smithay with a Skia-based rendering pipeline and the `lay-rs` engine (from `github.com/nongio/layers`) for scene graph/layout management.

### Backend System

Interchangeable backends implement the same compositor logic:
- `src/udev/` — Production backend using DRM/GBM/libinput for bare-metal display
- `src/winit.rs` — Development backend running as a window inside another compositor
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
3. **Skia Renderer**: the `otto-render` crate, re-exported by `src/lib.rs` under its old paths (`crate::renderer`, `crate::skia_renderer`, …). See `crates/otto-render/AGENTS.md`.
4. **Damage Tracking**: `OutputDamageTracker` from Smithay renders only damaged regions
5. **Frame Submission**: Backend submits the composed buffer (dmabuf on DRM, presented on winit/x11)

### Window Management

- `src/shell/` — Protocol implementations for XDG shell, layer shell, XWayland
- `src/workspaces/` — Workspace logic, window views, dock, app switcher, expose mode
- `src/workspaces/window_view/` — Individual window rendering and effects (genie minimize)

### Workspace crates

The root crate is the compositor. `crates/` holds libraries split out of it (`otto-render`), `components/` holds the apps and the libraries they share (`otto-kit` is the toolkit), and `tools/` holds repository tooling. Each has its own `AGENTS.md`; `docs/developer/project-structure.md` maps the whole tree.

### Screenshare System

Located in `src/screenshare/`. See [docs/developer/screenshare.md](./docs/developer/screenshare.md) for detailed architecture.

### Architecture Rules

Crate layers (foundation → service → toolkit → kit → app, plus compositor-core and the compositor), module boundaries and single-home code are rules in `architecture.toml`, checked by `cargo run -p otto-arch` in CI. Third-party policy (who may link Smithay, GStreamer, PipeWire) is in `deny.toml`. Fix code rather than add `[[allow]]` entries. A new crate goes in its layer and gets an `AGENTS.md`. Details: `tools/otto-arch/AGENTS.md` and [docs/developer/architecture.md](./docs/developer/architecture.md).

## Coordinate Systems & Naming Conventions

Otto has two coordinate spaces — mixing them causes subtle scale-dependent bugs.

- **Physical pixels** — raw hardware pixels. Used for layer positions (`set_position`, `change_position`) and `output.current_mode().size`.
- **Logical pixels (points)** — physical ÷ scale. `output_geometry(output).size` returns logical pixels — **do not use this for layer positions**.

Always use the **per-output scale**: `output.current_scale().fractional_scale() as f32`.
`WorkspacesModel.scale` is a global fallback only — avoid it in geometry code.

**Typed geometry is preferred:** Smithay's `Point`/`Size`/`Rectangle<_, Physical>` vs `<_, Logical>`, with the lay-rs edge helpers in `src/utils/geometry.rs` (see `docs/developer/README.md`).

**Naming convention:** where a value is still a bare scalar, suffix physical-pixel variables with `_px` (e.g. `width_px`, `offset_px`) to make the space explicit.

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

## Versioning

All workspace crates share one version, inherited from `[workspace.package]` in
the root `Cargo.toml` — components are never versioned individually. Bump with
`scripts/set-version.sh <version> [--tag]`; see
[docs/developer/versioning.md](./docs/developer/versioning.md). New workspace
members must use `version.workspace = true` (plus `authors`, `edition`,
`license`, `repository`).

## Spec Sync

After implementing a behavior change, check if a spec exists in `specs/` for the affected feature. If so, update it to match. If none exists and the feature is non-trivial, create one from `specs/SPEC-TEMPLATE.md`. See `.github/instructions/spec-sync.instructions.md` for details.

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

Key developer docs: `architecture.md`, `rendering.md`, `render_loop.md`, `wayland.md`, `screenshare.md`, `expose.md`, `dock-design.md`, `surface-style-protocol.md`.

Review guidelines: `.github/instructions/review.instructions.md` — before merging anything, run the `otto-reviewer` agent (`.claude/agents/otto-reviewer.md`).
