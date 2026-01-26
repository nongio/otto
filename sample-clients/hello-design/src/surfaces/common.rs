use std::fmt;
use wayland_client::{protocol::wl_surface, Dispatch, QueueHandle};

// Re-export sc-layer protocol from menu component for convenience
pub use crate::components::menu::{sc_layer_shell_v1, sc_layer_v1};

/// Error type for surface operations
#[derive(Debug)]
pub enum SurfaceError {
    /// Failed to create the surface
    CreationFailed,
    /// Surface not yet configured by compositor
    NotConfigured,
    /// Skia rendering error
    SkiaError(String),
    /// Wayland protocol error
    WaylandError(String),
    /// sc_layer protocol not available
    ScLayerNotAvailable,
    /// Failed to resize surface
    ResizeFailed,
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SurfaceError::CreationFailed => write!(f, "Failed to create surface"),
            SurfaceError::NotConfigured => write!(f, "Surface not yet configured"),
            SurfaceError::SkiaError(e) => write!(f, "Skia error: {}", e),
            SurfaceError::WaylandError(e) => write!(f, "Wayland error: {}", e),
            SurfaceError::ScLayerNotAvailable => write!(f, "sc_layer protocol not available"),
            SurfaceError::ResizeFailed => write!(f, "Failed to resize surface"),
        }
    }
}

impl std::error::Error for SurfaceError {}

/// Common trait for all surface types
pub trait Surface {
    /// Draw on the surface using a callback
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas);

    /// Get the underlying wl_surface
    fn wl_surface(&self) -> &wl_surface::WlSurface;

    /// Get dimensions (width, height) in logical pixels
    fn dimensions(&self) -> (i32, i32);
}

/// Trait for surfaces that support sc_layer augmentation
pub trait ScLayerAugment: Surface {
    /// Check if sc_layer is available for this surface
    fn has_sc_layer(&self) -> bool;

    /// Get mutable access to the sc_layer storage
    fn sc_layer_mut(&mut self) -> Option<&mut Option<sc_layer_v1::ScLayerV1>>;

    /// Get reference to the sc_layer_shell
    fn sc_layer_shell(&self) -> Option<&sc_layer_shell_v1::ScLayerShellV1>;

    /// Check if surface is configured
    fn is_configured(&self) -> bool;

    /// Apply sc_layer augmentation with queue handle
    ///
    /// This version can be called from the configure handler where
    /// we have access to the queue handle. It automatically applies
    /// default menu styling and then calls the optional augment_fn
    /// for additional customization.
    fn augment<F, D>(
        &mut self,
        augment_fn: Option<F>,
        qh: &QueueHandle<D>,
    ) -> Result<(), SurfaceError>
    where
        F: FnOnce(&sc_layer_v1::ScLayerV1),
        D: Dispatch<sc_layer_v1::ScLayerV1, ()> + 'static,
    {
        if !self.is_configured() {
            return Err(SurfaceError::NotConfigured);
        }

        // Clone the sc_layer_shell to avoid borrow conflicts
        let sc_layer_shell = self
            .sc_layer_shell()
            .ok_or(SurfaceError::ScLayerNotAvailable)?
            .clone();

        // Get wl_surface before mutable borrow
        let wl_surface = self.wl_surface().clone();

        let sc_layer = self
            .sc_layer_mut()
            .ok_or(SurfaceError::ScLayerNotAvailable)?;

        augment_surface_with_sc_layer(&wl_surface, &sc_layer_shell, sc_layer, augment_fn, qh);

        Ok(())
    }
}

/// Create or get an sc_layer for a surface and apply styling
///
/// This is a generic helper that can be used by any surface type to apply
/// sc_layer augmentation. It will create the sc_layer if it doesn't exist,
/// apply default menu styling, and optionally call a custom augment function.
///
/// # Arguments
/// * `wl_surface` - The Wayland surface to augment
/// * `sc_layer_shell` - The sc_layer_shell protocol object
/// * `sc_layer` - Mutable reference to store the created sc_layer
/// * `augment_fn` - Optional custom augmentation function
/// * `qh` - Queue handle for creating objects
pub fn augment_surface_with_sc_layer<F, D>(
    wl_surface: &wl_surface::WlSurface,
    sc_layer_shell: &sc_layer_shell_v1::ScLayerShellV1,
    sc_layer: &mut Option<sc_layer_v1::ScLayerV1>,
    augment_fn: Option<F>,
    qh: &QueueHandle<D>,
) where
    F: FnOnce(&sc_layer_v1::ScLayerV1),
    D: Dispatch<sc_layer_v1::ScLayerV1, ()> + 'static,
{
    // Create sc_layer if not exists
    if sc_layer.is_none() {
        let layer = sc_layer_shell.get_layer(wl_surface, qh, ());
        *sc_layer = Some(layer);
    }

    if let Some(ref layer) = sc_layer {
        // Allow caller to override or add more properties
        if let Some(f) = augment_fn {
            f(layer);
        }
    }
}
