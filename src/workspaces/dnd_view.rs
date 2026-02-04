use layers::{engine::Engine, prelude::taffy, types::Point};
use std::sync::Arc;

#[derive(Clone)]
pub struct DndView {
    pub layer: layers::prelude::Layer,
    pub content_layer: layers::prelude::Layer,
    pub initial_position: Point,
}

impl DndView {
    pub fn new(layers_engine: Arc<Engine>) -> Self {
        let layer = layers_engine.new_layer();
        layer.set_key("dnd_view");
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        let content_layer = layers_engine.new_layer();
        content_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        layers_engine.add_layer(&layer);
        layers_engine.append_layer(&content_layer, layer.id());

        Self {
            layer,
            content_layer,
            initial_position: Point::default(),
        }
    }
    pub fn set_initial_position(&mut self, point: Point) {
        self.initial_position = point;
    }
}
