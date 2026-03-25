pub mod surface;

pub use surface::LayerSurface;

use skia_safe::Canvas;

// Re-export sc-layer protocol from menu component
pub use crate::protocols::otto_surface_style_v1;

/// Layer component - a drawable subsurface that can be positioned on top of a parent surface
///
/// The Layer is a subsurface where you can draw custom content. It can be used for
/// overlays, decorations, or custom UI elements like menubars.
///
/// The layer can be augmented with custom drawing functions at any time.
/// The layer surface can also be augmented on configure with sc_layer properties.
pub struct Layer {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    draw_fn: Option<Box<dyn FnMut(&Canvas)>>,
    augment_fn: Option<Box<dyn Fn(&otto_surface_style_v1::OttoSurfaceStyleV1)>>,
}

impl Layer {
    /// Create a new Layer with the specified position and size
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            draw_fn: None,
            augment_fn: None,
        }
    }

    /// Set or augment the layer with a custom drawing function
    pub fn with_draw_fn<F>(mut self, draw_fn: F) -> Self
    where
        F: FnMut(&Canvas) + 'static,
    {
        self.draw_fn = Some(Box::new(draw_fn));
        self
    }

    /// Set or augment the layer with a custom drawing function (mutable version)
    pub fn set_draw_fn<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&Canvas) + 'static,
    {
        self.draw_fn = Some(Box::new(draw_fn));
    }

    /// Set augmentation function to apply sc_layer properties on configure
    pub fn with_augment_fn<F>(mut self, augment_fn: F) -> Self
    where
        F: Fn(&otto_surface_style_v1::OttoSurfaceStyleV1) + 'static,
    {
        self.augment_fn = Some(Box::new(augment_fn));
        self
    }

    /// Set augmentation function to apply sc_layer properties on configure (mutable version)
    pub fn set_augment_fn<F>(&mut self, augment_fn: F)
    where
        F: Fn(&otto_surface_style_v1::OttoSurfaceStyleV1) + 'static,
    {
        self.augment_fn = Some(Box::new(augment_fn));
    }

    /// Get the augmentation function
    pub(crate) fn augment_fn(
        &self,
    ) -> Option<&Box<dyn Fn(&otto_surface_style_v1::OttoSurfaceStyleV1)>> {
        self.augment_fn.as_ref()
    }

    /// Get the layer's X position
    pub fn x(&self) -> i32 {
        self.x
    }

    /// Get the layer's Y position
    pub fn y(&self) -> i32 {
        self.y
    }

    /// Get the layer's width
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Get the layer's height
    pub fn height(&self) -> i32 {
        self.height
    }

    /// Render the layer content
    pub fn render(&mut self, canvas: &Canvas) {
        if let Some(ref mut draw_fn) = self.draw_fn {
            draw_fn(canvas);
        }
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new(0, 0, 400, 40)
    }
}
