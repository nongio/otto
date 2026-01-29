//! Skia-based GPU renderer for the Otto compositor.
//!
//! This module provides a hardware-accelerated rendering backend that combines
//! Smithay's OpenGL renderer with Skia's 2D graphics library. This allows the
//! compositor to use both efficient buffer management from Smithay and advanced
//! drawing capabilities from Skia.
//!
//! # Architecture
//!
//! - `egl_context`: EGL surface wrappers for use in collections
//! - `sync`: GPU synchronization using EGL fences
//! - `skia_surface`: Skia surface creation and management
//! - `textures`: Texture types combining OpenGL and Skia
//!
//! The main `SkiaRenderer` in the parent module orchestrates these components.

pub mod egl_context;
pub mod skia_surface;
pub mod sync;
pub mod textures;

// Re-export commonly used types
pub use egl_context::EGLSurfaceWrapper;
pub use skia_surface::SkiaSurface;
pub use sync::{finished_proc, FlushInfo2, SkiaSync, FINISHED_PROC_STATE};
pub use textures::{SkiaFrame, SkiaGLesFbo, SkiaTexture, SkiaTextureImage, SkiaTextureMapping};
