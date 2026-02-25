use layers::{engine::Engine, prelude::*, skia, types::Size};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::{theme::theme_colors, utils::resource_image};

const PROGRESSBAR_STEPS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum OsdType {
    Brightness,
    Volume,
    // Future: Keyboard backlight, etc.
}

#[derive(Clone, Debug)]
pub struct OsdViewState {
    pub visible: bool,
    pub osd_type: OsdType,
    pub level: u8,        // 0-PROGRESSBAR_STEPS
    pub max_level: usize, // Number of squares/bars
    brightness_icon: Option<skia::Image>,
    audio_icon: Option<skia::Image>,
    audio_mute_icon: Option<skia::Image>,
}

impl Hash for OsdViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.visible.hash(state);
        // Hash discriminant of enum
        std::mem::discriminant(&self.osd_type).hash(state);
        self.level.hash(state);
        self.max_level.hash(state);
        // Note: We don't hash the images as they're loaded once and don't change
    }
}

pub struct OsdView {
    pub view: View<OsdViewState>,
    pub wrap_layer: Layer,
    pub view_layer: Layer,
    brightness_icon: Option<skia::Image>,
    audio_icon: Option<skia::Image>,
    audio_mute_icon: Option<skia::Image>,
}

impl OsdView {
    pub fn new(layers_engine: Arc<Engine>) -> Self {
        // Create wrap layer with flex centering
        let wrap = layers_engine.new_layer();
        wrap.set_key("osd_container");
        wrap.set_size(Size::percent(1.0, 1.0), None);
        wrap.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            display: taffy::style::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            justify_items: Some(taffy::JustifyItems::Center),
            ..Default::default()
        });
        wrap.set_pointer_events(false);

        // Create inner view layer
        let layer = layers_engine.new_layer();
        wrap.add_sublayer(&layer);
        layer.set_opacity(0.0, None);
        layer.set_pointer_events(false);
        wrap.set_hidden(true);

        // Load icons at startup
        let brightness_icon = resource_image("brightness.svg", "display-brightness-symbolic");
        let audio_icon = resource_image("audio.svg", "audio-volume-high-symbolic");
        let audio_mute_icon = resource_image("audio-mute.svg", "audio-volume-muted-symbolic");

        let state = OsdViewState {
            visible: false,
            osd_type: OsdType::Brightness,
            level: PROGRESSBAR_STEPS as u8,
            max_level: PROGRESSBAR_STEPS,
            brightness_icon: brightness_icon.clone(),
            audio_icon: audio_icon.clone(),
            audio_mute_icon: audio_mute_icon.clone(),
        };

        let view = View::new("osd_view".to_string(), state, Box::new(view_osd));

        view.mount_layer(layer.clone());

        Self {
            view,
            wrap_layer: wrap,
            view_layer: layer,
            brightness_icon,
            audio_icon,
            audio_mute_icon,
        }
    }

    /// Show brightness indicator
    pub fn show_brightness(&self, level: u8) {
        self.view.update_state(&OsdViewState {
            visible: true,
            osd_type: OsdType::Brightness,
            level: level.min(PROGRESSBAR_STEPS as u8),
            max_level: PROGRESSBAR_STEPS,
            brightness_icon: self.brightness_icon.clone(),
            audio_icon: self.audio_icon.clone(),
            audio_mute_icon: self.audio_mute_icon.clone(),
        });
        self.pulse();
    }

    /// Show volume indicator
    pub fn show_volume(&self, level: u8) {
        self.view.update_state(&OsdViewState {
            visible: true,
            osd_type: OsdType::Volume,
            level: level.min(PROGRESSBAR_STEPS as u8),
            max_level: PROGRESSBAR_STEPS,
            brightness_icon: self.brightness_icon.clone(),
            audio_icon: self.audio_icon.clone(),
            audio_mute_icon: self.audio_mute_icon.clone(),
        });
        self.pulse();
    }
    pub fn pulse(&self) {
        self.wrap_layer.set_hidden(false);
        let w = self.wrap_layer.clone();
        self.view_layer
            .set_opacity(
                1.0,
                Some(Transition {
                    delay: 0.0,
                    timing: TimingFunction::ease_out_quad(0.2),
                }),
            )
            .on_finish(
                move |l: &Layer, _| {
                    let w = w.clone();

                    l.set_opacity(
                        0.0,
                        Some(Transition {
                            delay: 0.5,
                            timing: TimingFunction::ease_out_quad(0.3),
                        }),
                    )
                    .on_finish(
                        move |_l: &Layer, _| {
                            w.set_hidden(true);
                        },
                        true,
                    );
                },
                true,
            );
    }
    /// Hide the OSD
    pub fn hide(&self) {
        let w = self.wrap_layer.clone();

        self.view_layer
            .set_opacity(
                0.0,
                Some(Transition {
                    delay: 1.0,
                    timing: TimingFunction::ease_out_quad(0.4),
                }),
            )
            .on_finish(
                move |_l: &Layer, _| {
                    w.set_hidden(true);
                },
                true,
            );
    }
}

pub fn view_osd(state: &OsdViewState, _view: &View<OsdViewState>) -> LayerTree {
    let level = state.level;
    let max_level = state.max_level;

    // Select icon based on OSD type from state
    let icon_image = match (&state.osd_type, level) {
        (OsdType::Brightness, _) => state.brightness_icon.clone(),
        (OsdType::Volume, 0) => state.audio_mute_icon.clone(),
        (OsdType::Volume, _) => state.audio_icon.clone(),
    };

    // Combined draw function for icon and progress
    let draw_osd_content = move |canvas: &skia::Canvas, w: f32, h: f32| {
        let text_color = theme_colors().text_secondary.c4f();

        // Icon dimensions and position (centered at top)
        let icon_size = h * 0.55;
        let icon_y = h * 0.16;
        let icon_x = (w - icon_size) / 2.0;

        // Draw icon if available
        if let Some(icon) = &icon_image {
            let icon_color = theme_colors().text_secondary.c4f();

            let mut paint = skia::Paint::default();
            paint.set_color_filter(skia::color_filters::blend(
                icon_color.to_color(),
                skia::BlendMode::SrcIn,
            ));
            paint.set_anti_alias(true);

            let resampler = skia::CubicResampler::catmull_rom();

            canvas.draw_image_rect_with_sampling_options(
                icon,
                None,
                skia::Rect::from_xywh(icon_x, icon_y, icon_size, icon_size),
                resampler,
                &paint,
            );
        }

        // Progress bar dimensions and position (centered at bottom)
        // Calculate to occupy 80% of width with gap = 0.5 * rect_width
        let rect_width = (w * 0.75) / (max_level as f32 + 0.5 * (max_level - 1) as f32);
        let rect_gap = rect_width * 0.4;
        let rect_height = rect_width * 0.9;
        let bar_width = max_level as f32 * (rect_width + rect_gap) - rect_gap;
        let bar_x = (w - bar_width) / 2.0;
        let bar_y = icon_y + icon_size + h * 0.1;

        let filled_squares = level as usize;

        // Draw progress squares
        for i in 0..max_level {
            let x = bar_x + i as f32 * (rect_width + rect_gap);
            let y = bar_y;

            let mut paint = layers::skia::Paint::new(text_color, None);
            paint.set_anti_alias(true);

            if i < filled_squares {
                paint.set_style(skia::PaintStyle::Fill);
            } else {
                paint.set_style(skia::PaintStyle::Stroke);
                paint.set_stroke_width(1.0);
            }

            let rect = skia::Rect::from_xywh(x, y, rect_width, rect_height);
            canvas.draw_rect(rect, &paint);
        }

        skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    let scale_factor = crate::config::Config::with(|c| c.screen_scale) as f32;
    LayerTreeBuilder::default()
        .key("osd_container")
        .size((Size::percent(1.0, 1.0), None))
        .layout_style(taffy::style::Style {
            display: taffy::style::Display::Flex,
            justify_content: Some(taffy::JustifyContent::Center),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        })
        .pointer_events(false)
        .children(vec![LayerTreeBuilder::default()
            .key("osd_inner")
            .size((
                Size {
                    width: taffy::Dimension::Length(180.0 * scale_factor),
                    height: taffy::Dimension::Length(180.0 * scale_factor),
                },
                None,
            ))
            .background_color(theme_colors().materials_thin)
            .blend_mode(BlendMode::BackgroundBlur)
            .border_corner_radius(BorderRadius::new_single(24.0 * scale_factor))
            .content(Some(draw_osd_content))
            .pointer_events(false)
            .build()
            .unwrap()])
        .build()
        .unwrap()
}
