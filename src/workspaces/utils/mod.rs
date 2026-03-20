pub mod context_menu_view;
pub use context_menu_view::ContextMenuView;

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

    /// Try fuzzy matching against available system font families.
    /// Attempts case-insensitive exact match first, then prefix matching
    /// (preferring shorter names as they're closer to the base family).
    fn fuzzy_match_font(
        &self,
        family: &str,
        style: layers::skia::FontStyle,
        size: f32,
    ) -> Option<layers::skia::Font> {
        let family_lower = family.to_lowercase();
        let mut best_prefix_match: Option<String> = None;

        for name in self.font_mgr.family_names() {
            let name_lower = name.to_lowercase();

            if name_lower == family_lower {
                tracing::info!("Font '{}' matched (case-insensitive) to '{}'", family, name);
                return self.make_font(&name, style, size);
            }

            if name_lower.starts_with(&family_lower)
                && best_prefix_match
                    .as_ref()
                    .is_none_or(|prev| name.len() < prev.len())
            {
                best_prefix_match = Some(name);
            }
        }

        if let Some(ref matched_name) = best_prefix_match {
            tracing::info!("Font '{}' fuzzy-matched to '{}'", family, matched_name);
            return self.make_font(matched_name, style, size);
        }

        None
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

        // Try fuzzy matching (case-insensitive, prefix)
        if let Some(font) = self.fuzzy_match_font(family.as_ref(), style, size) {
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

pub fn configure_surface_layer(
    layer: &Layer,
    wvs: &WindowViewSurface,
    gravity: crate::surface_style::ContentsGravity,
    client_owns_size: bool,
) {
    use crate::surface_style::ContentsGravity;

    // Position calculation: phy_dst is the buffer viewport offset, log_offset is from tree traversal
    let pos_x = wvs.phy_dst_x + wvs.log_offset_x;
    let pos_y = wvs.phy_dst_y + wvs.log_offset_y;

    layer.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });

    // Skip size/position override when client owns the bounds.
    // The compositor initializes from buffer on first commit (before client_owns_size is set).
    if !client_owns_size {
        layer.set_size(
            Size {
                width: taffy::Dimension::Length(wvs.phy_dst_w),
                height: taffy::Dimension::Length(wvs.phy_dst_h),
            },
            None,
        );

        let anchor_point = layer.anchor_point();
        let adjusted_pos = Point {
            x: pos_x + (wvs.phy_dst_w * anchor_point.x),
            y: pos_y + (wvs.phy_dst_h * anchor_point.y),
        };
        layer.set_position(adjusted_pos, None);
    } else {
        tracing::debug!(
            "configure_surface_layer: client owns bounds, skipping set_size/set_position (gravity={:?}, buf={}x{})",
            gravity, wvs.phy_dst_w, wvs.phy_dst_h
        );
    }

    layer.set_pointer_events(false);
    layer.set_picture_cached(true);

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

        // Use live w/h for all gravity modes so the draw scales correctly during animations.
        let (scale_x, scale_y, tx, ty) = match gravity {
            ContentsGravity::Resize => (w / src_w, h / src_h, 0.0f32, 0.0f32),
            ContentsGravity::ResizeAspect => {
                let s = (w / src_w).min(h / src_h);
                let tx = (w - src_w * s) / 2.0;
                let ty = (h - src_h * s) / 2.0;
                (s, s, tx, ty)
            }
            ContentsGravity::ResizeAspectFill => {
                let s = (w / src_w).max(h / src_h);
                let tx = (w - src_w * s) / 2.0;
                let ty = (h - src_h * s) / 2.0;
                (s, s, tx, ty)
            }
            ContentsGravity::Center => {
                let tx = (w - src_w) / 2.0;
                let ty = (h - src_h) / 2.0;
                (1.0f32, 1.0f32, tx, ty)
            }
            ContentsGravity::TopLeft => (1.0f32, 1.0f32, 0.0f32, 0.0f32),
        };

        let mut matrix = layers::skia::Matrix::new_identity();
        match draw_wvs.transform {
            Transform::Normal => {
                matrix.pre_translate((
                    -draw_wvs.phy_src_x + tx / scale_x,
                    -draw_wvs.phy_src_y + ty / scale_y,
                ));
                matrix.pre_scale((scale_x, scale_y), None);
            }
            Transform::Flipped180 => {
                matrix.pre_translate((
                    draw_wvs.phy_src_x + tx / scale_x,
                    draw_wvs.phy_src_y + ty / scale_y,
                ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_cache() -> FontCache {
        let font_mgr = layers::skia::FontMgr::new();
        let type_face_font_provider = layers::skia::textlayout::TypefaceFontProvider::new();
        let mut font_collection = layers::skia::textlayout::FontCollection::new();
        font_collection.set_asset_font_manager(Some(type_face_font_provider.clone().into()));
        font_collection.set_dynamic_font_manager(font_mgr.clone());
        FontCache {
            font_collection,
            font_mgr,
            type_face_font_provider: RefCell::new(type_face_font_provider),
        }
    }

    // These tests require fonts-dejavu-core (Ubuntu/Debian) or ttf-dejavu (Arch)
    // to be installed, providing "DejaVu Sans", "DejaVu Sans Condensed", etc.
    // Run manually with: cargo test --lib -p otto -- workspaces::utils::tests --ignored

    #[test]
    #[ignore]
    fn exact_match_works() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        assert!(
            cache.make_font("DejaVu Sans", style, 12.0).is_some(),
            "exact match for 'DejaVu Sans' should succeed"
        );
    }

    #[test]
    #[ignore]
    fn case_insensitive_match() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        // Whether Skia's own match_family_style is case-insensitive is platform-dependent.
        // Our fuzzy_match_font should always handle it regardless.
        let font = cache.fuzzy_match_font("dejavu sans", style, 12.0);
        assert!(
            font.is_some(),
            "case-insensitive match for 'dejavu sans' should succeed"
        );
    }

    #[test]
    #[ignore]
    fn prefix_match_picks_shortest() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();

        // "DejaVu" should prefix-match to "DejaVu Sans" (shortest family starting with "DejaVu")
        // rather than "DejaVu Sans Mono" or "DejaVu Sans Condensed"
        let font = cache
            .fuzzy_match_font("DejaVu", style, 12.0)
            .expect("prefix match for 'DejaVu' should find a font");
        let family_name = font.typeface().family_name();
        assert_eq!(
            family_name,
            "DejaVu Sans",
            "expected shortest DejaVu family ('DejaVu Sans'), got '{}'",
            family_name
        );
    }

    #[test]
    #[ignore]
    fn fallback_with_fuzzy_returns_font() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        // make_font_with_fallback should use fuzzy matching before falling to generic fallbacks
        let font = cache.make_font_with_fallback("dejavu sans", style, 12.0);
        let family_name = font.typeface().family_name();
        assert!(
            family_name.starts_with("DejaVu"),
            "expected DejaVu family, got '{}'",
            family_name
        );
    }

    #[test]
    #[ignore]
    fn nonexistent_font_falls_back() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        // A completely nonexistent font should still return something
        let _font = cache.make_font_with_fallback("ZzzNonExistentFont999", style, 12.0);
    }
}
