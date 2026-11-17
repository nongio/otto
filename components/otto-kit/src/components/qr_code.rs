//! A QR code drawn as a square: the symbol's modules on a light ground,
//! with the quiet zone a reader needs around it.
//!
//! Dark on light whatever the theme: phone cameras read light-on-dark codes
//! badly. Each module is a whole number of pixels where the square is big
//! enough, so its edges stay sharp, and the symbol is centred in what is
//! left.

// Rust guideline compliant 2026-02-21

use skia_safe::{Canvas, Color, Paint, Rect};

use crate::qr::QrCode;

/// Light modules around the symbol, as the standard asks for.
pub const QUIET_ZONE: usize = 4;

/// The square `code` takes when drawn `side` points wide: the quiet zone
/// and the modules, snapped to whole pixels at `scale`.
fn module_size(code: &QrCode, side: f32, scale: f32) -> f32 {
    let modules = (code.size() + QUIET_ZONE * 2) as f32;
    let px = (side * scale / modules).floor();
    if px >= 1.0 {
        px / scale
    } else {
        side / modules
    }
}

/// Draw `code` centred in `rect`, black modules on a white square, on whole
/// pixels of the canvas's device (its scale read from the canvas).
pub fn draw(canvas: &Canvas, code: &QrCode, rect: Rect) {
    let matrix = canvas.local_to_device_as_3x3();
    let scale = matrix.scale_x().abs().max(f32::EPSILON);
    let side = rect.width().min(rect.height());
    let module = module_size(code, side, scale);
    let drawn = module * (code.size() + QUIET_ZONE * 2) as f32;
    let mut left = rect.left + (rect.width() - drawn) / 2.0;
    let mut top = rect.top + (rect.height() - drawn) / 2.0;
    // The corner on a device pixel, so every module edge is on one too.
    let device = matrix.map_point((left, top));
    left += (device.x.round() - device.x) / scale;
    top += (device.y.round() - device.y) / scale;

    let mut paint = Paint::default();
    paint.set_anti_alias(false);
    paint.set_color(Color::WHITE);
    canvas.draw_rect(Rect::from_xywh(left, top, drawn, drawn), &paint);

    paint.set_color(Color::BLACK);
    let origin = QUIET_ZONE as f32 * module;
    for y in 0..code.size() {
        // One rectangle per run of dark modules along the row.
        let mut x = 0;
        while x < code.size() {
            if !code.dark(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < code.size() && code.dark(x, y) {
                x += 1;
            }
            canvas.draw_rect(
                Rect::from_xywh(
                    left + origin + start as f32 * module,
                    top + origin + y as f32 * module,
                    (x - start) as f32 * module,
                    module,
                ),
                &paint,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_are_whole_pixels_when_there_is_room() {
        let code = QrCode::encode(b"otto").unwrap();
        // 21 modules and 8 of quiet zone: 29.
        assert_eq!(module_size(&code, 200.0, 1.0), 6.0);
        assert_eq!(module_size(&code, 200.0, 2.0), 13.0 / 2.0);
        // Too small for a pixel each: the square is filled exactly.
        assert_eq!(module_size(&code, 14.5, 1.0), 0.5);
    }

    #[test]
    fn it_draws_dark_on_light() {
        let code = QrCode::encode(b"otto").unwrap();
        let mut surface = skia_safe::surfaces::raster_n32_premul((58, 58)).unwrap();
        draw(surface.canvas(), &code, Rect::from_wh(58.0, 58.0));
        let image = surface.image_snapshot();
        let info = skia_safe::ImageInfo::new_n32_premul((58, 58), None);
        let mut pixels = vec![0u8; 58 * 58 * 4];
        assert!(image.read_pixels(
            &info,
            &mut pixels,
            58 * 4,
            (0, 0),
            skia_safe::image::CachingHint::Allow
        ));
        let dark = |x: usize, y: usize| pixels[(y * 58 + x) * 4] < 128;
        // Two pixels a module, the quiet zone first: the top-left finder's
        // corner is dark, the zone around it light.
        assert!(!dark(7, 7));
        assert!(dark(8, 8));
        assert!(dark(9, 9));
        // The finder's light ring.
        assert!(!dark(10, 10));
    }
}
