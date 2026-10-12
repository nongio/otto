# otto-render

The compositor's render stack: `SkiaRenderer` over Smithay's GL renderer, Skia on Vulkan for the udev backend, the EGL, texture and sync plumbing under them, and the render elements that draw the scene graph whole or per KMS plane. Only the compositor links it. It turns elements into frames and knows nothing of windows, workspaces, input or configuration.

How it works: [otto-render](../../docs/developer/otto-render.md).

- **Layer:** compositor-core. Used by `otto`.

## Dependencies
- **Internal:** `otto-foundations` (the runtime directory for debug command files).
- **External:** Smithay (the workspace's fork, shared with the compositor) for the GL, EGL, DRM, GBM and Vulkan backends; `laye-rs` for the scene and, through it, Skia. With `udev`: libdrm and GBM. With `vulkan`: a Vulkan driver.
- **Features** mirror the compositor's and `otto` forwards each one (`egl`, `udev`, `vulkan`, `ticker`, `debug-hooks`, `perf-counters`, `profile-with-puffin`). The defaults are `egl`, `udev` and `vulkan`; `otto` depends on it with `default-features = false` and turns on what its own features need.

## Develop
- Build: `cargo build -p otto-render`. Lint as CI does: `cargo clippy -p otto-render --all-targets -- -D warnings`.
- Tests: `cargo test -p otto-render`. The Vulkan GPU tests need a real device and are ignored by default: `cargo test -p otto-render -- --ignored vulkan`.
- To see a change on screen, run the compositor (`cargo run -- --winit` for GL, `--tty-udev` for planes and Vulkan).
- With `debug-hooks` (part of the compositor's `dev` feature), `touch /tmp/otto-<name>` flips live toggles and `$XDG_RUNTIME_DIR/otto-action` takes commands; see `src/debug_hooks.rs`.

## Where things are
- `src/skia_renderer.rs` is `SkiaRenderer` and the texture import paths.
- `src/renderer/` holds the GL and Vulkan plumbing: Skia surfaces, textures, EGL fences, frames, the backend-neutral traits and the shared drawing helpers (`draw.rs`).
- `src/render_elements/` is the scene as Smithay render elements, including the dmabuf element for planes.
- The rest is small: the pointer and fps counter (`drawing.rs`), the texture cache, per-phase timings and debug toggles.

## Rules
- It cannot name the compositor. Window management, input and configuration stay in `otto`, which passes in what the renderer needs (see `SceneElement::with_engine`).
- The compositor's `src/lib.rs` re-exports its modules under their old paths, so code in `src/` says `crate::renderer::…` or `crate::skia_renderer::…`. The module rules in `architecture.toml` may name those re-exported modules.
- A new feature is added here and in the root `Cargo.toml`, which forwards it.
- Drawing code exists once, in `src/renderer/draw.rs`: GL and Vulkan frames call the same helpers, so a visual change needs no Vulkan twin.
- The lib root keeps `#![warn(unreachable_pub)]`, which the compositor-core layer requires: the public API is what `lib.rs` exports.

## See also
- [Rendering pipeline](../../docs/developer/rendering.md), [Vulkan renderer](../../docs/developer/vulkan-renderer.md), [Render loop](../../docs/developer/render_loop.md).
- [Scene graph](../../docs/developer/scene-graph.md), [KMS planes](../../docs/developer/drm_plane.md), [Debug action hook](../../docs/developer/debug-action-hook.md).
- [Architecture](../../docs/developer/architecture.md).
