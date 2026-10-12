//! EGL surface and context management utilities.
//!
//! This module provides thin wrappers around Smithay's EGL types to enable
//! them to be used as hash keys and in collections.

use std::{hash::Hash, rc::Rc};

use smithay::backend::egl::EGLSurface;

/// Wrapper around `EGLSurface` that implements `PartialEq`, `Eq`, and `Hash`.
///
/// This allows EGL surfaces to be used as keys in HashMaps and other collections.
/// Equality is based on the underlying surface handle pointer, not surface content.
#[derive(Debug, Clone)]
pub struct EGLSurfaceWrapper(pub Rc<EGLSurface>);

impl PartialEq for EGLSurfaceWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0.get_surface_handle() == other.0.get_surface_handle()
    }
}

impl Eq for EGLSurfaceWrapper {}

impl Hash for EGLSurfaceWrapper {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.get_surface_handle().hash(state);
    }
}
