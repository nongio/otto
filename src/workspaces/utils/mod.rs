pub mod context_menu_view;
pub use context_menu_view::ContextMenuView;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use layers::prelude::{taffy, Layer};
use layers::types::{Point, Size};
use smithay::utils::Transform;

use super::WindowViewSurface;

/// `OTTO_ADAPTIVE_SAMPLING=0` forces bicubic for every per-window draw (the
/// pre-change baseline). Anything else (unset, `1`, `true`, ...) keeps the
/// adaptive path that picks nearest/linear/bicubic from the matrix. Read once
/// at startup so the per-frame cost is a single atomic load.
fn adaptive_sampling_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("OTTO_ADAPTIVE_SAMPLING").as_deref(),
            Ok("0") | Ok("false") | Ok("off")
        )
    })
}

#[allow(unused)]
pub struct FontCache {
    pub font_collection: layers::skia::textlayout::FontCollection,
    pub font_mgr: layers::skia::FontMgr,
    pub type_face_font_provider: RefCell<layers::skia::textlayout::TypefaceFontProvider>,
    /// Maps requested family name (lowercased) → resolved system family name from fuzzy matching.
    /// Avoids re-scanning all system families on every call to `make_font_with_fallback`.
    family_name_cache: RefCell<HashMap<String, String>>,
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
                tracing::debug!("Font '{}' matched (case-insensitive) to '{}'", family, name);
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
            tracing::debug!("Font '{}' fuzzy-matched to '{}'", family, matched_name);
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

        // Try fuzzy matching (case-insensitive, prefix), using the cache to avoid
        // re-scanning all system families on every call from the render path.
        let family_lower = family.as_ref().to_lowercase();
        let cached_name = self.family_name_cache.borrow().get(&family_lower).cloned();
        if let Some(ref resolved) = cached_name {
            if let Some(font) = self.make_font(resolved, style, size) {
                return font;
            }
        } else if let Some(font) = self.fuzzy_match_font(family.as_ref(), style, size) {
            // Store the resolved name so future calls skip the O(N) scan.
            self.family_name_cache
                .borrow_mut()
                .insert(family_lower, font.typeface().family_name());
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
        FontCache { font_collection, font_mgr, type_face_font_provider: RefCell::new(type_face_font_provider), family_name_cache: RefCell::new(HashMap::new()) }
    };
}

/// Which edge of a balloon its arrow sticks out of, i.e. which way it points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalloonArrow {
    Bottom,
    Left,
    Right,
}

/// A rounded rect with an arrow on one edge, filling `width` × `height` at
/// `(x, y)` — arrow included.
///
/// Only the bottom-arrow shape is built by hand; the side variants are that
/// same path rotated a quarter turn, which keeps the rounded arrow tip and the
/// corner radii identical whichever way the balloon points.
#[allow(clippy::too_many_arguments)]
pub fn draw_balloon_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    corner_radius: f32,
    arrow_width: f32,
    arrow_height: f32,
    arrow_position: f32, // Position of the arrow along the arrow edge (0.0 to 1.0)
    arrow_corner_radius: f32,
    arrow: BalloonArrow,
) -> layers::skia::Path {
    if arrow != BalloonArrow::Bottom {
        // Build it pointing down in a frame with the axes swapped, then turn it.
        let base = draw_balloon_rect(
            0.0,
            0.0,
            height,
            width,
            corner_radius,
            arrow_width,
            arrow_height,
            arrow_position,
            arrow_corner_radius,
            BalloonArrow::Bottom,
        );
        let (degrees, offset) = match arrow {
            // Clockwise: down becomes left, and the path lands at negative x.
            BalloonArrow::Left => (90.0, (x + width, y)),
            // Counter-clockwise: down becomes right, negative y.
            _ => (-90.0, (x, y + height)),
        };
        let rotated = base.with_transform(&layers::skia::Matrix::rotate_deg(degrees));
        return rotated.with_transform(&layers::skia::Matrix::translate(offset));
    }

    let mut builder = layers::skia::PathBuilder::new();

    // Calculate the arrow tip position
    let arrow_tip_x = x + arrow_position * width;
    let arrow_base_left_x = arrow_tip_x - arrow_width / 2.0;
    let arrow_base_right_x = arrow_tip_x + arrow_width / 2.0;

    // Move to the starting point (top-left corner)
    builder.move_to((x + corner_radius, y));

    // Top edge
    builder.line_to((x + width - corner_radius, y));
    builder.arc_to_tangent(
        (x + width, y),
        (x + width, y + corner_radius),
        corner_radius,
    );

    // Right edge
    builder.line_to((x + width, y + height - corner_radius - arrow_height));
    builder.arc_to_tangent(
        (x + width, y + height - arrow_height),
        (x + width - corner_radius, y + height - arrow_height),
        corner_radius,
    );

    // Arrow with rounded corners
    builder.line_to((
        arrow_base_right_x, //- arrow_corner_radius,
        y + height - arrow_height,
    ));
    builder.arc_to_tangent(
        (arrow_base_right_x, y + height - arrow_height),
        (arrow_tip_x, y + height),
        arrow_corner_radius,
    );
    builder.arc_to_tangent(
        (arrow_tip_x, y + height),
        (arrow_base_left_x, y + height - arrow_height),
        arrow_corner_radius,
    );
    builder.arc_to_tangent(
        (arrow_base_left_x, y + height - arrow_height),
        (x + corner_radius, y + height - arrow_height),
        arrow_corner_radius,
    );

    // Bottom edge
    builder.line_to((x + corner_radius, y + height - arrow_height));
    builder.arc_to_tangent(
        (x, y + height - arrow_height),
        (x, y + height - corner_radius - arrow_height),
        corner_radius,
    );

    // Left edge
    builder.line_to((x, y + corner_radius));
    builder.arc_to_tangent((x, y), (x + corner_radius, y), corner_radius);

    // Close the path
    builder.close();
    builder.detach()
}

/// Whether a client surface really covers its whole layer with opaque pixels.
///
/// `configure_surface_layer` marks every surface layer `content_opaque`, which
/// is what lets occlusion culling treat a window as an occluder. For a
/// wlr-layer-shell surface that claim is often a lie: a fullscreen overlay
/// (the launcher) is mostly transparent, and a backend that renders the output
/// as ONE tree — winit — then culls the wallpaper and every window under it and
/// paints the screen black. Smithay already tracks the truth in
/// `RendererSurfaceState::opaque_regions`, computed from the buffer's alpha
/// channel and the client's `wl_surface.set_opaque_region`; this reports
/// whether one of those regions covers the surface whole.
///
/// Conservative on purpose: a surface whose opacity is only expressible as a
/// union of rects reads as non-opaque, which costs some culling but never
/// erases content.
pub fn surface_is_fully_opaque(states: &smithay::wayland::compositor::SurfaceData) -> bool {
    let Some(data) = states
        .data_map
        .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>()
    else {
        return false;
    };
    let data = data.lock().unwrap();
    let Some(view) = data.view() else {
        return false;
    };
    let full_rect = smithay::utils::Rectangle::from_size(view.dst);
    data.opaque_regions()
        .map(|regions| regions.iter().any(|r| r.contains_rect(full_rect)))
        .unwrap_or(false)
}

/// Round a physical-pixel position onto the whole-pixel grid.
///
/// A window's position is chosen in logical integers and multiplied by the
/// output scale, so on a fractional scale it lands mid-pixel (logical 101 x
/// 1.5 = 151.5). A container layer left there offsets its whole subtree by
/// half a pixel, and every texture under it is resampled when that subtree is
/// composited — including a client buffer that matches the output exactly and
/// was point-sampled 1:1 by the draw closure, because the closure records into
/// a picture whose transform is applied later. Half a pixel of placement
/// accuracy does not pay for that.
///
/// Snap the ENDPOINTS of a move, not the frames of one: lay-rs interpolates
/// between two snapped positions, and rounding every interpolated frame would
/// quantise the animation.
pub fn snap_position_px(x: f64, y: f64) -> layers::types::Point {
    layers::types::Point {
        x: x.round() as f32,
        y: y.round() as f32,
    }
}

/// Round a physical-pixel EXTENT so that both of a box's edges land on the
/// pixel grid.
///
/// [`snap_position_px`] puts an origin on the grid, but a size reaches the
/// layer the same way a position does — a logical integer multiplied by the
/// output scale — so the FAR edge is left fractional. The server-side
/// titlebar is [`WindowElement::DECORATION_HEIGHT`] = 34 logical points, and
/// 34 x 1.75 = 59.5: its bottom hairline paints across three physical rows
/// with no fully covered one, and the client content below it starts on a
/// half pixel, which resamples the whole surface subtree.
///
/// Snap the far EDGE rather than the extent on its own. Rounding an origin
/// and an extent independently can move the far edge a whole pixel when the
/// two round in opposite directions (origin 10.5 -> 11, extent 9.5 -> 10
/// puts the edge at 21 instead of 20).
pub fn snap_extent_px(origin: f32, extent: f32) -> f32 {
    (origin + extent).round() - origin.round()
}

/// Which filter to sample a surface's texture with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceFilter {
    Nearest,
    Linear,
    Cubic,
}

impl SurfaceFilter {
    fn sampling_options(self) -> layers::skia::SamplingOptions {
        match self {
            // `default()` is nearest + no mipmaps.
            Self::Nearest => layers::skia::SamplingOptions::default(),
            Self::Linear => layers::skia::SamplingOptions::new(
                layers::skia::FilterMode::Linear,
                layers::skia::MipmapMode::None,
            ),
            Self::Cubic => {
                layers::skia::SamplingOptions::from(layers::skia::CubicResampler::catmull_rom())
            }
        }
    }
}

/// Pick the cheapest filter the mapping allows.
///
/// Bicubic is ~12-16× the fragment-shader cost of nearest, and the dominant
/// desktop case — a static window whose buffer matches the output — maps every
/// output pixel to one source pixel and gets identical results from any filter.
///
/// `scale` and `translation` describe the texture's mapping onto the LAYER.
/// That only tells us what reaches the framebuffer if the layer itself starts
/// on a whole physical pixel, which is what `pixel_grid_aligned` asserts:
/// without it, a 1:1 texture on a fractionally positioned layer would be point
/// sampled half a pixel off and come out with doubled and dropped pixel rows.
/// Both cheap branches therefore require it.
fn surface_filter(
    is_normal_transform: bool,
    scale: (f32, f32),
    translation: (f32, f32),
    pixel_grid_aligned: bool,
) -> SurfaceFilter {
    let (scale_x, scale_y) = scale;
    let (tx, ty) = translation;

    if !is_normal_transform || !pixel_grid_aligned {
        return SurfaceFilter::Cubic;
    }

    let is_identity_scale = (scale_x - 1.0).abs() < 1e-4 && (scale_y - 1.0).abs() < 1e-4;
    let is_pixel_aligned = (tx - tx.round()).abs() < 1e-4 && (ty - ty.round()).abs() < 1e-4;

    if is_identity_scale && is_pixel_aligned {
        SurfaceFilter::Nearest
    } else if (scale_x - 1.0).abs() < 0.05 && (scale_y - 1.0).abs() < 0.05 {
        SurfaceFilter::Linear
    } else {
        SurfaceFilter::Cubic
    }
}

pub fn configure_surface_layer(
    layer: &Layer,
    wvs: &WindowViewSurface,
    gravity: crate::surface_style::ContentsGravity,
    client_owns_size: bool,
    shared_gravity: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
) {
    use crate::surface_style::ContentsGravity;

    // Every setter below schedules a lay-rs change unconditionally — lay-rs
    // does not compare the incoming value, and `set_draw_content` always
    // raises NEEDS_PAINT. Re-running this for a surface that did not change
    // therefore invents damage, and the sync above calls it for every surface
    // of a window (plus its popups) on every commit of any one of them. Reduce
    // the whole configuration to one key and skip the body when it matches
    // what the layer already holds. `WindowViewSurface`'s `Hash` covers the
    // commit counter and texture id, so real content changes still fall
    // through; the node id is in the key too, since a surface whose layer was
    // recreated needs configuring even with identical geometry.
    // See `crate::surface_config_cache`.
    {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        wvs.hash(&mut hasher);
        (gravity as u8).hash(&mut hasher);
        client_owns_size.hash(&mut hasher);
        shared_gravity.is_some().hash(&mut hasher);
        usize::from(layer.id()).hash(&mut hasher);
        if !crate::surface_config_cache::record_if_changed(&wvs.id, hasher.finish()) {
            return;
        }
    }

    // Position calculation: phy_dst is the buffer viewport offset, log_offset is from tree traversal
    //
    // Rounded to whole physical pixels. Both terms come from logical values
    // multiplied by the output scale, so on a fractional scale they land
    // mid-pixel (logical 101 x 1.65 = 166.65) — and a client that went to the
    // trouble of painting an exactly 1:1 buffer for this output then has that
    // buffer resampled across the pixel grid for the sake of two thirds of a
    // pixel of placement accuracy. Snapping costs at most half a pixel of
    // position and buys back the identity mapping, which is both the crisp
    // result and the cheap one (see the sampling gate in the draw closure).
    let pos_x = (wvs.phy_dst_x + wvs.log_offset_x).round();
    let pos_y = (wvs.phy_dst_y + wvs.log_offset_y).round();

    layer.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });

    // Skip size/position override when client owns the bounds.
    // The compositor initializes from buffer on first commit (before client_owns_size is set).
    if !client_owns_size {
        // The size is snapped for the same reason the position above is, and
        // against the UNROUNDED origin so the surface's far edge lands on the
        // grid too rather than inheriting the origin's rounding error.
        let w = snap_extent_px(wvs.phy_dst_x + wvs.log_offset_x, wvs.phy_dst_w);
        let h = snap_extent_px(wvs.phy_dst_y + wvs.log_offset_y, wvs.phy_dst_h);
        layer.set_size(
            Size {
                width: taffy::Dimension::Length(w),
                height: taffy::Dimension::Length(h),
            },
            None,
        );

        let anchor_point = layer.anchor_point();
        let adjusted_pos = Point {
            x: pos_x + (w * anchor_point.x),
            y: pos_y + (h * anchor_point.y),
        };
        layer.set_position(adjusted_pos, None);
    }

    layer.set_pointer_events(false);
    // Picture caching keeps opacity/transform animations cheap — the cached
    // bitmap is composited without re-rasterising the draw closure.
    // Re-installing set_draw_content below on every commit does NOT clear
    // the cache (lay-rs only swaps the closure); the closure's returned
    // damage rect is the source of truth for partial repaint.
    layer.set_picture_cached(true);
    layer.set_content_opaque(true);

    // Does this surface sit on the physical pixel grid?
    //
    // The sampling gate in the draw closure reasons in LAYER space, but what
    // it is really choosing is how the texture lands on the FRAMEBUFFER, and
    // the two only agree when the layer's origin is a whole physical pixel:
    // a texture that maps 1:1 onto its layer still lands half a pixel off on
    // screen if the layer does, and point-sampling that is what doubles and
    // drops rows of source pixels. The rounding above buys the guarantee for
    // every surface we position; `client_owns_size` surfaces take their
    // position from the client, so we cannot make the claim for them.
    let pixel_grid_aligned = !client_owns_size
        && (pos_x - pos_x.round()).abs() < 1e-3
        && (pos_y - pos_y.round()).abs() < 1e-3;

    let draw_wvs = wvs.clone();
    let draw_shared_gravity = shared_gravity.clone();
    layer.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        // Read gravity live from shared atomic (updated when client calls set_contents_gravity)
        let shared_val = draw_shared_gravity
            .as_ref()
            .map(|g| g.load(std::sync::atomic::Ordering::Relaxed));
        let gravity = shared_val.map(ContentsGravity::from_u8).unwrap_or(gravity);

        if w == 0.0 || h == 0.0 {
            return layers::skia::Rect::default();
        }
        let tex = crate::textures_storage::get(&draw_wvs.id);
        if tex.is_none() {
            return layers::skia::Rect::default();
        }
        let tex = tex.unwrap();

        // Use the viewport source dimensions, NOT the raw GPU texture size.
        // Clients like Chrome reuse oversized GPU texture allocations; the
        // viewport crop (phy_src) tells us the actual content region.
        let src_w = draw_wvs.phy_src_w.max(1.0);
        let src_h = draw_wvs.phy_src_h.max(1.0);

        // Buffer pixels are not physical pixels: a client painting at buffer
        // scale 2 on a 1.5x output hands us a 60px buffer for 45 physical px.
        // The non-resizing gravities align the texture rather than stretch it,
        // but they still have to bridge that ratio — otherwise the content is
        // drawn oversized and cropped to the layer.
        let base_x = if draw_wvs.phy_dst_w > 0.0 {
            draw_wvs.phy_dst_w / src_w
        } else {
            1.0
        };
        let base_y = if draw_wvs.phy_dst_h > 0.0 {
            draw_wvs.phy_dst_h / src_h
        } else {
            1.0
        };

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
                let tx = (w - src_w * base_x) / 2.0;
                let ty = (h - src_h * base_y) / 2.0;
                (base_x, base_y, tx, ty)
            }
            ContentsGravity::TopLeft => (base_x, base_y, 0.0f32, 0.0f32),
            ContentsGravity::TopRight => {
                let tx = w - src_w * base_x;
                (base_x, base_y, tx, 0.0f32)
            }
        };

        // Convert buffer-pixel damage to layer-local coords using the same
        // scale + offset that the texture is drawn with.
        let mut damage = layers::skia::Rect::default();
        if let Some(tex_damage) = tex.damage {
            tex_damage.iter().for_each(|bd| {
                let r = layers::skia::Rect::from_xywh(
                    bd.loc.x as f32 * scale_x + tx,
                    bd.loc.y as f32 * scale_y + ty,
                    bd.size.w as f32 * scale_x,
                    bd.size.h as f32 * scale_y,
                );
                damage.join(r);
            });
        }

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

        let sampling = if adaptive_sampling_enabled() {
            surface_filter(
                matches!(draw_wvs.transform, Transform::Normal),
                (scale_x, scale_y),
                (
                    -draw_wvs.phy_src_x + tx / scale_x,
                    -draw_wvs.phy_src_y + ty / scale_y,
                ),
                pixel_grid_aligned,
            )
        } else {
            SurfaceFilter::Cubic
        }
        .sampling_options();

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

    /// The initial placement of a window is chosen in logical integers, so on
    /// a fractional scale it lands mid-pixel and drags its whole subtree off
    /// the grid with it.
    #[test]
    fn a_position_off_the_grid_is_snapped_onto_it() {
        // logical (101, 7) on a 1.5x output = (151.5, 10.5)
        let p = snap_position_px(101.0 * 1.5, 7.0 * 1.5);
        assert_eq!((p.x, p.y), (152.0, 11.0));
    }

    /// An integer scale never leaves the grid, so snapping is a no-op there —
    /// the fix costs nothing on the common case.
    #[test]
    fn a_position_on_the_grid_is_left_alone() {
        let p = snap_position_px(101.0 * 2.0, 7.0 * 2.0);
        assert_eq!((p.x, p.y), (202.0, 14.0));
    }

    /// The regression this pairs with: snapping the origin alone still leaves
    /// the far edge fractional. A 34pt titlebar on a 1.75x output is 59.5px,
    /// which paints its bottom hairline across three physical rows.
    #[test]
    fn an_extent_off_the_grid_is_snapped_onto_it() {
        assert_eq!(snap_extent_px(0.0, 34.0 * 1.75), 60.0);
    }

    /// Snapping against the origin, not in isolation: origin 10.5 and extent
    /// 9.5 round in opposite directions, so an independently rounded extent
    /// would put the far edge at 21 instead of the correct 20.
    #[test]
    fn an_extent_is_snapped_against_its_own_origin() {
        assert_eq!(snap_extent_px(10.5, 9.5), 9.0);
        assert_eq!(10.5_f32.round() + snap_extent_px(10.5, 9.5), 20.0);
    }

    /// Integer scales stay a no-op here too.
    #[test]
    fn an_extent_on_the_grid_is_left_alone() {
        assert_eq!(snap_extent_px(202.0, 34.0 * 2.0), 68.0);
    }

    /// The whole point of the cheap branches: a window whose client painted a
    /// buffer at exactly the output scale is copied, not resampled.
    #[test]
    fn an_exact_buffer_on_the_pixel_grid_is_point_sampled() {
        assert_eq!(
            surface_filter(true, (1.0, 1.0), (0.0, 0.0), true),
            SurfaceFilter::Nearest
        );
    }

    /// ...and the regression that put shimmer on every window of a
    /// fractionally scaled output: the same exact buffer, on a layer that
    /// starts mid-pixel, is NOT a 1:1 copy however identity the layer-space
    /// numbers look. Point sampling it doubles and drops rows of pixels.
    #[test]
    fn an_exact_buffer_off_the_pixel_grid_is_not_point_sampled() {
        assert_eq!(
            surface_filter(true, (1.0, 1.0), (0.0, 0.0), false),
            SurfaceFilter::Cubic
        );
    }

    #[test]
    fn a_shifted_buffer_is_not_point_sampled() {
        assert_eq!(
            surface_filter(true, (1.0, 1.0), (0.5, 0.0), true),
            SurfaceFilter::Linear
        );
    }

    #[test]
    fn a_nearly_unscaled_buffer_is_filtered_linearly() {
        assert_eq!(
            surface_filter(true, (1.02, 1.02), (0.0, 0.0), true),
            SurfaceFilter::Linear
        );
        // ...but only on the grid.
        assert_eq!(
            surface_filter(true, (1.02, 1.02), (0.0, 0.0), false),
            SurfaceFilter::Cubic
        );
    }

    /// A 2x buffer on a 1.65x output, the other half of the fractional-scale
    /// story: a real resample, which has always wanted the bicubic.
    #[test]
    fn a_rescaled_buffer_is_filtered_bicubically() {
        assert_eq!(
            surface_filter(true, (0.825, 0.825), (0.0, 0.0), true),
            SurfaceFilter::Cubic
        );
    }

    #[test]
    fn a_flipped_buffer_is_filtered_bicubically() {
        assert_eq!(
            surface_filter(false, (1.0, 1.0), (0.0, 0.0), true),
            SurfaceFilter::Cubic
        );
    }

    /// Not an assertion: writes the three balloon shapes to /tmp so they can be
    /// eyeballed. Run with `cargo test --lib dump_balloons -- --ignored`.
    #[test]
    #[ignore]
    fn dump_balloons() {
        use layers::skia;
        let mut surface = skia::surfaces::raster_n32_premul((900, 200)).unwrap();
        let canvas = surface.canvas();
        canvas.clear(skia::Color::WHITE);
        let mut paint = skia::Paint::new(skia::Color4f::new(0.2, 0.2, 0.9, 1.0), None);
        paint.set_anti_alias(true);
        for (i, arrow) in [
            BalloonArrow::Bottom,
            BalloonArrow::Left,
            BalloonArrow::Right,
        ]
        .into_iter()
        .enumerate()
        {
            let x = 20.0 + i as f32 * 290.0;
            let path = draw_balloon_rect(x, 40.0, 200.0, 60.0, 5.0, 12.5, 10.0, 0.5, 1.5, arrow);
            canvas.draw_path(&path, &paint);
        }
        let image = surface.image_snapshot();
        let data = image
            .encode(None, skia::EncodedImageFormat::PNG, None)
            .unwrap();
        std::fs::write("/tmp/balloons.png", data.as_bytes()).unwrap();
    }

    /// The balloon must fill exactly the rect it was asked for, arrow included,
    /// whichever edge the arrow sticks out of — the tooltip layer is sized from
    /// those same numbers, so a path that overflows or falls short of them puts
    /// the balloon somewhere other than where the layout put the layer.
    #[test]
    fn balloon_fills_the_requested_rect_on_every_edge() {
        for arrow in [
            BalloonArrow::Bottom,
            BalloonArrow::Left,
            BalloonArrow::Right,
        ] {
            let path = draw_balloon_rect(50.0, 20.0, 200.0, 60.0, 5.0, 12.5, 10.0, 0.5, 1.5, arrow);
            let bounds = path.compute_tight_bounds();
            // The arrow tip is rounded off, so the edge it points at stops one
            // arrow corner radius short of the rect.
            let slack = 1.5;
            assert!(
                (bounds.x() - 50.0).abs() < slack
                    && (bounds.y() - 20.0).abs() < slack
                    && (bounds.width() - 200.0).abs() < slack
                    && (bounds.height() - 60.0).abs() < slack,
                "{arrow:?}: balloon bounds {bounds:?} should be 50,20 200x60"
            );
        }
    }

    /// Where the arrow tip is decides which way the tooltip points: the tip has
    /// to sit on the named edge, halfway along it.
    #[test]
    fn the_arrow_tip_sits_on_its_own_edge() {
        let (x, y, w, h) = (50.0_f32, 20.0_f32, 200.0_f32, 60.0_f32);
        let cases = [
            (BalloonArrow::Bottom, (x + w / 2.0, y + h)),
            (BalloonArrow::Left, (x, y + h / 2.0)),
            (BalloonArrow::Right, (x + w, y + h / 2.0)),
        ];
        for (arrow, (tip_x, tip_y)) in cases {
            let path = draw_balloon_rect(x, y, w, h, 5.0, 12.5, 10.0, 0.5, 1.5, arrow);
            let closest = path
                .points()
                .iter()
                .map(|p| ((p.x - tip_x).powi(2) + (p.y - tip_y).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            // The tip is rounded, so it stops just short of the exact corner.
            assert!(
                closest < 2.0,
                "{arrow:?}: no point near the tip ({tip_x}, {tip_y}); closest was {closest} away"
            );
        }
    }

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
            family_name_cache: RefCell::new(HashMap::new()),
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
            family_name, "DejaVu Sans",
            "expected shortest DejaVu family ('DejaVu Sans'), got '{}'",
            family_name
        );
    }

    #[test]
    #[ignore]
    fn fallback_with_fuzzy_returns_font() {
        let cache = make_test_cache();
        let style = layers::skia::FontStyle::normal();
        // make_font_with_fallback should use fuzzy matching before falling back to generic fallbacks
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
