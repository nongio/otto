//! Lottie animation player using Skia's Skottie module

use skia_safe::skottie;
use std::sync::Arc;

/// Simple Lottie animation player
pub struct LottiePlayer {
    animation: Arc<skottie::Animation>,
    duration: f64,
}

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
