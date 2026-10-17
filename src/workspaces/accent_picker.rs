use layers::{
    engine::Engine,
    prelude::*,
    skia,
    types::{Point, Size},
};
use std::sync::Arc;

use crate::{
    config::Config,
    theme::{accent_color, chrome_blend_mode, chrome_material, theme_colors},
    utils::FONT_CACHE,
};

/// One choice's cell, in points.
const CELL_W: f32 = 34.0;
const CELL_H: f32 = 50.0;
/// Space around the row of cells, and between the row and the caret.
const PADDING: f32 = 5.0;
const CARET_GAP: f32 = 6.0;

/// The row of a letter's accented forms shown at the caret while the letter
/// is held. Each form has its number under it; the keys that pick one are
/// handled in [`crate::input::accents`].
///
/// A single rounded layer, drawn by hand, inside a full-screen,
/// non-interactive `wrap_layer` on the overlay tree. Modeled on
/// [`crate::workspaces::osd::OsdView`].
pub struct AccentPickerView {
    pub wrap_layer: Layer,
    row_layer: Layer,
}

impl AccentPickerView {
    pub fn new(layers_engine: Arc<Engine>) -> Self {
        let wrap = layers_engine.new_layer();
        wrap.set_key("accent_picker_container");
        wrap.set_size(Size::percent(1.0, 1.0), None);
        wrap.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        wrap.set_pointer_events(false);
        wrap.set_hidden(true);

        let row = layers_engine.new_layer();
        row.set_key("accent_picker");
        row.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        row.set_pointer_events(false);
        let _ = wrap.add_sublayer(&row);

        Self {
            wrap_layer: wrap,
            row_layer: row,
        }
    }

    /// Show `choices` next to the caret, which is given in physical pixels
    /// relative to the output, as is `output_px`, the output's size.
    pub fn show(
        &self,
        choices: &[char],
        selected: Option<usize>,
        caret_px: (f32, f32, f32, f32),
        output_px: (f32, f32),
        scale: f32,
    ) {
        let width_px = (choices.len() as f32 * CELL_W + 2.0 * PADDING) * scale;
        let height_px = (CELL_H + 2.0 * PADDING) * scale;
        let (caret_x, caret_y, caret_w, caret_h) = caret_px;
        let gap_px = CARET_GAP * scale;

        // Above the line being typed, centred on the caret; below it when
        // the line is at the top of the screen.
        let x_px = (caret_x + caret_w / 2.0 - width_px / 2.0)
            .min(output_px.0 - width_px)
            .max(0.0);
        let above = caret_y - gap_px - height_px;
        let y_px = if above >= 0.0 {
            above
        } else {
            (caret_y + caret_h + gap_px).min(output_px.1 - height_px)
        };

        self.row_layer
            .set_background_color(chrome_material(theme_colors().material_thin), None);
        self.row_layer.set_blend_mode(chrome_blend_mode());
        self.row_layer.set_border_corner_radius(
            BorderRadius::new_single(otto_kit::corners::radius(10.0 * scale)),
            None,
        );
        self.row_layer
            .set_position(Point { x: x_px, y: y_px }, None);
        self.row_layer
            .set_size(Size::points(width_px, height_px), None);
        self.select(choices, selected, scale);

        self.wrap_layer.set_hidden(false);
        self.row_layer.set_opacity(0.0_f32, None);
        self.row_layer
            .set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.12)));
    }

    /// Redraw the row with `selected` highlighted.
    pub fn select(&self, choices: &[char], selected: Option<usize>, scale: f32) {
        self.row_layer
            .set_draw_content(draw_choices(choices.to_vec(), selected, scale));
    }

    pub fn hide(&self) {
        let w = self.wrap_layer.clone();
        self.row_layer
            .set_opacity(0.0_f32, Some(Transition::ease_out_quad(0.1)))
            .on_finish(
                move |_l: &Layer, _| {
                    w.set_hidden(true);
                },
                true,
            );
    }

    pub fn is_visible(&self) -> bool {
        !self.wrap_layer.hidden()
    }
}

fn draw_choices(choices: Vec<char>, selected: Option<usize>, scale: f32) -> ContentDrawFunction {
    let draw = move |canvas: &skia::Canvas, w: f32, h: f32| -> skia::Rect {
        let colors = theme_colors();
        let font_family = Config::with(|c| c.font_family.clone());
        let regular = skia::FontStyle::new(
            skia::font_style::Weight::NORMAL,
            skia::font_style::Width::NORMAL,
            skia::font_style::Slant::Upright,
        );
        let (glyph_font, number_font) = FONT_CACHE.with(|fc| {
            (
                fc.make_font_with_fallback(font_family.clone(), regular, 22.0 * scale),
                fc.make_font_with_fallback(font_family, regular, 11.0 * scale),
            )
        });

        let cell_w = CELL_W * scale;
        let cell_h = CELL_H * scale;
        let padding = PADDING * scale;
        for (index, choice) in choices.iter().enumerate() {
            let x = padding + index as f32 * cell_w;
            let is_selected = selected == Some(index);
            if is_selected {
                let mut paint = skia::Paint::new(accent_color().c4f(), None);
                paint.set_anti_alias(true);
                let cell = skia::Rect::from_xywh(x, padding, cell_w, cell_h);
                canvas.draw_round_rect(cell, 6.0 * scale, 6.0 * scale, &paint);
            }
            let (glyph_color, number_color) = if is_selected {
                (
                    skia::Color4f::new(1.0, 1.0, 1.0, 1.0),
                    skia::Color4f::new(1.0, 1.0, 1.0, 0.8),
                )
            } else {
                (colors.text_primary.c4f(), colors.text_secondary.c4f())
            };

            let mut paint = skia::Paint::new(glyph_color, None);
            paint.set_anti_alias(true);
            let glyph = choice.to_string();
            let (advance, _) = glyph_font.measure_str(&glyph, Some(&paint));
            let baseline = padding + cell_h * 0.55;
            canvas.draw_str(
                &glyph,
                (x + (cell_w - advance) / 2.0, baseline),
                &glyph_font,
                &paint,
            );

            let mut paint = skia::Paint::new(number_color, None);
            paint.set_anti_alias(true);
            let number = (index + 1).to_string();
            let (advance, _) = number_font.measure_str(&number, Some(&paint));
            let baseline = padding + cell_h * 0.88;
            canvas.draw_str(
                &number,
                (x + (cell_w - advance) / 2.0, baseline),
                &number_font,
                &paint,
            );
        }
        skia::Rect::from_xywh(0.0, 0.0, w, h)
    };
    draw.into()
}
