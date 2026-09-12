//! Render window decorations to a PNG, offscreen.
//!
//! The compact bar a tile wears is small enough that a point of type or a
//! point of padding decides whether it reads — and it only ever appears on a
//! running compositor, on a window that happens to be tiled. This draws the
//! same `WindowDecoration` the compositor and every otto-kit client draw,
//! straight into a raster surface, so the bar can be looked at and measured
//! without a session in the way.
//!
//! ```sh
//! cargo run -p otto-kit --example decoration_png -- /tmp/decorations.png
//! ```

use otto_kit::components::titlebar::{DecorationVariant, WindowDecoration};
use skia_safe::{surfaces, Color, EncodedImageFormat, Paint, Rect};

/// Width of each bar drawn, in points.
const WIDTH: f32 = 520.0;
/// Vertical room given to each sample, so the bars do not touch.
const ROW: f32 = 64.0;
/// Scale drawn at, so the type is legible in the file.
const SCALE: f32 = 3.0;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or("/tmp/decorations.png".into());

    let samples: Vec<(&str, WindowDecoration)> = vec![
        (
            "floating / active",
            WindowDecoration::new("Settings", WIDTH).with_corner_radius(12.0),
        ),
        (
            "normal (tiled) / active",
            WindowDecoration::new("Settings", WIDTH).with_variant(DecorationVariant::Normal),
        ),
        (
            "minimal / active",
            WindowDecoration::new("Settings", WIDTH).with_variant(DecorationVariant::Minimal),
        ),
        (
            "minimal / inactive",
            WindowDecoration::new("Settings", WIDTH)
                .with_variant(DecorationVariant::Minimal)
                .with_active(false),
        ),
        (
            "minimal / dark",
            WindowDecoration::new("Settings", WIDTH)
                .with_variant(DecorationVariant::Minimal)
                .with_dark(true),
        ),
    ];

    let height = ROW * samples.len() as f32;
    let mut surface =
        surfaces::raster_n32_premul(((WIDTH * SCALE) as i32, (height * SCALE) as i32)).unwrap();
    let canvas = surface.canvas();
    canvas.clear(Color::from_argb(255, 90, 110, 130));
    canvas.scale((SCALE, SCALE));

    for (i, (name, decoration)) in samples.iter().enumerate() {
        canvas.save();
        canvas.translate((0.0, ROW * i as f32));
        // A slab of window under the bar, so the bar's own edge is visible.
        let mut paint = Paint::default();
        paint.set_color(if decoration.dark {
            Color::from_argb(255, 30, 30, 32)
        } else {
            Color::from_argb(255, 245, 245, 247)
        });
        canvas.draw_rect(Rect::from_wh(WIDTH, ROW - 8.0), &paint);
        decoration.draw(canvas);
        canvas.restore();
        println!("{i}: {name} — height {:.1}", decoration.titlebar_height);
    }

    let image = surface.image_snapshot();
    let data = image.encode(None, EncodedImageFormat::PNG, 100).unwrap();
    std::fs::write(&path, data.as_bytes()).unwrap();
    println!("wrote {path}");
}
