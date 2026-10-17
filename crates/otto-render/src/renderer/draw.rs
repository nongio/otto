//! Skia drawing shared by the renderer frames.
//!
//! The GL and Vulkan frames draw through the same Skia calls; only the
//! surface they draw into differs. These functions hold that one copy of the
//! drawing logic.

// Rust guideline compliant 2026-02-21

use layers::skia;
use smithay::{
    backend::renderer::Color32F,
    utils::{Buffer, Physical, Rectangle, Transform},
};

/// Clips `rect` (relative to `dst`) to `dst` and returns it in canvas space.
fn damage_clip(
    dst: Rectangle<i32, Physical>,
    rect: &Rectangle<i32, Physical>,
) -> Option<skia::Rect> {
    let loc = rect
        .loc
        .constrain(Rectangle::from_extremities((0, 0), dst.size.to_point()));
    let size = rect
        .size
        .clamp((0, 0), (dst.size.to_point() - loc).to_size());
    if size.w <= 0 || size.h <= 0 {
        return None;
    }
    Some(skia::Rect::from_xywh(
        (dst.loc.x + loc.x) as f32,
        (dst.loc.y + loc.y) as f32,
        size.w as f32,
        size.h as f32,
    ))
}

/// Fills `dst` with `color`, touching only the `damage` rects.
///
/// Damage is relative to `dst`. The fill replaces the pixels (`Src` blend),
/// alpha included.
pub fn draw_solid(
    surface: &mut skia::Surface,
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    color: Color32F,
) {
    if damage.is_empty() {
        return;
    }

    let dest_rect = skia::Rect::from_xywh(
        dst.loc.x as f32,
        dst.loc.y as f32,
        dst.size.w as f32,
        dst.size.h as f32,
    );
    let color = skia::Color4f::new(color.r(), color.g(), color.b(), color.a());
    let mut paint = skia::Paint::new(color, None);
    paint.set_blend_mode(skia::BlendMode::Src);

    let canvas = surface.canvas();
    for clip_rect in damage.iter().filter_map(|rect| damage_clip(dst, rect)) {
        canvas.save();
        canvas.clip_rect(clip_rect, None, None);
        canvas.draw_rect(dest_rect, &paint);
        canvas.restore();
    }
}

/// A color filter that makes its input fully opaque, keeping the color.
///
/// Pixel formats without alpha (`Xrgb8888`, `Xbgr8888`, `Xrgb2101010`, ...)
/// carry a padding byte where the alpha would be, and XWayland leaves it at 0
/// in depth-24 pixmaps. A GL texture for such a format has no alpha channel,
/// so sampling it yields alpha 1. The Vulkan renderer wraps the same memory as
/// an alpha format and Skia samples the padding byte, giving `(r, g, b, 0)`:
/// invisible when drawn straight over an opaque clear, but anything that
/// keeps the intermediate (an image-cached layer, an exposé mirror) blends
/// the window away.
///
/// `DstOver` of opaque black, in premultiplied space, keeps the input color
/// and yields alpha 1. Filter the image shader with it ([`opaque_shader`]),
/// so a paint alpha still fades the result; setting it on the paint instead
/// is only right when the paint alpha is 1, because Skia applies the paint
/// alpha before the paint's color filter.
pub fn opaque_color_filter() -> skia::ColorFilter {
    skia::color_filters::blend(skia::Color::BLACK, skia::BlendMode::DstOver)
        .expect("DstOver of opaque black is never a no-op")
}

/// Wraps an image shader of a format without alpha so it draws opaque.
///
/// See [`opaque_color_filter`] for why the padding byte cannot be trusted.
pub fn opaque_shader(shader: skia::Shader) -> skia::Shader {
    shader.with_color_filter(opaque_color_filter())
}

/// Draws the `src` region of `image` into `dst`, touching only the `damage` rects.
///
/// Damage is relative to `dst`. The image is blended over what is there
/// (`SrcOver`) at `alpha`. With `padding_alpha` the image's alpha byte is
/// format padding and the image draws opaque whatever that byte holds.
///
/// # Panics
///
/// Panics on a `src_transform` other than `Normal` or `Flipped180`; no
/// caller produces one.
pub fn render_texture(
    surface: &mut skia::Surface,
    image: &skia::Image,
    src: Rectangle<f64, Buffer>,
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    src_transform: Transform,
    alpha: f32,
    padding_alpha: bool,
) {
    if damage.is_empty() {
        return;
    }

    let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, alpha), None);
    paint.set_blend_mode(skia::BlendMode::SrcOver);

    let scale_x = dst.size.w as f32 / src.size.w as f32;
    let scale_y = dst.size.h as f32 / src.size.h as f32;

    let mut matrix = skia::Matrix::new_identity();
    match src_transform {
        Transform::Normal => {
            matrix.pre_scale((scale_x, scale_y), None);
            matrix.pre_translate((
                dst.loc.x as f32 / scale_x - (src.loc.x as f32),
                dst.loc.y as f32 / scale_y - (src.loc.y as f32),
            ));
        }
        Transform::Flipped180 => {
            matrix.pre_scale((scale_x, -scale_y), None);
            matrix.pre_translate((
                dst.loc.x as f32 / scale_x - src.loc.x as f32,
                -dst.loc.y as f32 / scale_y + src.loc.y as f32,
            ));
        }
        _ => panic!("unhandled transform {src_transform:?}"),
    }

    let shader = image.to_shader(
        (skia::TileMode::Repeat, skia::TileMode::Repeat),
        skia::SamplingOptions::default(),
        &matrix,
    );
    paint.set_shader(match shader {
        Some(shader) if padding_alpha => Some(opaque_shader(shader)),
        shader => shader,
    });

    let draw_rect = skia::Rect::from_xywh(
        dst.loc.x as f32,
        dst.loc.y as f32,
        dst.size.w as f32,
        dst.size.h as f32,
    );

    let canvas = surface.canvas();
    for clip_rect in damage.iter().filter_map(|rect| damage_clip(dst, rect)) {
        canvas.save();
        canvas.clip_rect(clip_rect, None, None);
        canvas.draw_rect(draw_rect, &paint);
        canvas.restore();
    }
}
