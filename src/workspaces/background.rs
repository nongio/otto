use std::hash::{Hash, Hasher};

use layers::{prelude::*, skia};

#[derive(Clone, Debug)]
pub struct BackgroundViewState {
    pub image: Option<skia::Image>,
    pub debug_string: String,
    pub fallback_color: skia::Color4f,
}
impl Hash for BackgroundViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if let Some(image) = self.image.as_ref() {
            image.unique_id().hash(state);
        }
        self.debug_string.hash(state);
        // Hash the color components
        self.fallback_color.r.to_bits().hash(state);
        self.fallback_color.g.to_bits().hash(state);
        self.fallback_color.b.to_bits().hash(state);
        self.fallback_color.a.to_bits().hash(state);
    }
}

pub struct BackgroundView {
    // engine: layers::prelude::LayersEngine,
    pub view: layers::prelude::View<BackgroundViewState>,
    // pub state: RwLock<BackgroundViewState>,
    pub base_layer: Layer,
}

impl BackgroundView {
    pub fn new(index: usize, layer: Layer, fallback_color: skia::Color4f) -> Self {
        let state = BackgroundViewState {
            image: None,
            debug_string: "Screen composer 0.1".to_string(),
            fallback_color,
        };
        let view = layers::prelude::View::new(
            format!("background_view_{}", index),
            state,
            Box::new(view_background),
        );
        view.mount_layer(layer.clone());
        Self {
            view,
            base_layer: layer,
        }
    }

    pub fn set_debug_text(&self, text: String) {
        self.view.update_state(&BackgroundViewState {
            debug_string: text,
            ..self.view.get_state()
        });
    }

    pub fn set_image(&self, image: skia::Image) {
        self.view.update_state(&BackgroundViewState {
            image: Some(image),
            ..self.view.get_state()
        });
    }
}

// static mut COUNTER: f32 = 1.0;
pub fn view_background(
    state: &BackgroundViewState,
    _view: &View<BackgroundViewState>,
) -> LayerTree {
    let image = state.image.clone();
    let fallback_color = state.fallback_color;

    // let debug_text = state.debug_string.clone();

    let draw_container = move |canvas: &skia::Canvas, w, h| {
        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);

        if let Some(image) = image.as_ref() {
            let mut matrix = skia::Matrix::new_identity();
            let image_width = image.width() as f32;
            let image_height = image.height() as f32;
            let scale_x: f32 = w / image_width;
            let scale_y: f32 = h / image_height;
            let scale = scale_x.max(scale_y); // Choose the smaller scale to maintain aspect ratio

            // Calculate the offsets for centering the image
            let offset_x = (w - image_width * scale) / 2.0;
            let offset_y = (h - image_height * scale) / 2.0;

            matrix.set_scale_translate((scale, scale), (offset_x, offset_y)); // canvas.concat(&matrix);
                                                                              // canvas.draw_image_rect(image, None, rect, &paint);
            paint.set_shader(image.to_shader(
                (skia::TileMode::Repeat, skia::TileMode::Repeat),
                skia::SamplingOptions::default(),
                &matrix,
            ));
        } else {
            // Create a gradient from bottom (fallback_color) to top (lighter version)
            // Make the top color lighter by interpolating towards white
            let lighter_factor = 0.3; // 30% lighter
            let top_color = skia::Color4f::new(
                fallback_color.r + (1.0 - fallback_color.r) * lighter_factor,
                fallback_color.g + (1.0 - fallback_color.g) * lighter_factor,
                fallback_color.b + (1.0 - fallback_color.b) * lighter_factor,
                fallback_color.a,
            );

            // Convert Color4f to Color for gradient shader
            let colors = [
                skia::Color::from_argb(
                    (fallback_color.a * 255.0) as u8,
                    (fallback_color.r * 255.0) as u8,
                    (fallback_color.g * 255.0) as u8,
                    (fallback_color.b * 255.0) as u8,
                ),
                skia::Color::from_argb(
                    (top_color.a * 255.0) as u8,
                    (top_color.r * 255.0) as u8,
                    (top_color.g * 255.0) as u8,
                    (top_color.b * 255.0) as u8,
                ),
            ];
            let positions: &[f32] = &[0.0, 1.0];

            let start_point = skia::Point::new(0.0, h); // Bottom
            let end_point = skia::Point::new(0.0, 0.0); // Top

            if let Some(shader) = skia::gradient_shader::linear(
                (start_point, end_point),
                skia::gradient_shader::GradientShaderColors::Colors(&colors),
                Some(positions),
                skia::TileMode::Clamp,
                None,
                None,
            ) {
                paint.set_shader(shader);
            }
        }

        let split = 1;
        let rect_size_w = w / split as f32;
        let rect_size_h = h / split as f32;

        for i in 0..split {
            for j in 0..split {
                let rect = skia::Rect::from_xywh(
                    i as f32 * rect_size_w,
                    j as f32 * rect_size_h,
                    rect_size_w,
                    rect_size_h,
                );
                canvas.draw_rect(rect, &paint);
            }
        }

        // let color = skia::Color4f::new(0.0, 0.0, 0.0, 1.0);
        // let paint = skia::Paint::new(color, None);
        // let mut font = skia::Font::default();
        // let font_size = 26.0;
        // font.set_size(font_size);
        // canvas.draw_str("test string string", (80.0, 100.0), &font, &paint);
        // canvas.draw_rect(skia::Rect::from_xywh(80.0, 100.0, 200.0, 100.0), &paint);
        skia::Rect::from_xywh(0.0, 0.0, w, h)
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
