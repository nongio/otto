use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI32, Ordering};

use layers::{prelude::*, skia};
use otto_kit::utils::resample::resample_to;
use otto_kit::wallpaper::{cover, cover_size};

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
/// The image is then resampled to exactly the cover size, so the draw scale
/// is 1.0 and nothing is filtered per frame. That costs one resample when the
/// wallpaper is loaded and is paid back in sampling quality: the GPU would
/// otherwise minify a large photo without mipmaps, or enlarge a small one
/// nearest-neighbour, on every frame.
pub fn decode_wallpaper(path: &str) -> Option<skia::Image> {
    let (target_w, target_h) = wallpaper_target_px();
    // SVGs are rasterized at the size passed here; raster formats ignore it and
    // decode at their own resolution. A square keeps the framing an SVG
    // wallpaper has always had, at the screen's resolution rather than 2048.
    let edge = target_w.max(target_h);
    let image = crate::utils::image_from_path(path, (edge, edge))?;
    Some(resample_to_cover(image, target_w, target_h))
}

/// Resample `image` to the size that covers `target_w x target_h` at exactly
/// 1:1, or return it untouched when it already is that size.
///
/// Both ways: a photo bigger than the screen is reduced, a smaller one is
/// enlarged, so the draw never scales and nearest sampling is exact. The
/// resample is [`otto_kit::utils::resample`]'s, in linear light with a
/// kernel that widens with the ratio, so a large photo shrinks without
/// aliasing and a small one grows without blocks.
fn resample_to_cover(image: skia::Image, target_w: i32, target_h: i32) -> skia::Image {
    let (iw, ih) = (image.width(), image.height());
    if iw <= 0 || ih <= 0 || target_w <= 0 || target_h <= 0 {
        return image;
    }
    let size = cover_size((iw, ih), (target_w, target_h));
    if size == (iw, ih) {
        return image;
    }
    resample_to(&image, size).unwrap_or(image)
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

pub fn view_background(
    state: &BackgroundViewState,
    _view: &View<BackgroundViewState>,
) -> LayerTree {
    let image = state.image.clone();
    let fallback_color = state.fallback_color;

    let draw_container = move |canvas: &skia::Canvas, w: f32, h: f32| {
        let bounds = skia::Rect::from_wh(w, h);
        match image.as_ref() {
            Some(image) => {
                // Scaled to cover the layer, at a whole-pixel offset: see
                // `otto_kit::wallpaper` for why a half pixel is the worst case.
                let target = (w.round() as i32, h.round() as i32);
                let placement = cover((image.width(), image.height()), target);
                otto_kit::wallpaper::paint(canvas, image, placement, bounds);
            }
            None => paint_fallback_gradient(canvas, fallback_color, bounds),
        }
        bounds
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

/// The gradient shown when no wallpaper is set: `color` at the bottom, 30%
/// of the way to white at the top.
fn paint_fallback_gradient(canvas: &skia::Canvas, color: skia::Color4f, bounds: skia::Rect) {
    const LIGHTER: f32 = 0.3;
    let top = skia::Color4f::new(
        color.r + (1.0 - color.r) * LIGHTER,
        color.g + (1.0 - color.g) * LIGHTER,
        color.b + (1.0 - color.b) * LIGHTER,
        color.a,
    );
    let colors = [color.to_color(), top.to_color()];
    let mut paint = skia::Paint::default();
    if let Some(shader) = skia::gradient_shader::linear(
        ((0.0, bounds.bottom), (0.0, bounds.top)),
        skia::gradient_shader::GradientShaderColors::Colors(&colors),
        Some(&[0.0, 1.0][..]),
        skia::TileMode::Clamp,
        None,
        None,
    ) {
        paint.set_shader(shader);
    }
    canvas.draw_rect(bounds, &paint);
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
        let scaled = resample_to_cover(solid(7680, 4320), 2880, 1920);
        assert!(scaled.width() >= 2880 && scaled.height() >= 1920);
        // Cover is driven by the taller ratio here, so height lands on target.
        assert_eq!(scaled.height(), 1920);
    }

    /// The #361 case: a 16:9 photo on a 21:9 screen. Reduced to the cover
    /// size, it is drawn 1:1 at a whole-pixel offset, so nearest sampling
    /// reads exactly one image pixel per screen pixel however the repaint is
    /// cropped.
    #[test]
    fn a_16_9_wallpaper_on_an_ultrawide_is_drawn_pixel_exact() {
        let scaled = resample_to_cover(solid(3840, 2160), 3440, 1440);
        assert_eq!((scaled.width(), scaled.height()), (3440, 1935));
        let placement = cover((scaled.width(), scaled.height()), (3440, 1440));
        assert!(placement.is_pixel_exact(), "{placement:?}");
    }

    /// A wallpaper smaller than the screen is enlarged to the cover size too,
    /// so the draw does not scale it per frame.
    #[test]
    fn a_smaller_wallpaper_is_enlarged_to_the_cover_size() {
        let scaled = resample_to_cover(solid(2560, 1600), 2880, 1920);
        assert_eq!((scaled.width(), scaled.height()), (3072, 1920));
        assert!(cover((3072, 1920), (2880, 1920)).is_pixel_exact());
    }

    /// One already at the cover size is not touched.
    #[test]
    fn a_wallpaper_at_the_cover_size_is_left_alone() {
        let image = solid(2880, 1920);
        let kept = resample_to_cover(image.clone(), 2880, 1920);
        assert_eq!(kept.unique_id(), image.unique_id());
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
