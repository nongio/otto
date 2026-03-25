use crate::app_runner::AppContext;
use layers::prelude::*;
use layers::types::{Point, Size};

/// A styled rectangular container using the layers engine
pub struct LayerFrame {
    layer: Layer,
}

impl LayerFrame {
    /// Create a new frame (automatically added to scene)
    pub fn new() -> Self {
        let engine = AppContext::layers_engine()
            .expect("Layers engine not initialized. Make sure to call this after app starts.");

        let layer = engine.new_layer();
        engine.add_layer(&layer.id());

        LayerFrame { layer }
    }

    /// Get the underlying layer
    pub fn layer(&self) -> &Layer {
        &self.layer
    }

    /// Get the layer ID
    pub fn id(&self) -> layers::engine::NodeRef {
        self.layer.id()
    }

    // Direct setters - match the layers API

    pub fn set_size(&self, width: f32, height: f32) {
        self.layer.set_size(
            Size {
                width: taffy::Dimension::Length(width),
                height: taffy::Dimension::Length(height),
            },
            None,
        );
    }

    pub fn set_position(&self, x: f32, y: f32) {
        self.layer.set_position(Point { x, y }, None);
    }

    pub fn set_background(&self, color: Color) {
        self.layer
            .set_background_color(PaintColor::Solid { color }, None);
    }

    pub fn set_corner_radius(&self, radius: f32) {
        self.layer
            .set_border_corner_radius(BorderRadius::new_single(radius), None);
    }

    pub fn add_child(&self, child: &LayerFrame) {
        self.layer.add_sublayer(&child.id());
    }

    pub fn set_draw(&self, draw_fn: impl Into<ContentDrawFunction>) {
        self.layer.set_draw_content(draw_fn.into());
    }
}

impl Default for LayerFrame {
    fn default() -> Self {
        Self::new()
    }
}
