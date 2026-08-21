//! Offscreen gallery of the toggle and slider form controls.
//!
//! Renders every interaction state, in both themes, straight to a PNG with
//! Skia's raster backend — no compositor, no Wayland. Follows
//! `titlebar_gallery.rs`.
//!
//! ```sh
//! cargo run -p otto-kit --example form_controls_gallery -- /tmp/form_controls.png
//! ```

use otto_kit::components::slider::{self, SliderInteraction};
use otto_kit::components::toggle::{self, ToggleInteraction};
use otto_kit::prelude::*;
use skia_safe::{surfaces, Color, EncodedImageFormat, Paint, Rect};

struct Variant {
    caption: &'static str,
    theme: Theme,
    dark: bool,
    interaction: &'static str,
}

impl Variant {
    fn new(caption: &'static str, dark: bool, interaction: &'static str) -> Self {
        Self {
            caption,
            theme: if dark { Theme::dark() } else { Theme::light() },
            dark,
            interaction,
        }
    }

    fn toggle_state(&self) -> ToggleInteraction {
        match self.interaction {
            "hovered" => ToggleInteraction::Hovered,
            "pressed" => ToggleInteraction::Pressed,
            "disabled" => ToggleInteraction::Disabled,
            _ => ToggleInteraction::Normal,
        }
    }

    fn slider_state(&self) -> SliderInteraction {
        match self.interaction {
            "hovered" => SliderInteraction::Hovered,
            "pressed" => SliderInteraction::Pressed,
            "disabled" => SliderInteraction::Disabled,
            _ => SliderInteraction::Normal,
        }
    }
}

const CELL_W: f32 = 300.0;
const CELL_H: f32 = 120.0;
const COLS: usize = 3;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "form_controls_gallery.png".to_string());

    let variants = [
        Variant::new("light · off · normal", false, "normal"),
        Variant::new("light · on · hovered", false, "hovered"),
        Variant::new("light · on · pressed", false, "pressed"),
        Variant::new("light · disabled", false, "disabled"),
        Variant::new("dark · off · normal", true, "normal"),
        Variant::new("dark · on · hovered", true, "hovered"),
        Variant::new("dark · on · pressed", true, "pressed"),
        Variant::new("dark · disabled", true, "disabled"),
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
        draw_cell(canvas, variant, i);
        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&out, data.as_bytes()).expect("write png");
    println!("wrote {out} ({width}x{height})");
}

fn draw_cell(canvas: &Canvas, v: &Variant, index: usize) {
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

    // Alternate on/off within each interaction state, so both knob
    // positions and the accent-vs-neutral track colour are all visible
    // across the gallery.
    let on = index % 2 == 1 || v.interaction == "hovered" || v.interaction == "pressed";

    let toggle_rect = Rect::from_xywh(16.0, 44.0, toggle::WIDTH, toggle::HEIGHT);
    toggle::draw(
        canvas,
        toggle_rect,
        toggle::knob_fraction_for(on),
        v.toggle_state(),
        &v.theme,
    );

    let slider_value = if on { 70.0 } else { 25.0 };
    let slider_rect = Rect::from_xywh(16.0, 88.0, 160.0, 24.0);
    let readout = format!("{} px", slider_value as i32);
    slider::draw(
        canvas,
        slider_rect,
        slider_value,
        0.0,
        100.0,
        Some(&readout),
        v.slider_state(),
        &v.theme,
    );
}
