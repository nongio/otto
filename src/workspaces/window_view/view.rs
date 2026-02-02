use layers::{
    engine::{Engine, TransactionRef},
    prelude::{taffy, Layer, Transition},
    skia,
    types::Point,
    view::RenderLayerTree,
};
use smithay::{reexports::wayland_server::backend::ObjectId, utils::Logical};
use std::sync::{atomic::AtomicBool, Arc};

use crate::shell::WindowElement;

use super::{
    effects::GenieEffect,
    model::WindowViewBaseModel,
    render::view_window_shadow,
};

#[derive(Clone)]
pub struct WindowView {
    pub window_id: ObjectId,
    // views
    pub view_base: layers::prelude::View<WindowViewBaseModel>,

    // layers
    pub window_layer: layers::prelude::Layer,
    pub shadow_layer: layers::prelude::Layer,
    pub content_layer: layers::prelude::Layer,
    pub mirror_layer: layers::prelude::Layer,

    pub genie_effect: GenieEffect,

    pub unmaximised_rect: smithay::utils::Rectangle<i32, Logical>,
    pub minimizing_animation: Arc<AtomicBool>,
}

impl WindowView {
    pub fn new(layers_engine: Arc<Engine>, window: &WindowElement) -> Self {
        let window_id = window.id();
        let layer = window.base_layer().clone();
        layer.set_key("window");
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        let content_layer = layers_engine.new_layer();
        content_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        let shadow_layer = layers_engine.new_layer();
        shadow_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });

        layers_engine.append_layer(&shadow_layer, layer.id());
        layers_engine.append_layer(&content_layer, layer.id());

        let base_rect = WindowViewBaseModel {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            title: "".to_string(),
            fullscreen: false,
            active: false,
        };
        let view_base =
            layers::prelude::View::new("window_shadow", base_rect, Box::new(view_window_shadow));

        view_base.mount_layer(shadow_layer.clone());
        let mirror_layer = window.mirror_layer().clone();
        mirror_layer.set_size(shadow_layer.render_layer().bounds.size(), None);

        layer.set_image_cached(true);

        let genie_effect = GenieEffect::new();

        Self {
            window_id,
            view_base,
            window_layer: layer,
            content_layer,
            shadow_layer,
            genie_effect,
            mirror_layer,
            unmaximised_rect: smithay::utils::Rectangle::default(),
            minimizing_animation: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn set_is_minimizing(&self, minimizing: bool) {
        self.minimizing_animation
            .store(minimizing, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn is_minimizing(&self) -> bool {
        self.minimizing_animation
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn minimize(&self, to_rect: skia::Rect) -> TransactionRef {
        self.window_layer.set_effect(self.genie_effect.clone());
        self.genie_effect.set_destination(to_rect, true);

        let render_layer = self.window_layer.render_bounds_with_children();

        let w = render_layer.width();
        let h = render_layer.height();

        self.window_layer
            .set_draw_content(move |_: &skia::Canvas, _w, _h| {
                skia::Rect::join2(skia::Rect::from_wh(w, h), to_rect).with_outset((100.0, 100.0))
            });

        let tr = self
            .window_layer
            .set_image_filter_progress(1.0, Transition::linear(0.7));

        self.set_is_minimizing(true);
        let view_ref = self.clone();
        tr.on_finish(
            move |l: &Layer, _| {
                view_ref.set_is_minimizing(false);
                // After the animation, drop the shader and keep a simple scaled layer
                l.remove_effect();
                view_ref.apply_minimized_scale_to_layer(l, (w, h), to_rect);
                l.set_position(Point { x: 0.0, y: 0.0 }, None);
            },
            true,
        );
        tr
    }

    pub fn unminimize(&self, from: skia::Rect) -> TransactionRef {
        self.set_is_minimizing(true);
        // Re-enable the shader and reset the scale before running the animation
        // we need set opacity to 0 first to avoid flickering
        self.window_layer.set_hidden(false);
        self.window_layer.set_opacity(0.0, None);
        self.window_layer.set_scale(Point { x: 1.0, y: 1.0 }, None);
        self.window_layer.set_effect(self.genie_effect.clone());

        self.window_layer.engine.update(0.0);
        self.genie_effect.set_destination(from, false);
        let view_ref = self.clone();
        self.window_layer.set_image_filter_progress(1.0, None);
        *self
            .window_layer
            .set_image_filter_progress(0.0, Transition::linear(0.8))
            .on_start(
                |l: &Layer, _| {
                    l.set_opacity(1.0, None);
                },
                true,
            )
            .on_finish(
                move |l: &Layer, _| {
                    l.remove_effect();
                    view_ref.set_is_minimizing(false);
                },
                true,
            )
    }

    /// Apply a scale/position to make the window fit inside the minimized drawer rect.
    /// This is used both after the minimize animation and when the dock resizes.
    pub fn apply_minimized_scale(&self, target: skia::Rect) {
        let bounds = self.window_layer.render_layer().bounds_with_children;
        let base_size = (bounds.width(), bounds.height());
        self.apply_minimized_scale_to_layer(&self.window_layer, base_size, target);
    }

    fn apply_minimized_scale_to_layer(
        &self,
        layer: &Layer,
        base_size: (f32, f32),
        target: skia::Rect,
    ) {
        if self.is_minimizing() {
            return;
        }
        let (w, h) = base_size;
        let scale_x = if w > 0.0 { target.width() / w } else { 1.0 };
        let scale_y = if h > 0.0 { target.height() / h } else { 1.0 };
        let target_scale = scale_x.min(scale_y);
        layer.set_scale(
            Point {
                x: target_scale,
                y: target_scale,
            },
            None,
        );
    }
}

impl std::fmt::Debug for WindowView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowView")
            .field("view_base", &self.view_base)
            .field("window_layer", &self.window_layer)
            .field("content_layer", &self.content_layer)
            .finish()
    }
}
