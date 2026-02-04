use std::cell::RefCell;

use layers::prelude::{taffy, Layer};
use layers::types::{Point, Size};
use smithay::utils::Transform;

use super::WindowViewSurface;

#[allow(unused)]
pub struct FontCache {
    pub font_collection: layers::skia::textlayout::FontCollection,
    pub font_mgr: layers::skia::FontMgr,
    pub type_face_font_provider: RefCell<layers::skia::textlayout::TypefaceFontProvider>,
}

impl FontCache {
    /// Create a Font with subpixel rendering and antialiasing enabled
    pub fn make_font(
        &self,
        family: impl AsRef<str>,
        style: layers::skia::FontStyle,
        size: f32,
    ) -> Option<layers::skia::Font> {
        let typeface = self.font_mgr.match_family_style(family.as_ref(), style)?;
        let mut font = layers::skia::Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(layers::skia::font::Edging::SubpixelAntiAlias);
        Some(font)
    }

    /// Create a Font with fallback to system default if family not found
    pub fn make_font_with_fallback(
        &self,
        family: impl AsRef<str>,
        style: layers::skia::FontStyle,
        size: f32,
    ) -> layers::skia::Font {
        if let Some(font) = self.make_font(&family, style, size) {
            return font;
        }

        // Try common fallback fonts
        for fallback in ["sans-serif", "DejaVu Sans", "Liberation Sans", "Arial"] {
            if let Some(font) = self.make_font(fallback, style, size) {
                tracing::warn!(
                    "Font '{}' not found, using fallback: '{}'",
                    family.as_ref(),
                    fallback
                );
                return font;
            }
        }

        // Last resort: use default typeface from font manager
        tracing::error!(
            "Font '{}' and all fallbacks failed, using default",
            family.as_ref()
        );
        let typeface = self
            .font_mgr
            .legacy_make_typeface(None, style)
            .expect("Failed to create default typeface");
        let mut font = layers::skia::Font::from_typeface(typeface, size);
        font.set_subpixel(true);
        font.set_edging(layers::skia::font::Edging::SubpixelAntiAlias);
        font
    }
}

thread_local! {
    pub static FONT_CACHE: FontCache = {
        let font_mgr = layers::skia::FontMgr::new();
        let type_face_font_provider = layers::skia::textlayout::TypefaceFontProvider::new();
        let mut font_collection = layers::skia::textlayout::FontCollection::new();
        font_collection.set_asset_font_manager(Some(type_face_font_provider.clone().into()));
        font_collection.set_dynamic_font_manager(font_mgr.clone());
        FontCache { font_collection, font_mgr, type_face_font_provider: RefCell::new(type_face_font_provider) }
    };
}

#[allow(clippy::too_many_arguments)]
pub fn draw_balloon_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    corner_radius: f32,
    arrow_width: f32,
    arrow_height: f32,
    arrow_position: f32, // Position of the arrow along the bottom edge (0.0 to 1.0)
    arrow_corner_radius: f32,
) -> layers::skia::Path {
    let mut path = layers::skia::Path::new();

    // Calculate the arrow tip position
    let arrow_tip_x = x + arrow_position * width;
    let arrow_base_left_x = arrow_tip_x - arrow_width / 2.0;
    let arrow_base_right_x = arrow_tip_x + arrow_width / 2.0;

    // Move to the starting point (top-left corner)
    path.move_to((x + corner_radius, y));

    // Top edge
    path.line_to((x + width - corner_radius, y));
    path.arc_to_tangent(
        (x + width, y),
        (x + width, y + corner_radius),
        corner_radius,
    );

    // Right edge
    path.line_to((x + width, y + height - corner_radius - arrow_height));
    path.arc_to_tangent(
        (x + width, y + height - arrow_height),
        (x + width - corner_radius, y + height - arrow_height),
        corner_radius,
    );

    // Arrow with rounded corners
    path.line_to((
        arrow_base_right_x, //- arrow_corner_radius,
        y + height - arrow_height,
    ));
    path.arc_to_tangent(
        (arrow_base_right_x, y + height - arrow_height),
        (arrow_tip_x, y + height),
        arrow_corner_radius,
    );
    path.arc_to_tangent(
        (arrow_tip_x, y + height),
        (arrow_base_left_x, y + height - arrow_height),
        arrow_corner_radius,
    );
    path.arc_to_tangent(
        (arrow_base_left_x, y + height - arrow_height),
        (x + corner_radius, y + height - arrow_height),
        arrow_corner_radius,
    );

    // Bottom edge
    path.line_to((x + corner_radius, y + height - arrow_height));
    path.arc_to_tangent(
        (x, y + height - arrow_height),
        (x, y + height - corner_radius - arrow_height),
        corner_radius,
    );

    // Left edge
    path.line_to((x, y + corner_radius));
    path.arc_to_tangent((x, y), (x + corner_radius, y), corner_radius);

    // Close the path
    path.close();
    path
}

/// Configure a surface layer with all rendering properties and draw callback
///
/// This sets up:
/// - Position (relative to parent)
/// - Size
/// - Layout style (absolute positioning)
/// - Pointer events (disabled)
/// - Picture caching (disabled)
/// - Draw content callback with texture rendering
pub fn configure_surface_layer(layer: &Layer, wvs: &WindowViewSurface) {
    // Position calculation: phy_dst is the buffer viewport offset, log_offset is from tree traversal
    // log_offset is now relative to parent (or absolute for root surfaces)
    let pos_x = wvs.phy_dst_x + wvs.log_offset_x;
    let pos_y = wvs.phy_dst_y + wvs.log_offset_y;

    // Apply all layer properties
    layer.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });

    layer.set_position(Point { x: pos_x, y: pos_y }, None);
    layer.set_size(
        Size {
            width: taffy::Dimension::Length(wvs.phy_dst_w),
            height: taffy::Dimension::Length(wvs.phy_dst_h),
        },
        None,
    );

    layer.set_pointer_events(false);
    layer.set_picture_cached(true); // Enable picture caching for proper opacity inheritance

    // Set the draw content callback for texture rendering
    let draw_wvs = wvs.clone();
    layer.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        if w == 0.0 || h == 0.0 {
            return layers::skia::Rect::default();
        }
        let tex = crate::textures_storage::get(&draw_wvs.id);
        if tex.is_none() {
            return layers::skia::Rect::default();
        }
        let tex = tex.unwrap();
        let mut damage = layers::skia::Rect::default();
        if let Some(tex_damage) = tex.damage {
            tex_damage.iter().for_each(|bd| {
                let r = layers::skia::Rect::from_xywh(
                    bd.loc.x as f32,
                    bd.loc.y as f32,
                    bd.size.w as f32,
                    bd.size.h as f32,
                );
                damage.join(r);
            });
        }

        let src_h = (draw_wvs.phy_src_h - draw_wvs.phy_src_y).max(1.0);
        let src_w = (draw_wvs.phy_src_w - draw_wvs.phy_src_x).max(1.0);
        let scale_y = draw_wvs.phy_dst_h / src_h;
        let scale_x = draw_wvs.phy_dst_w / src_w;
        let mut matrix = layers::skia::Matrix::new_identity();
        match draw_wvs.transform {
            Transform::Normal => {
                matrix.pre_translate((-draw_wvs.phy_src_x, -draw_wvs.phy_src_y));
                matrix.pre_scale((scale_x, scale_y), None);
            }
            Transform::Flipped180 => {
                matrix.pre_translate((draw_wvs.phy_src_x, draw_wvs.phy_src_y));
                matrix.pre_scale((scale_x, -scale_y), None);
            }
            Transform::_90 => {}
            Transform::_180 => {}
            Transform::_270 => {}
            Transform::Flipped => {}
            Transform::Flipped90 => {}
            Transform::Flipped270 => {}
        }

        let sampling =
            layers::skia::SamplingOptions::from(layers::skia::CubicResampler::catmull_rom());
        let mut paint =
            layers::skia::Paint::new(layers::skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_shader(tex.image.to_shader(
            (layers::skia::TileMode::Clamp, layers::skia::TileMode::Clamp),
            sampling,
            &matrix,
        ));

        let rect = layers::skia::Rect::from_xywh(0.0, 0.0, w, h);
        canvas.draw_rect(rect, &paint);
        damage
    });
}
