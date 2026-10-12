# otto-render

`otto-render` is the compositor's render stack: `SkiaRenderer` over Smithay's GL renderer, Skia on Vulkan for the udev backend, the EGL, texture and sync plumbing under them, and the render elements that draw the scene graph whole or per KMS plane. It is a library linked only by the compositor. It turns elements into frames and knows nothing of windows, workspaces, input or configuration.

It sits in the **compositor-core** layer (see [Architecture Rules](architecture.md)) and is used by `otto`.

## Dependencies

- **Internal:** `otto-foundations` (the runtime directory for debug command files).
- **External:** Smithay (the workspace's fork, shared with the compositor) for the GL, EGL, DRM, GBM and Vulkan backends; `laye-rs` for the scene and, through it, Skia. With `udev`: libdrm and GBM. With `vulkan`: a Vulkan driver.
- Features mirror the compositor's and `otto` forwards each one (`egl`, `udev`, `vulkan`, `ticker`, `debug-hooks`, `perf-counters`, `profile-with-puffin`). The defaults are `egl`, `udev` and `vulkan`; `otto` depends on it with `default-features = false` and turns on what its own features need.

## Develop

- Build: `cargo build -p otto-render`. Lint as CI does: `cargo clippy -p otto-render --all-targets -- -D warnings`.
- Tests: `cargo test -p otto-render`. The Vulkan GPU tests need a real device and are ignored by default: `cargo test -p otto-render -- --ignored vulkan`.
- To see a change on screen, run the compositor (`cargo run -- --winit` for GL, `--tty-udev` for planes and Vulkan).
- With `debug-hooks` (part of the compositor's `dev` feature), `touch /tmp/otto-<name>` flips live toggles and `$XDG_RUNTIME_DIR/otto-action` takes commands; see [src/debug_hooks.rs](../../crates/otto-render/src/debug_hooks.rs) and [debug-action-hook.md](debug-action-hook.md).

## Code map

- [src/skia_renderer.rs](../../crates/otto-render/src/skia_renderer.rs): `SkiaRenderer` and the texture import paths.
- [src/renderer/](../../crates/otto-render/src/renderer/): Skia surfaces, textures, EGL fences and contexts, frames, the backend-neutral traits (`active.rs`), shared drawing helpers (`draw.rs`) and Skia on Vulkan (`vulkan/`, `vulkan_api.rs`).
- [src/render_elements/](../../crates/otto-render/src/render_elements/): the scene as Smithay render elements, the dmabuf element for planes, and the element enums the backends render.
- [src/drawing.rs](../../crates/otto-render/src/drawing.rs): the pointer element and, with `ticker`, the fps counter (digits from `resources/numbers.png`).
- `src/textures_storage.rs`, `src/render_phase_stats.rs`, `src/debug_hooks.rs`: the texture cache, per-phase frame timings, debug toggles.

## Rules

- It cannot name the compositor. Window management, input and configuration stay in `otto`, which passes in what the renderer needs (see `SceneElement::with_engine`).
- The compositor's `src/lib.rs` re-exports its modules under their old paths, so code in `src/` says `crate::renderer::…` or `crate::skia_renderer::…`. The module rules in `architecture.toml` may name those re-exported modules.
- A new feature is added here and in the root `Cargo.toml`, which forwards it.
- Drawing code exists once, in `src/renderer/draw.rs`: GL and Vulkan frames call the same helpers, so a visual change needs no Vulkan twin.
- The lib root keeps `#![warn(unreachable_pub)]`, which the compositor-core layer requires: the public API is what `lib.rs` exports.

## See also

- [Rendering pipeline](rendering.md)
- [Vulkan renderer](vulkan-renderer.md)
- [Render loop](render_loop.md)
- [Scene graph](scene-graph.md)
- [KMS planes](drm_plane.md)
