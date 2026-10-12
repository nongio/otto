//! Otto's render stack: how the compositor turns elements into pixels.
//!
//! - [`skia_renderer`]: `SkiaRenderer`, Smithay's GLES renderer with a Skia
//!   surface on every frame, and the texture import paths.
//! - [`renderer`]: the parts it is built from (EGL surfaces and fences, Skia
//!   surfaces, textures, frames), the backend-neutral renderer traits and,
//!   with `vulkan`, Skia on Vulkan for the udev backend.
//! - [`render_elements`]: the scene graph as Smithay render elements, drawn
//!   whole or per KMS plane, and the element enums the backends render.
//! - [`drawing`]: the pointer element and, with `ticker`, the fps counter.
//! - [`textures_storage`], [`render_phase_stats`] and [`debug_hooks`]: the
//!   texture cache, per-phase frame timings and the file-driven debug toggles
//!   the render path polls.
//!
//! Nothing here knows about windows, workspaces, input or configuration: the
//! compositor (`otto`) decides what is on screen and hands it over, and
//! passes in the settings the renderer needs (see
//! [`render_elements::scene_element::SceneElement::with_engine`]).

// A `pub` item nothing outside the crate can reach is an accident: say
// `pub(crate)` or `pub(super)`, so the public API is what lib.rs exports.
#![warn(unreachable_pub)]

pub mod debug_hooks;
pub mod drawing;
pub mod render_elements;
pub mod render_phase_stats;
pub mod renderer;
pub mod skia_renderer;
pub mod textures_storage;
