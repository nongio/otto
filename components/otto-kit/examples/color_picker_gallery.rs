//! Offscreen gallery of the colour picker: the closed well in every
//! interaction state, then the open panel in each of its three modes — all
//! straight to a PNG with Skia's raster backend, no compositor, no
//! Wayland. Proves `color_picker::well::draw` and `color_picker::panel::draw`
//! are both callable from a bare canvas, the way `otto-kit-roadmap.md`
//! requires for anything the compositor draws server-side. Follows
//! `dropdown_gallery.rs`.
//!
//! ```sh
//! cargo run -p otto-kit --example color_picker_gallery -- /tmp/color_picker_gallery.png
//! ```
//!
//! Worth eyeballing once rendered: the HSV square in each `Hsv` cell has to
//! fade white-to-hue left-to-right and bright-to-black top-to-bottom for
//! the *selected* hue — the easiest part of this component to get subtly
//! wrong.

use otto_kit::components::color_picker::panel::{self, HexField, Mode};
use otto_kit::components::color_picker::well::{self, WellInteraction};
use otto_kit::prelude::*;
use skia_safe::{surfaces, Color, EncodedImageFormat, Paint, Rect};

const WELL_CELL_W: f32 = 200.0;
const WELL_CELL_H: f32 = 70.0;
const WELL_COLS: usize = 5;

fn presets() -> Vec<panel::Swatch> {
    vec![
        panel::Swatch::new("Blue", Color::from_rgb(0x0A, 0x84, 0xFF)),
        panel::Swatch::new("Purple", Color::from_rgb(0xBF, 0x5A, 0xF2)),
        panel::Swatch::new("Pink", Color::from_rgb(0xFF, 0x2D, 0x55)),
        panel::Swatch::new("Red", Color::from_rgb(0xFF, 0x3B, 0x30)),
        panel::Swatch::new("Orange", Color::from_rgb(0xFF, 0x95, 0x00)),
        panel::Swatch::new("Yellow", Color::from_rgb(0xFF, 0xCC, 0x00)),
        panel::Swatch::new("Green", Color::from_rgb(0x34, 0xC7, 0x59)),
        panel::Swatch::new("Teal", Color::from_rgb(0x30, 0xB0, 0xC7)),
        panel::Swatch::new("Graphite", Color::from_rgb(0x8E, 0x8E, 0x93)),
    ]
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "color_picker_gallery.png".to_string());

    let well_variants: Vec<(&str, bool, WellInteraction)> = vec![
        ("light · normal", false, WellInteraction::Normal),
        ("light · hovered", false, WellInteraction::Hovered),
        ("light · pressed", false, WellInteraction::Pressed),
        ("light · open", false, WellInteraction::Open),
        ("light · disabled", false, WellInteraction::Disabled),
        ("dark · normal", true, WellInteraction::Normal),
        ("dark · hovered", true, WellInteraction::Hovered),
        ("dark · pressed", true, WellInteraction::Pressed),
        ("dark · open", true, WellInteraction::Open),
        ("dark · disabled", true, WellInteraction::Disabled),
    ];

    let swatches = presets();
    let (panel_w, panel_h) = panel::panel_size(swatches.len());
    let panel_cell_w = panel_w + 32.0;
    let panel_cell_h = panel_h + 56.0;
    let panel_cols = 3;
    let panel_variants: Vec<(&str, bool, Mode)> = vec![
        ("light · swatches", false, Mode::Swatches),
        ("light · hsv", false, Mode::Hsv),
        ("light · hex", false, Mode::Hex),
        ("dark · swatches", true, Mode::Swatches),
        ("dark · hsv", true, Mode::Hsv),
        ("dark · hex", true, Mode::Hex),
    ];

    let scale = 2.0_f32;
    let well_rows = well_variants.len().div_ceil(WELL_COLS);
    let well_section_h = well_rows as f32 * WELL_CELL_H;
    let panel_rows = panel_variants.len().div_ceil(panel_cols);
    let panel_section_h = panel_rows as f32 * panel_cell_h;

    let width = (WELL_CELL_W * WELL_COLS as f32).max(panel_cell_w * panel_cols as f32) * scale;
    let height = (well_section_h + panel_section_h) * scale;

    let mut surface =
        surfaces::raster_n32_premul((width as i32, height as i32)).expect("raster surface");
    let canvas = surface.canvas();
    canvas.scale((scale, scale));

    for (i, (caption, dark, interaction)) in well_variants.iter().enumerate() {
        let col = i % WELL_COLS;
        let row = i / WELL_COLS;
        canvas.save();
        canvas.translate((col as f32 * WELL_CELL_W, row as f32 * WELL_CELL_H));
        draw_well_cell(canvas, caption, *dark, *interaction);
        canvas.restore();
    }

    for (i, (caption, dark, mode)) in panel_variants.iter().enumerate() {
        let col = i % panel_cols;
        let row = i / panel_cols;
        canvas.save();
        canvas.translate((
            col as f32 * panel_cell_w,
            well_section_h + row as f32 * panel_cell_h,
        ));
        draw_panel_cell(canvas, caption, *dark, *mode, &swatches, panel_w, panel_h);
        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&out, data.as_bytes()).expect("write png");
    println!("wrote {out} ({width}x{height})");
}

fn draw_well_cell(canvas: &Canvas, caption: &str, dark: bool, interaction: WellInteraction) {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut bg = Paint::default();
    bg.set_color(if dark {
        Color::from_rgb(0x1E, 0x22, 0x2B)
    } else {
        Color::from_rgb(0xE7, 0xE9, 0xEC)
    });
    canvas.draw_rect(Rect::from_wh(WELL_CELL_W - 8.0, WELL_CELL_H - 8.0), &bg);

    canvas.save();
    canvas.translate((16.0, 14.0));
    Label::new(caption)
        .with_style(styles::CAPTION_1)
        .with_color(if dark {
            Color::from_argb(0xCC, 0xFF, 0xFF, 0xFF)
        } else {
            Color::from_argb(0xB0, 0x00, 0x00, 0x00)
        })
        .render(canvas);
    canvas.restore();

    let color = Color::from_rgb(0x0A, 0x84, 0xFF);
    let rect = Rect::from_xywh(16.0, 36.0, well::measure(color), well::HEIGHT);
    well::draw(canvas, rect, color, interaction, &theme);
}

#[allow(clippy::too_many_arguments)]
fn draw_panel_cell(
    canvas: &Canvas,
    caption: &str,
    dark: bool,
    mode: Mode,
    swatches: &[panel::Swatch],
    panel_w: f32,
    panel_h: f32,
) {
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut bg = Paint::default();
    bg.set_color(if dark {
        Color::from_rgb(0x1E, 0x22, 0x2B)
    } else {
        Color::from_rgb(0xE7, 0xE9, 0xEC)
    });
    canvas.draw_rect(
        Rect::from_wh(panel_w + 32.0 - 8.0, panel_h + 56.0 - 8.0),
        &bg,
    );

    canvas.save();
    canvas.translate((16.0, 14.0));
    Label::new(caption)
        .with_style(styles::CAPTION_1)
        .with_color(if dark {
            Color::from_argb(0xCC, 0xFF, 0xFF, 0xFF)
        } else {
            Color::from_argb(0xB0, 0x00, 0x00, 0x00)
        })
        .render(canvas);
    canvas.restore();

    let color = Color::from_rgb(0x2E, 0xB8, 0x5C); // a saturated hue away from
                                                   // the swatch presets, so
                                                   // HSV mode's cursor and
                                                   // hue strip indicator are
                                                   // both visibly off the
                                                   // extremes.
    let rect = Rect::from_xywh(16.0, 36.0, panel_w, panel_h);
    panel::draw(
        canvas,
        rect,
        mode,
        color,
        swatches,
        Some(0),
        Some(HexField::Hex),
        &theme,
    );
}
