use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI32, Ordering};

use layers::{prelude::*, skia};

/// Fallback wallpaper decode size, used until an output has reported its mode.
///
/// Big enough to look right on a laptop panel, small enough that a wallpaper
/// set before the first output exists costs nothing much.
const WALLPAPER_FALLBACK_EDGE_PX: i32 = 2048;

/// Hard ceiling on either edge of a decoded wallpaper, in physical pixels.
///
/// The decode size is derived from the real output below, so this only bites
/// on exotic displays; it is here to keep the wallpaper inside the smallest
/// `GL_MAX_TEXTURE_SIZE` worth relying on, and to keep one image from eating
/// hundreds of megabytes.
const WALLPAPER_MAX_EDGE_PX: i32 = 8192;

/// Physical size of the largest output, which is what a wallpaper is decoded
/// for. Global because every workspace decodes its own copy, and none of them
/// knows about outputs.
static WALLPAPER_TARGET_W_PX: AtomicI32 = AtomicI32::new(0);
static WALLPAPER_TARGET_H_PX: AtomicI32 = AtomicI32::new(0);

/// Tell the wallpaper decoder how big the biggest output is, in physical
/// pixels. Returns `true` when that is *larger* than what wallpapers were last
/// decoded for, i.e. when the current ones are now upscaled and the caller
/// should reload them.
pub fn set_wallpaper_target_px(width_px: f32, height_px: f32) -> bool {
    let (w, h) = (width_px as i32, height_px as i32);
    if w <= 0 || h <= 0 {
        return false;
    }
    let grew = w > WALLPAPER_TARGET_W_PX.load(Ordering::Relaxed)
        || h > WALLPAPER_TARGET_H_PX.load(Ordering::Relaxed);
    WALLPAPER_TARGET_W_PX.store(w, Ordering::Relaxed);
    WALLPAPER_TARGET_H_PX.store(h, Ordering::Relaxed);
    grew
}

/// The output box a wallpaper has to cover, in physical pixels.
fn wallpaper_target_px() -> (i32, i32) {
    let w = WALLPAPER_TARGET_W_PX.load(Ordering::Relaxed);
    let h = WALLPAPER_TARGET_H_PX.load(Ordering::Relaxed);
    if w <= 0 || h <= 0 {
        (WALLPAPER_FALLBACK_EDGE_PX, WALLPAPER_FALLBACK_EDGE_PX)
    } else {
        (w.min(WALLPAPER_MAX_EDGE_PX), h.min(WALLPAPER_MAX_EDGE_PX))
    }
}

/// Decode the wallpaper at `path` at a size that suits the screen it will be
/// drawn on.
///
/// [`view_background`] scales the image to *cover* the output, so anything
/// smaller than the cover size is visibly upscaled — which is what a fixed
/// 2048x2048 decode used to do to every wallpaper on a display bigger than
/// that. The target is the largest output's physical mode instead, capped by
/// [`WALLPAPER_MAX_EDGE_PX`].
///
/// A bigger image than that is resampled down to exactly the cover size, so
/// the draw scale lands on 1.0. That costs one resample at startup — noticeable
/// only on a very large photo — and is paid back in texture memory and in
/// sampling quality, since the GPU would otherwise minify it without mipmaps
/// on every frame. Images already at or below the cover size are left alone:
/// enlarging them here would only waste memory, the draw upscales either way.
pub fn decode_wallpaper(path: &str) -> Option<skia::Image> {
    let (target_w, target_h) = wallpaper_target_px();
    // SVGs are rasterized at the size passed here; raster formats ignore it and
    // decode at their own resolution. A square keeps the framing an SVG
    // wallpaper has always had, at the screen's resolution rather than 2048.
    let edge = target_w.max(target_h);
    let image = crate::utils::image_from_path(path, (edge, edge))?;
    Some(downscale_to_cover(image, target_w, target_h))
}

/// Shrink `image` to the smallest size that still covers `target_w x target_h`,
/// or return it untouched when it is already that small.
fn downscale_to_cover(image: skia::Image, target_w: i32, target_h: i32) -> skia::Image {
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    if iw <= 0.0 || ih <= 0.0 {
        return image;
    }
    let cover = (target_w as f32 / iw).max(target_h as f32 / ih);
    if cover >= 1.0 {
        return image;
    }
    let (w, h) = (
        (iw * cover).ceil().max(1.0) as i32,
        (ih * cover).ceil().max(1.0) as i32,
    );
    let Some(mut surface) = skia::surfaces::raster_n32_premul((w, h)) else {
        return image;
    };
    let sampling = skia::SamplingOptions::from(skia::CubicResampler::mitchell());
    surface.canvas().draw_image_rect_with_sampling_options(
        &image,
        None,
        skia::Rect::from_iwh(w, h),
        sampling,
        &skia::Paint::default(),
    );
    surface.image_snapshot()
}

#[derive(Clone, Debug)]
pub struct BackgroundViewState {
    pub image: Option<skia::Image>,
    pub debug_string: String,
    pub fallback_color: skia::Color4f,
}
impl Hash for BackgroundViewState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if let Some(image) = self.image.as_ref() {
            image.unique_id().hash(state);
        }
        self.debug_string.hash(state);
        // Hash the color components
        self.fallback_color.r.to_bits().hash(state);
        self.fallback_color.g.to_bits().hash(state);
        self.fallback_color.b.to_bits().hash(state);
        self.fallback_color.a.to_bits().hash(state);
    }
}

pub struct BackgroundView {
    // engine: layers::prelude::LayersEngine,
    pub view: layers::prelude::View<BackgroundViewState>,
    // pub state: RwLock<BackgroundViewState>,
    pub base_layer: Layer,
}

impl BackgroundView {
    pub fn new(index: usize, layer: Layer, fallback_color: skia::Color4f) -> Self {
        let state = BackgroundViewState {
            image: None,
            debug_string: "Screen composer 0.1".to_string(),
            fallback_color,
        };
        let view = layers::prelude::View::new(
            format!("background_view_{}", index),
            state,
            Box::new(view_background),
        );
        view.mount_layer(layer.clone());
        // The draw callback fills the entire bounds with opaque pixels (image or gradient).
        layer.set_content_opaque(true);
        Self {
            view,
            base_layer: layer,
        }
    }

    pub fn set_debug_text(&self, text: String) {
        self.view.update_state(&BackgroundViewState {
            debug_string: text,
            ..self.view.get_state()
        });
    }

    /// Point the background at `path`, or clear it back to the fallback
    /// gradient when the path is empty or cannot be decoded.
    ///
    /// Clearing is why this exists rather than callers using
    /// [`Self::set_image`]: emptying the setting has to *remove* the
    /// wallpaper, and there is no image to pass for that.
    pub fn set_image_path(&self, path: &str) -> bool {
        if path.is_empty() {
            self.clear_image();
            return true;
        }
        match decode_wallpaper(path) {
            Some(image) => {
                self.set_image(image);
                true
            }
            None => false,
        }
    }

    /// Drop back to the fallback gradient.
    pub fn clear_image(&self) {
        self.view.update_state(&BackgroundViewState {
            image: None,
            ..self.view.get_state()
        });
    }

    /// Change the colour the gradient is drawn from, for when no wallpaper is
    /// set. Repainting is the view's job — the state hash covers the colour.
    pub fn set_fallback_color(&self, color: skia::Color4f) {
        self.view.update_state(&BackgroundViewState {
            fallback_color: color,
            ..self.view.get_state()
        });
    }

    pub fn set_image(&self, image: skia::Image) {
        // Force an eager decode. `Image::from_encoded` is LAZY: every playback
        // re-decodes, and on the GPU that decode+upload happens at flush time,
        // where it can silently produce nothing under memory pressure — the
        // wallpaper then paints black on exactly the frames that redraw the
        // background plane around an exposé transition (CPU playback of the
        // same picture always decodes fine, which is how this was isolated).
        // A raster image uploads from stable pixels instead.
        let image = image.make_raster_image(None, None).unwrap_or(image);
        self.view.update_state(&BackgroundViewState {
            image: Some(image),
            ..self.view.get_state()
        });
    }
}

// static mut COUNTER: f32 = 1.0;
pub fn view_background(
    state: &BackgroundViewState,
    _view: &View<BackgroundViewState>,
) -> LayerTree {
    let image = state.image.clone();
    let fallback_color = state.fallback_color;

    // let debug_text = state.debug_string.clone();

    let draw_container = move |canvas: &skia::Canvas, w, h| {
        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);

        if let Some(image) = image.as_ref() {
            let mut matrix = skia::Matrix::new_identity();
            let image_width = image.width() as f32;
            let image_height = image.height() as f32;
            let scale_x: f32 = w / image_width;
            let scale_y: f32 = h / image_height;
            let scale = scale_x.max(scale_y); // Choose the smaller scale to maintain aspect ratio

            // Calculate the offsets for centering the image
            let offset_x = (w - image_width * scale) / 2.0;
            let offset_y = (h - image_height * scale) / 2.0;

            matrix.set_scale_translate((scale, scale), (offset_x, offset_y)); // canvas.concat(&matrix);
                                                                              // canvas.draw_image_rect(image, None, rect, &paint);
            paint.set_shader(image.to_shader(
                (skia::TileMode::Repeat, skia::TileMode::Repeat),
                skia::SamplingOptions::default(),
                &matrix,
            ));
        } else {
            // Create a gradient from bottom (fallback_color) to top (lighter version)
            // Make the top color lighter by interpolating towards white
            let lighter_factor = 0.3; // 30% lighter
            let top_color = skia::Color4f::new(
                fallback_color.r + (1.0 - fallback_color.r) * lighter_factor,
                fallback_color.g + (1.0 - fallback_color.g) * lighter_factor,
                fallback_color.b + (1.0 - fallback_color.b) * lighter_factor,
                fallback_color.a,
            );

            // Convert Color4f to Color for gradient shader
            let colors = [
                skia::Color::from_argb(
                    (fallback_color.a * 255.0) as u8,
                    (fallback_color.r * 255.0) as u8,
                    (fallback_color.g * 255.0) as u8,
                    (fallback_color.b * 255.0) as u8,
                ),
                skia::Color::from_argb(
                    (top_color.a * 255.0) as u8,
                    (top_color.r * 255.0) as u8,
                    (top_color.g * 255.0) as u8,
                    (top_color.b * 255.0) as u8,
                ),
            ];
            let positions: &[f32] = &[0.0, 1.0];

            let start_point = skia::Point::new(0.0, h); // Bottom
            let end_point = skia::Point::new(0.0, 0.0); // Top

            if let Some(shader) = skia::gradient_shader::linear(
                (start_point, end_point),
                skia::gradient_shader::GradientShaderColors::Colors(&colors),
                Some(positions),
                skia::TileMode::Clamp,
                None,
                None,
            ) {
                paint.set_shader(shader);
            }
        }

        let split = 1;
        let rect_size_w = w / split as f32;
        let rect_size_h = h / split as f32;

        for i in 0..split {
            for j in 0..split {
                let rect = skia::Rect::from_xywh(
                    i as f32 * rect_size_w,
                    j as f32 * rect_size_h,
                    rect_size_w,
                    rect_size_h,
                );
                canvas.draw_rect(rect, &paint);
            }
        }

        // let color = skia::Color4f::new(0.0, 0.0, 0.0, 1.0);
        // let paint = skia::Paint::new(color, None);
        // let mut font = skia::Font::default();
        // let font_size = 26.0;
        // font.set_size(font_size);
        // canvas.draw_str("test string string", (80.0, 100.0), &font, &paint);
        // canvas.draw_rect(skia::Rect::from_xywh(80.0, 100.0, 200.0, 100.0), &paint);
        skia::Rect::from_xywh(0.0, 0.0, w, h)
    };

    LayerTreeBuilder::default()
        .key("background_view")
        .opacity((
            1.0,
            Some(Transition {
                delay: 0.2,
                timing: TimingFunction::ease_out_quad(0.8),
            }),
        ))
        .border_corner_radius(BorderRadius::new_single(24.0))
        .content(Some(draw_container))
        // .image_cache(true)
        .background_color(layers::prelude::Color::new_rgba(0.0, 0.0, 0.0, 1.0))
        .pointer_events(false)
        .build()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: i32, h: i32) -> skia::Image {
        let mut surface = skia::surfaces::raster_n32_premul((w, h)).unwrap();
        surface.canvas().clear(skia::Color::RED);
        surface.image_snapshot()
    }

    /// A wallpaper larger than the screen is reduced to exactly the size that
    /// covers it — never below, or the draw would upscale it again.
    #[test]
    fn an_oversized_wallpaper_is_reduced_to_the_cover_size() {
        let scaled = downscale_to_cover(solid(7680, 4320), 2880, 1920);
        assert!(scaled.width() >= 2880 && scaled.height() >= 1920);
        // Cover is driven by the taller ratio here, so height lands on target.
        assert_eq!(scaled.height(), 1920);
    }

    /// Anything already at or below the cover size is left alone: enlarging it
    /// here would only cost memory, the draw scales it either way.
    #[test]
    fn a_smaller_wallpaper_is_left_alone() {
        let image = solid(2560, 1600);
        let kept = downscale_to_cover(image.clone(), 2880, 1920);
        assert_eq!((kept.width(), kept.height()), (2560, 1600));
    }

    /// Without an output the decode falls back to the old fixed bound; the
    /// first output that reports a bigger mode asks for a reload.
    ///
    /// The target is process-global, so this is the one test allowed to move
    /// it — and it runs alone.
    #[test]
    #[serial_test::serial]
    fn the_target_grows_with_the_biggest_output() {
        assert_eq!(
            wallpaper_target_px(),
            (WALLPAPER_FALLBACK_EDGE_PX, WALLPAPER_FALLBACK_EDGE_PX)
        );
        assert!(set_wallpaper_target_px(2880.0, 1920.0));
        assert_eq!(wallpaper_target_px(), (2880, 1920));
        // A second, smaller output does not shrink what is already decoded.
        assert!(!set_wallpaper_target_px(1920.0, 1080.0));
        // And nothing exceeds the texture-size ceiling.
        set_wallpaper_target_px(20000.0, 20000.0);
        assert_eq!(
            wallpaper_target_px(),
            (WALLPAPER_MAX_EDGE_PX, WALLPAPER_MAX_EDGE_PX)
        );
    }
}
