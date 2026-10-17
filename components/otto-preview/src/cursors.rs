//! The cursors the theme has no names for: a pencil while the pen is on, and
//! a bin over a mark's badge, where a click deletes the mark.
//!
//! Drawn here and handed to the toolkit as pictures. Everything else is a
//! named shape. The viewer still thinks in [`CursorShape`]s: the pen is
//! `Crosshair` and the bin `NotAllowed`, which [`show`] turns into the
//! pictures, so one comparison keeps telling a change from none.

// Rust guideline compliant 2026-02-21

use otto_kit::prelude::*;
use otto_kit::skia::{PaintCap, PaintJoin, PaintStyle, PathBuilder};
use otto_kit::{CursorImage, CursorShape};

/// The cursor's size, in points.
const SIZE: f32 = 24.0;
/// Pixels to a point: sharp on a doubled display, scaled down elsewhere.
const SCALE: u32 = 2;

/// Show `shape`, as a picture when it stands for the pencil or the bin.
pub fn show(shape: CursorShape) {
    match shape {
        CursorShape::Crosshair => AppContext::set_cursor_image(pencil()),
        CursorShape::NotAllowed => AppContext::set_cursor_image(bin()),
        other => AppContext::set_cursor_shape(other),
    }
}

/// Draw a cursor: `paint` draws in points on a `SIZE`-point canvas.
fn picture(key: &str, hotspot: (i32, i32), paint: impl Fn(&Canvas)) -> CursorImage {
    let pixels = (SIZE as u32 * SCALE) as i32;
    let mut pixels_out = Vec::new();
    if let Some(mut surface) = otto_kit::skia::surfaces::raster_n32_premul((pixels, pixels)) {
        let canvas = surface.canvas();
        canvas.clear(Color::TRANSPARENT);
        canvas.scale((SCALE as f32, SCALE as f32));
        paint(canvas);
        let info = otto_kit::skia::ImageInfo::new_n32_premul((pixels, pixels), None);
        pixels_out = vec![0; (pixels * pixels * 4) as usize];
        if !surface.read_pixels(&info, &mut pixels_out, (pixels * 4) as usize, (0, 0)) {
            pixels_out.clear();
        }
    }
    CursorImage {
        key: key.to_owned(),
        pixels: pixels_out,
        width: pixels as u32,
        height: pixels as u32,
        scale: SCALE,
        hotspot,
    }
}

/// A stroke of `width` points, dark inside a white edge so it reads on any
/// picture.
fn outlined(canvas: &Canvas, path: &otto_kit::skia::Path, width: f32) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_join(PaintJoin::Round);
    paint.set_stroke_cap(PaintCap::Round);
    paint.set_stroke_width(width + 2.5);
    paint.set_color(Color::WHITE);
    canvas.draw_path(path, &paint);
    paint.set_stroke_width(width);
    paint.set_color(Color::from_rgb(30, 30, 32));
    canvas.draw_path(path, &paint);
}

/// A pencil, its tip at the lower left: the hotspot, where the line lands.
fn pencil() -> CursorImage {
    picture("otto-preview-pencil", (3, 21), |canvas| {
        let mut body = PathBuilder::new();
        // The barrel from the eraser end down to where the point starts,
        // then the point to the tip.
        body.move_to((17.0, 3.0));
        body.line_to((21.0, 7.0));
        body.line_to((8.0, 20.0));
        body.line_to((3.0, 21.0));
        body.line_to((4.0, 16.0));
        body.close();
        let body = body.detach();
        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(Color::from_rgb(255, 214, 10));
        canvas.draw_path(&body, &fill);
        outlined(canvas, &body, 1.5);
        let mut line = PathBuilder::new();
        line.move_to((4.0, 16.0));
        line.line_to((8.0, 20.0));
        line.move_to((14.5, 5.5));
        line.line_to((18.5, 9.5));
        outlined(canvas, &line.detach(), 1.5);
    })
}

/// A bin, centred on the pointer.
fn bin() -> CursorImage {
    picture("otto-preview-bin", (12, 12), |canvas| {
        let mut bin = PathBuilder::new();
        // The lid and its handle.
        bin.move_to((5.0, 7.0));
        bin.line_to((19.0, 7.0));
        bin.move_to((10.0, 7.0));
        bin.line_to((10.0, 4.5));
        bin.line_to((14.0, 4.5));
        bin.line_to((14.0, 7.0));
        // The can, narrowing to its foot.
        bin.move_to((6.5, 7.0));
        bin.line_to((7.5, 20.0));
        bin.line_to((16.5, 20.0));
        bin.line_to((17.5, 7.0));
        // Its ribs.
        bin.move_to((10.5, 10.0));
        bin.line_to((10.5, 17.0));
        bin.move_to((13.5, 10.0));
        bin.line_to((13.5, 17.0));
        outlined(canvas, &bin.detach(), 1.6);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_are_drawn_at_their_size() {
        for image in [pencil(), bin()] {
            assert_eq!(image.width, SIZE as u32 * SCALE);
            assert_eq!(
                image.pixels.len(),
                (image.width * image.height * 4) as usize
            );
            assert!(
                image.pixels.chunks(4).any(|pixel| pixel[3] > 0),
                "something is drawn"
            );
        }
    }
}
