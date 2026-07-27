//! Lottie animation player using Skia's Skottie module

use skia_safe::skottie;
use std::sync::{Arc, OnceLock};

/// Simple Lottie animation player
pub struct LottiePlayer {
    animation: Arc<skottie::Animation>,
    duration: f64,
    /// Measured on first use by [`LottiePlayer::content_bounds`].
    content_bounds: OnceLock<skia_safe::Rect>,
}

/// Longest edge of the raster probe used to measure the artwork's bounds. Big
/// enough that a thin stroke still lands on a pixel, small enough that the
/// handful of renders it takes are not worth caching to disk.
const PROBE_EDGE: f32 = 256.0;

/// How many frames the probe samples. An animation that draws itself in covers
/// different ground at different times, so the bounds are the union over the
/// timeline rather than the extent of any single frame.
const PROBE_SAMPLES: usize = 8;

impl LottiePlayer {
    /// Load a Lottie animation from JSON data
    pub fn from_json(json_data: &[u8]) -> Result<Self, String> {
        let json_str =
            std::str::from_utf8(json_data).map_err(|e| format!("Invalid UTF-8 in JSON: {e}"))?;
        Self::parse(json_str)
    }

    /// Load a Lottie animation from a JSON string
    pub fn parse(json: &str) -> Result<Self, String> {
        let animation = skottie::Animation::from_str(json).ok_or("Failed to parse Lottie JSON")?;
        let duration = animation.duration() as f64;
        Ok(Self {
            animation: Arc::new(animation),
            duration,
            content_bounds: OnceLock::new(),
        })
    }

    /// Load from JSON data, replacing the stroke/fill color before parsing.
    /// `color` is an RGBA array [r, g, b, a] with values 0.0..1.0.
    pub fn from_json_with_color(json_data: &[u8], color: [f32; 4]) -> Result<Self, String> {
        let json_str =
            std::str::from_utf8(json_data).map_err(|e| format!("Invalid UTF-8 in JSON: {e}"))?;
        let colored = replace_stroke_color(json_str, color);
        Self::parse(&colored)
    }

    /// Get animation duration in seconds
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Get animation size
    pub fn size(&self) -> (f32, f32) {
        let size = self.animation.size();
        (size.width, size.height)
    }

    /// Render animation to canvas at given time (0.0 to 1.0 progress)
    pub fn render(
        &self,
        canvas: &skia_safe::Canvas,
        progress: f64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        let time = (progress.clamp(0.0, 1.0) * self.duration).max(0.0);

        canvas.save();
        canvas.translate((x, y));

        let (w, h) = self.size();
        if w > 0.0 && h > 0.0 {
            canvas.scale((width / w, height / h));
        }

        self.animation.seek(time as f32);
        self.animation.render(canvas, None);

        canvas.restore();
    }

    /// Where the artwork actually is inside the animation's own canvas, as
    /// fractions of it.
    ///
    /// Lottie assets are routinely exported with generous padding — the Touch
    /// ID mark covers barely a third of its 800×600 canvas — so scaling the
    /// *canvas* into a box, which is what [`LottiePlayer::render`] does, leaves
    /// the artwork a fraction of the size the box asked for. Callers that want
    /// the artwork itself to fill a box use [`LottiePlayer::render_fit`], which
    /// is what this measurement is for.
    ///
    /// Skottie exposes no bounds query, so the artwork is found by rasterising
    /// the animation once and taking the extent of the pixels it touched. An
    /// animation that draws nothing at all reports an empty rect.
    pub fn content_bounds(&self) -> skia_safe::Rect {
        *self.content_bounds.get_or_init(|| self.measure_content())
    }

    fn measure_content(&self) -> skia_safe::Rect {
        let (w, h) = self.size();
        if w <= 0.0 || h <= 0.0 {
            return skia_safe::Rect::new_empty();
        }

        let scale = PROBE_EDGE / w.max(h);
        let (probe_w, probe_h) = ((w * scale) as i32, (h * scale) as i32);
        let Some(mut surface) = skia_safe::surfaces::raster_n32_premul((probe_w, probe_h)) else {
            return skia_safe::Rect::new_empty();
        };

        for sample in 0..PROBE_SAMPLES {
            let progress = (sample + 1) as f64 / PROBE_SAMPLES as f64;
            let canvas = surface.canvas();
            canvas.save();
            canvas.scale((scale, scale));
            self.animation.seek((progress * self.duration) as f32);
            self.animation.render(canvas, None);
            canvas.restore();
        }

        let image = surface.image_snapshot();
        let Some(pixels) = image.peek_pixels() else {
            return skia_safe::Rect::new_empty();
        };

        let (mut left, mut top) = (probe_w, probe_h);
        let (mut right, mut bottom) = (0, 0);
        for y in 0..probe_h {
            for x in 0..probe_w {
                // Anti-aliased edges of a thin stroke can be very faint; the
                // threshold is only there to reject arithmetic dust.
                if pixels.get_color((x, y)).a() > 2 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        if left >= right || top >= bottom {
            return skia_safe::Rect::new_empty();
        }

        skia_safe::Rect::new(
            left as f32 / probe_w as f32,
            top as f32 / probe_h as f32,
            right as f32 / probe_w as f32,
            bottom as f32 / probe_h as f32,
        )
    }

    /// Render the animation so its *artwork* fits `dst`, ignoring whatever
    /// padding the asset's canvas carries around it. The aspect ratio is kept
    /// and the result is centred, so the artwork touches two sides of `dst`.
    pub fn render_fit(&self, canvas: &skia_safe::Canvas, progress: f64, dst: skia_safe::Rect) {
        self.fit(canvas, progress, dst, None);
    }

    /// [`LottiePlayer::render_fit`], recoloured. Useful when the same asset
    /// stands for several states and the colour is what tells them apart —
    /// tinting at draw time avoids parsing the JSON once per colour.
    pub fn render_fit_with_color(
        &self,
        canvas: &skia_safe::Canvas,
        progress: f64,
        dst: skia_safe::Rect,
        color: skia_safe::Color,
    ) {
        self.fit(canvas, progress, dst, Some(color));
    }

    fn fit(
        &self,
        canvas: &skia_safe::Canvas,
        progress: f64,
        dst: skia_safe::Rect,
        color: Option<skia_safe::Color>,
    ) {
        let bounds = self.content_bounds();
        let (w, h) = self.size();
        if bounds.is_empty() || w <= 0.0 || h <= 0.0 {
            // Nothing measurable: fall back to filling the box with the canvas,
            // which is at least in the right place.
            match color {
                Some(color) => self.render_with_color(
                    canvas,
                    progress,
                    dst.x(),
                    dst.y(),
                    dst.width(),
                    dst.height(),
                    color,
                ),
                None => self.render(
                    canvas,
                    progress,
                    dst.x(),
                    dst.y(),
                    dst.width(),
                    dst.height(),
                ),
            }
            return;
        }

        let art = skia_safe::Rect::new(
            bounds.left * w,
            bounds.top * h,
            bounds.right * w,
            bounds.bottom * h,
        );
        let scale = (dst.width() / art.width()).min(dst.height() / art.height());

        canvas.save();
        canvas.translate((dst.center_x(), dst.center_y()));
        canvas.scale((scale, scale));
        canvas.translate((-art.center_x(), -art.center_y()));
        self.animation
            .seek((progress.clamp(0.0, 1.0) * self.duration).max(0.0) as f32);

        match color
            .and_then(|color| skia_safe::color_filters::blend(color, skia_safe::BlendMode::SrcIn))
        {
            Some(filter) => {
                let mut paint = skia_safe::Paint::default();
                paint.set_color_filter(filter);
                let layer_rec = skia_safe::canvas::SaveLayerRec::default()
                    .bounds(&art)
                    .paint(&paint);
                canvas.save_layer(&layer_rec);
                self.animation.render(canvas, None);
                canvas.restore();
            }
            None => self.animation.render(canvas, None),
        }
        canvas.restore();
    }

    /// Render with a color filter applied (tints the entire animation)
    #[allow(clippy::too_many_arguments)]
    pub fn render_with_color(
        &self,
        canvas: &skia_safe::Canvas,
        progress: f64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: skia_safe::Color,
    ) {
        let time = (progress.clamp(0.0, 1.0) * self.duration).max(0.0);

        canvas.save();
        canvas.translate((x, y));

        let (w, h) = self.size();
        if w > 0.0 && h > 0.0 {
            canvas.scale((width / w, height / h));
        }

        self.animation.seek(time as f32);

        // Use a save layer with a color filter to tint the animation
        let color_filter = skia_safe::color_filters::blend(color, skia_safe::BlendMode::SrcIn);
        if let Some(filter) = color_filter {
            let mut paint = skia_safe::Paint::default();
            paint.set_color_filter(filter);
            let bounds = skia_safe::Rect::from_wh(w, h);
            let layer_rec = skia_safe::canvas::SaveLayerRec::default()
                .bounds(&bounds)
                .paint(&paint);
            canvas.save_layer(&layer_rec);
            self.animation.render(canvas, None);
            canvas.restore(); // restore save_layer
        } else {
            self.animation.render(canvas, None);
        }

        canvas.restore();
    }
}

/// Replace all static color values `"c":{"a":0,"k":[r,g,b,a],...}` in Lottie JSON.
fn replace_stroke_color(json: &str, color: [f32; 4]) -> String {
    let color_str = format!("[{},{},{},{}]", color[0], color[1], color[2], color[3]);
    let needle = "\"c\":{\"a\":0,\"k\":";
    let mut result = json.to_string();
    let mut search_from = 0;
    while let Some(pos) = result[search_from..].find(needle) {
        let abs_pos = search_from + pos;
        let start = abs_pos + needle.len();
        // Find the opening [ and closing ] of the color array
        if result.as_bytes().get(start) == Some(&b'[') {
            if let Some(bracket) = result[start..].find(']') {
                let end = start + bracket + 1; // include the ]
                result.replace_range(start..end, &color_str);
                search_from = start + color_str.len();
            } else {
                break;
            }
        } else {
            // Not a static color (could be animated), skip
            search_from = start;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200×200 canvas with a single white 40×40 square centred on (70, 70),
    /// so the artwork occupies exactly the quarter-to-half of each axis and
    /// three quarters of the canvas is padding — the shape of the problem
    /// [`LottiePlayer::content_bounds`] exists to solve.
    const SQUARE: &str = r#"{
      "v":"5.5.7","fr":30,"ip":0,"op":30,"w":200,"h":200,"ddd":0,"assets":[],
      "layers":[{"ddd":0,"ind":1,"ty":4,"nm":"square","sr":1,"ao":0,"ip":0,"op":30,"st":0,
        "ks":{"o":{"a":0,"k":100},"r":{"a":0,"k":0},"p":{"a":0,"k":[0,0,0]},
              "a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},
        "shapes":[{"ty":"gr","it":[
          {"ty":"rc","d":1,"s":{"a":0,"k":[40,40]},"p":{"a":0,"k":[70,70]},"r":{"a":0,"k":0}},
          {"ty":"fl","c":{"a":0,"k":[1,1,1,1]},"o":{"a":0,"k":100}},
          {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},
           "s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}
        ]}]}]
    }"#;

    /// Bounds of the non-transparent pixels of a raster surface, in pixels.
    fn painted_bounds(surface: &mut skia_safe::Surface) -> skia_safe::IRect {
        let (w, h) = (surface.width(), surface.height());
        let image = surface.image_snapshot();
        let pixels = image.peek_pixels().unwrap();
        let mut bounds = skia_safe::IRect::new(w, h, 0, 0);
        for y in 0..h {
            for x in 0..w {
                if pixels.get_color((x, y)).a() > 2 {
                    bounds.left = bounds.left.min(x);
                    bounds.top = bounds.top.min(y);
                    bounds.right = bounds.right.max(x + 1);
                    bounds.bottom = bounds.bottom.max(y + 1);
                }
            }
        }
        bounds
    }

    #[test]
    fn content_bounds_find_the_artwork_inside_the_padding() {
        let player = LottiePlayer::parse(SQUARE).unwrap();
        let bounds = player.content_bounds();

        // The probe rasterises, so allow a pixel of slop at 256 across.
        for (found, expected) in [
            (bounds.left, 0.25),
            (bounds.top, 0.25),
            (bounds.right, 0.45),
            (bounds.bottom, 0.45),
        ] {
            assert!(
                (found - expected).abs() < 0.01,
                "expected {expected}, found {found} in {bounds:?}"
            );
        }
    }

    /// The point of the exercise: a box handed to `render_fit` comes back full,
    /// where `render` leaves the artwork covering a fifth of each axis.
    #[test]
    fn render_fit_fills_the_box_that_render_does_not() {
        let player = LottiePlayer::parse(SQUARE).unwrap();

        // The measured bounds are quantised to the probe's pixels, so the fit
        // can fall a pixel short of the box; it must not fall further.
        let mut fitted = skia_safe::surfaces::raster_n32_premul((100, 100)).unwrap();
        player.render_fit(fitted.canvas(), 1.0, skia_safe::Rect::from_wh(100.0, 100.0));
        let bounds = painted_bounds(&mut fitted);
        assert!(
            bounds.left <= 1 && bounds.top <= 1 && bounds.right >= 99 && bounds.bottom >= 99,
            "the artwork should fill the box, found {bounds:?}"
        );

        // Scaling the canvas instead puts the same square in the middle fifth.
        let mut scaled = skia_safe::surfaces::raster_n32_premul((100, 100)).unwrap();
        player.render(scaled.canvas(), 1.0, 0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            painted_bounds(&mut scaled),
            skia_safe::IRect::new(25, 25, 45, 45)
        );
    }

    /// An asset that draws nothing must not send `render_fit` through a
    /// divide-by-zero; it falls back to scaling the canvas.
    #[test]
    fn an_empty_animation_has_empty_bounds() {
        let empty = r#"{"v":"5.5.7","fr":30,"ip":0,"op":30,"w":100,"h":100,"ddd":0,
                        "assets":[],"layers":[]}"#;
        let player = LottiePlayer::parse(empty).unwrap();
        assert!(player.content_bounds().is_empty());

        let mut surface = skia_safe::surfaces::raster_n32_premul((10, 10)).unwrap();
        player.render_fit(surface.canvas(), 1.0, skia_safe::Rect::from_wh(10.0, 10.0));
    }
}
