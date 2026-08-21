//! Offscreen gallery of the dropdown field — the closed pop-up button only.
//!
//! Renders every interaction state, in both themes, straight to a PNG with
//! Skia's raster backend — no compositor, no Wayland. This is the proof
//! that `dropdown::field::draw` really is callable from a bare canvas, the
//! way `otto-kit-roadmap.md` requires for anything the compositor draws
//! server-side. Follows `form_controls_gallery.rs`.
//!
//! ```sh
//! cargo run -p otto-kit --example dropdown_gallery -- /tmp/dropdown_gallery.png
//! ```

use otto_kit::components::dropdown::field::{self, DropdownInteraction};
use otto_kit::prelude::*;
use skia_safe::{surfaces, Color, EncodedImageFormat, Paint, Rect};

struct Variant {
    caption: &'static str,
    theme: Theme,
    dark: bool,
    label: &'static str,
    interaction: DropdownInteraction,
}

impl Variant {
    fn new(
        caption: &'static str,
        dark: bool,
        label: &'static str,
        interaction: DropdownInteraction,
    ) -> Self {
        Self {
            caption,
            theme: if dark { Theme::dark() } else { Theme::light() },
            dark,
            label,
            interaction,
        }
    }
}

const CELL_W: f32 = 260.0;
const CELL_H: f32 = 90.0;
const COLS: usize = 3;
const FIELD_W: f32 = 176.0;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dropdown_gallery.png".to_string());

    let variants = [
        Variant::new(
            "light · normal",
            false,
            "Automatic",
            DropdownInteraction::Normal,
        ),
        Variant::new(
            "light · hovered",
            false,
            "Automatic",
            DropdownInteraction::Hovered,
        ),
        Variant::new(
            "light · pressed",
            false,
            "Automatic",
            DropdownInteraction::Pressed,
        ),
        Variant::new(
            "light · open",
            false,
            "Automatic",
            DropdownInteraction::Open,
        ),
        Variant::new(
            "light · disabled",
            false,
            "Automatic",
            DropdownInteraction::Disabled,
        ),
        Variant::new(
            "light · long value clips",
            false,
            "A very long option name that overflows the field",
            DropdownInteraction::Normal,
        ),
        Variant::new(
            "dark · normal",
            true,
            "Automatic",
            DropdownInteraction::Normal,
        ),
        Variant::new(
            "dark · hovered",
            true,
            "Automatic",
            DropdownInteraction::Hovered,
        ),
        Variant::new(
            "dark · pressed",
            true,
            "Automatic",
            DropdownInteraction::Pressed,
        ),
        Variant::new("dark · open", true, "Automatic", DropdownInteraction::Open),
        Variant::new(
            "dark · disabled",
            true,
            "Automatic",
            DropdownInteraction::Disabled,
        ),
        Variant::new(
            "dark · long value clips",
            true,
            "A very long option name that overflows the field",
            DropdownInteraction::Normal,
        ),
    ];

    let rows = variants.len().div_ceil(COLS);
    let scale = 2.0_f32;
    let width = (CELL_W * COLS as f32 * scale) as i32;
    let height = (CELL_H * rows as f32 * scale) as i32;

    let mut surface = surfaces::raster_n32_premul((width, height)).expect("raster surface");
    let canvas = surface.canvas();
    canvas.scale((scale, scale));

    for (i, variant) in variants.iter().enumerate() {
        let col = i % COLS;
        let row = i / COLS;
        canvas.save();
        canvas.translate((col as f32 * CELL_W, row as f32 * CELL_H));
        draw_cell(canvas, variant);
        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&out, data.as_bytes()).expect("write png");
    println!("wrote {out} ({width}x{height})");
}

fn draw_cell(canvas: &Canvas, v: &Variant) {
    let mut bg = Paint::default();
    bg.set_color(if v.dark {
        Color::from_rgb(0x1E, 0x22, 0x2B)
    } else {
        Color::from_rgb(0xE7, 0xE9, 0xEC)
    });
    canvas.draw_rect(Rect::from_wh(CELL_W - 8.0, CELL_H - 8.0), &bg);

    canvas.save();
    canvas.translate((16.0, 14.0));
    Label::new(v.caption)
        .with_style(styles::CAPTION_1)
        .with_color(if v.dark {
            Color::from_argb(0xCC, 0xFF, 0xFF, 0xFF)
        } else {
            Color::from_argb(0xB0, 0x00, 0x00, 0x00)
        })
        .render(canvas);
    canvas.restore();

    let rect = Rect::from_xywh(16.0, 40.0, FIELD_W, field::HEIGHT);
    field::draw(canvas, rect, v.label, v.interaction, &v.theme);
}
