//! The desktop wallpaper behind each workspace.
//!
//! [`store`] decodes wallpapers and renders them into frames the size of an
//! output; [`BackgroundView`] shows one. The geometry and the drawing are
//! `otto_kit::wallpaper`'s, shared with the settings app's preview.

pub(crate) mod store;

use std::hash::{Hash, Hasher};

use layers::{prelude::*, skia};
use otto_kit::wallpaper::cover;

/// What a background shows.
#[derive(Clone, Debug)]
pub enum BackgroundContent {
    /// A wallpaper is on its way: show nothing over the layer's black
    /// backing rather than flash the gradient first.
    Loading,
    /// A rendered frame, normally the output's exact size.
    Frame(skia::Image),
    /// No wallpaper, or one that cannot be shown: the gradient.
    Gradient,
}

#[derive(Clone, Debug)]
pub struct BackgroundViewState {
    pub content: BackgroundContent,
    pub fallback_color: skia::Color4f,
}

impl Hash for BackgroundViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.content {
            BackgroundContent::Loading => 0u8.hash(state),
            BackgroundContent::Frame(image) => {
                1u8.hash(state);
                image.unique_id().hash(state);
            }
            BackgroundContent::Gradient => 2u8.hash(state),
        }
        self.fallback_color.r.to_bits().hash(state);
        self.fallback_color.g.to_bits().hash(state);
        self.fallback_color.b.to_bits().hash(state);
        self.fallback_color.a.to_bits().hash(state);
    }
}

pub struct BackgroundView {
    pub view: layers::prelude::View<BackgroundViewState>,
    pub base_layer: Layer,
}

impl BackgroundView {
    pub fn new(index: usize, layer: Layer, fallback_color: skia::Color4f) -> Self {
        let state = BackgroundViewState {
            content: BackgroundContent::Gradient,
            fallback_color,
        };
        let view = layers::prelude::View::new(
            format!("background_view_{}", index),
            state,
            Box::new(view_background),
        );
        view.mount_layer(layer.clone());
        // The draw callback fills the entire bounds with opaque pixels (image or gradient).
        layer.set_content_opaque(true);
        Self {
            view,
            base_layer: layer,
        }
    }

    /// Show `content`, with `fallback_color` for the gradient. A no-op when
    /// neither changed: the state hash covers both.
    pub fn show(&self, content: BackgroundContent, fallback_color: skia::Color4f) {
        self.view.update_state(&BackgroundViewState {
            content,
            fallback_color,
        });
    }

    /// What the background is showing now.
    pub fn content(&self) -> BackgroundContent {
        self.view.get_state().content
    }
}

pub fn view_background(
    state: &BackgroundViewState,
    _view: &View<BackgroundViewState>,
) -> LayerTree {
    let content = state.content.clone();
    let fallback_color = state.fallback_color;

    let draw_container = move |canvas: &skia::Canvas, w: f32, h: f32| {
        let bounds = skia::Rect::from_wh(w, h);
        match &content {
            BackgroundContent::Frame(frame) => {
                // A frame is rendered at the output's size and drawn 1:1. For
                // the moment between a mode change and its new frame, cover
                // the layer with the old one — at a whole-pixel offset, see
                // `otto_kit::wallpaper`.
                let target = (w.round() as i32, h.round() as i32);
                let placement = cover((frame.width(), frame.height()), target);
                otto_kit::wallpaper::paint(canvas, frame, placement, bounds);
            }
            BackgroundContent::Gradient => paint_fallback_gradient(canvas, fallback_color, bounds),
            BackgroundContent::Loading => {}
        }
        bounds
    };

    LayerTreeBuilder::default()
        .key("background_view")
        .opacity((
            1.0,
            Some(Transition {
                delay: 0.2,
                timing: TimingFunction::ease_out_quad(0.8),
            }),
        ))
        .border_corner_radius(BorderRadius::new_single(24.0))
        .content(Some(draw_container))
        // .image_cache(true)
        .background_color(layers::prelude::Color::new_rgba(0.0, 0.0, 0.0, 1.0))
        .pointer_events(false)
        .build()
        .unwrap()
}

/// The gradient shown when no wallpaper is set: `color` at the bottom, 30%
/// of the way to white at the top.
fn paint_fallback_gradient(canvas: &skia::Canvas, color: skia::Color4f, bounds: skia::Rect) {
    const LIGHTER: f32 = 0.3;
    let top = skia::Color4f::new(
        color.r + (1.0 - color.r) * LIGHTER,
        color.g + (1.0 - color.g) * LIGHTER,
        color.b + (1.0 - color.b) * LIGHTER,
        color.a,
    );
    let colors = [color.to_color(), top.to_color()];
    let mut paint = skia::Paint::default();
    if let Some(shader) = skia::gradient_shader::linear(
        ((0.0, bounds.bottom), (0.0, bounds.top)),
        skia::gradient_shader::GradientShaderColors::Colors(&colors),
        Some(&[0.0, 1.0][..]),
        skia::TileMode::Clamp,
        None,
        None,
    ) {
        paint.set_shader(shader);
    }
    canvas.draw_rect(bounds, &paint);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_of(state: &BackgroundViewState) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        state.hash(&mut hasher);
        hasher.finish()
    }

    /// A new frame, a different content kind or a new colour each re-render
    /// the view; the same frame again does not.
    #[test]
    fn the_state_hash_follows_what_is_shown() {
        let colour = skia::Color4f::new(0.1, 0.2, 0.3, 1.0);
        let frame = |c| {
            let mut surface = skia::surfaces::raster_n32_premul((4, 4)).unwrap();
            surface.canvas().clear(c);
            surface.image_snapshot()
        };
        let red = frame(skia::Color::RED);
        let state = |content| BackgroundViewState {
            content,
            fallback_color: colour,
        };
        let a = hash_of(&state(BackgroundContent::Frame(red.clone())));
        assert_eq!(a, hash_of(&state(BackgroundContent::Frame(red))));
        assert_ne!(
            a,
            hash_of(&state(BackgroundContent::Frame(frame(skia::Color::BLUE))))
        );
        assert_ne!(
            hash_of(&state(BackgroundContent::Loading)),
            hash_of(&state(BackgroundContent::Gradient))
        );
        let mut other = state(BackgroundContent::Gradient);
        other.fallback_color.r = 0.5;
        assert_ne!(
            hash_of(&other),
            hash_of(&state(BackgroundContent::Gradient))
        );
    }
}
