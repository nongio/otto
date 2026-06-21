use layers::{
    engine::Engine,
    prelude::*,
    types::{Point, Size},
};
use smithay::utils::{Logical, Rectangle};
use std::sync::Arc;


/// A window-tiling drop zone a dragged window can snap into.
///
/// macOS-style: pushing a dragged window toward the top-center maximizes it,
/// toward the left/right edge tiles it to that half of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileZone {
    Maximize,
    LeftHalf,
    RightHalf,
}

impl TileZone {
    /// The target rectangle (logical pixels) for this zone within the usable area.
    pub fn target_rect(self, usable: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let half_w = usable.size.w / 2;
        match self {
            TileZone::Maximize => usable,
            TileZone::LeftHalf => Rectangle::new(usable.loc, (half_w, usable.size.h).into()),
            TileZone::RightHalf => Rectangle::new(
                (usable.loc.x + usable.size.w - half_w, usable.loc.y).into(),
                (half_w, usable.size.h).into(),
            ),
        }
    }
}

/// Classify a pointer position (logical) within the usable output area into a
/// tiling zone, or `None` when it isn't near an activating edge.
pub fn zone_from_pointer(
    usable: Rectangle<i32, Logical>,
    pointer: smithay::utils::Point<f64, Logical>,
) -> Option<TileZone> {
    // Top maximize band, in logical pixels.
    const TOP_BAND: f64 = 80.0;
    // Left/right zones span a generous fraction of the width (with a floor) so
    // they're easy to hit, not just a thin strip at the very edge.
    let edge_band = (usable.size.w as f64 * 0.15).max(120.0);

    let left = usable.loc.x as f64;
    let right = (usable.loc.x + usable.size.w) as f64;
    let top = usable.loc.y as f64;

    // Central horizontal band (middle 50% of the usable width) for maximize.
    let center_lo = left + usable.size.w as f64 * 0.25;
    let center_hi = right - usable.size.w as f64 * 0.25;

    // Top-center → maximize wins over the side bands at the upper corners.
    if pointer.y <= top + TOP_BAND && pointer.x >= center_lo && pointer.x <= center_hi {
        return Some(TileZone::Maximize);
    }
    if pointer.x <= left + edge_band {
        return Some(TileZone::LeftHalf);
    }
    if pointer.x >= right - edge_band {
        return Some(TileZone::RightHalf);
    }
    None
}

/// Compositor-drawn overlay showing where a dragged window will snap.
///
/// A single translucent rounded rectangle (`preview_layer`) lives inside a
/// full-screen, non-interactive `wrap_layer` attached to the overlay tree. The
/// preview animates between zones, fading in when a zone is active and out when
/// none is. Modeled on [`crate::workspaces::osd::OsdView`].
pub struct TilingOverlayView {
    pub wrap_layer: Layer,
    pub preview_layer: Layer,
}

impl TilingOverlayView {
    pub fn new(layers_engine: Arc<Engine>) -> Self {
        let wrap = layers_engine.new_layer();
        wrap.set_key("tiling_overlay_container");
        wrap.set_size(Size::percent(1.0, 1.0), None);
        wrap.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        wrap.set_pointer_events(false);
        wrap.set_hidden(true);

        let preview = layers_engine.new_layer();
        preview.set_key("tiling_overlay_preview");
        preview.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        preview.set_pointer_events(false);
        preview.set_opacity(0.0, None);
        preview.set_background_color(
            PaintColor::Solid {
                color: Color::new_rgba(1.0, 1.0, 1.0, 0.3),
            },
            None,
        );
        preview.set_border_color(
            PaintColor::Solid {
                color: Color::new_rgba(1.0, 1.0, 1.0, 0.8),
            },
            None,
        );
        let _ = wrap.add_sublayer(&preview);

        Self {
            wrap_layer: wrap,
            preview_layer: preview,
        }
    }

    /// Show the preview at the given rectangle, in **physical pixels**.
    ///
    /// `scale` is the output's fractional scale, used for the corner radius and
    /// border width so they look right on HiDPI outputs.
    pub fn show_zone(&self, x_px: f32, y_px: f32, w_px: f32, h_px: f32, scale: f32) {
        // First appearance: place the preview at the zone instantly and fade it
        // in, so it doesn't fly in from the (0,0) corner. While already visible,
        // animate position/size as the pointer moves between zones.
        let first_show = self.wrap_layer.hidden();
        self.wrap_layer.set_hidden(false);

        self.preview_layer.set_border_width(2.0 * scale, None);
        self.preview_layer
            .set_border_corner_radius(BorderRadius::new_single(12.0 * scale), None);

        let move_transition = if first_show {
            None
        } else {
            Some(Transition::ease_out_quad(0.15))
        };
        self.preview_layer
            .set_position(Point { x: x_px, y: y_px }, move_transition.clone());
        self.preview_layer
            .set_size(Size::points(w_px, h_px), move_transition);
        self.preview_layer
            .set_opacity(1.0, Some(Transition::ease_out_quad(0.2)));
    }

    /// Fade the preview out and hide the container once it finishes.
    pub fn hide(&self) {
        let w = self.wrap_layer.clone();
        self.preview_layer
            .set_opacity(0.0, Some(Transition::ease_out_quad(0.15)))
            .on_finish(
                move |_l: &Layer, _| {
                    w.set_hidden(true);
                },
                true,
            );
    }

    /// Whether the overlay is currently shown.
    pub fn is_visible(&self) -> bool {
        !self.wrap_layer.hidden()
    }
}
