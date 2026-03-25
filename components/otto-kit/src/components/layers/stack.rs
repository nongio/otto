use crate::app_runner::AppContext;
use layers::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackDirection {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackAlignment {
    Start,
    Center,
    End,
    Stretch,
}

pub struct LayerStack {
    layer: Layer,
}

impl LayerStack {
    /// Create a new stack (automatically added to scene)
    pub fn new(direction: StackDirection) -> Self {
        let engine = AppContext::layers_engine()
            .expect("Layers engine not initialized. Make sure to call this after app starts.");

        let layer = engine.new_layer();

        let flex_direction = match direction {
            StackDirection::Vertical => taffy::FlexDirection::Column,
            StackDirection::Horizontal => taffy::FlexDirection::Row,
        };

        layer.set_layout_style(taffy::Style {
            display: taffy::Display::Flex,
            flex_direction,
            ..Default::default()
        });

        engine.add_layer(&layer.id());

        LayerStack { layer }
    }

    pub fn layer(&self) -> &Layer {
        &self.layer
    }

    pub fn id(&self) -> layers::engine::NodeRef {
        self.layer.id()
    }

    // Direct setters - read current style and update

    pub fn set_gap(&self, gap: f32) {
        let mut style = self.layer.node_layout_style();

        style.gap = taffy::Size {
            width: taffy::LengthPercentage::Length(gap),
            height: taffy::LengthPercentage::Length(gap),
        };

        self.layer.set_layout_style(style);
    }

    pub fn set_alignment(&self, alignment: StackAlignment) {
        let mut style = self.layer.node_layout_style();

        style.align_items = Some(match alignment {
            StackAlignment::Start => taffy::AlignItems::Start,
            StackAlignment::Center => taffy::AlignItems::Center,
            StackAlignment::End => taffy::AlignItems::End,
            StackAlignment::Stretch => taffy::AlignItems::Stretch,
        });

        self.layer.set_layout_style(style);
    }

    pub fn add_child(&self, child: &super::frame::LayerFrame) {
        self.layer.add_sublayer(&child.id());
    }

    pub fn add_stack(&self, child: &LayerStack) {
        self.layer.add_sublayer(&child.id());
    }

    pub fn set_draw(&self, draw_fn: impl Into<ContentDrawFunction>) {
        self.layer.set_draw_content(draw_fn.into());
    }
}
