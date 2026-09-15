//! A rect of one colour on a surface of its own.
//!
//! One pixel, stretched and rounded by the compositor: moving, resizing or
//! recolouring it is a request, never a paint. What a scroll pane's highlight
//! is made of, and what a host uses for anything flat that has to move with a
//! pane rather than be painted into one — a divider down a column's edge, a
//! tint behind it.

use skia_safe::{Color, Rect};
use wayland_client::protocol::wl_surface::WlSurface;

use crate::app_runner::AppContext;
use crate::surfaces::{SubsurfaceSurface, SurfaceError};

/// A rect of one colour, a child of some parent surface.
pub struct Fill {
    surface: SubsurfaceSurface,
    color: Option<Color>,
    radius: f32,
    /// Where it is wanted, in the parent's points, and the rect and scale last
    /// sent for it.
    wanted: Option<Rect>,
    sent: Option<(Rect, f32)>,
    hidden: bool,
}

impl Fill {
    /// A fill under `parent`, taking no pointer input, with nothing to show
    /// until it has a colour and a rect.
    pub fn new(parent: &WlSurface) -> Result<Self, SurfaceError> {
        let surface = SubsurfaceSurface::new(parent, 0, 0, 1, 1)?;
        let region = AppContext::compositor_state()
            .wl_compositor()
            .create_region(AppContext::queue_handle(), ());
        surface.wl_surface().set_input_region(Some(&region));
        region.destroy();
        surface.commit();
        Ok(Self {
            surface,
            color: None,
            radius: 0.0,
            wanted: None,
            sent: None,
            hidden: false,
        })
    }

    pub fn wl_surface(&self) -> &WlSurface {
        self.surface.wl_surface()
    }

    /// Stack directly above `sibling`. Parent state: it lands with the
    /// parent's next commit.
    pub fn place_above(&self, sibling: &WlSurface) {
        self.surface.place_above(sibling);
    }

    /// Stack directly below `sibling`. Parent state, like [`Self::place_above`].
    pub fn place_below(&self, sibling: &WlSurface) {
        self.surface.place_below(sibling);
    }

    /// Its colour and corner radius, in points. The pixel is painted again
    /// only when the colour changes. Returns whether anything was sent.
    pub fn set_style(&mut self, color: Color, radius: f32) -> bool {
        let mut sent = false;
        if self.color != Some(color) {
            self.color = Some(color);
            self.surface.draw(|canvas| {
                canvas.clear(color);
            });
            sent = true;
        }
        if self.radius != radius {
            self.radius = radius;
            if let Some(style) = self.surface.layer() {
                style.set_corner_radius(radius as f64);
            }
            sent = true;
        }
        // A rect asked for before there was a pixel to stretch goes out now.
        sent |= self.send_rect();
        if sent {
            self.surface.commit();
        }
        sent
    }

    /// Where it is, in the parent's points. Placed on whole pixels. Returns
    /// whether anything was sent.
    pub fn set_rect(&mut self, rect: Rect) -> bool {
        self.wanted = Some(rect);
        let sent = self.send_rect();
        if sent {
            self.surface.commit();
        }
        sent
    }

    /// Take it out of sight, or bring it back. Returns whether that changed
    /// anything.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        if let Some(style) = self.surface.layer() {
            style.set_opacity(if hidden { 0.0 } else { 1.0 });
        }
        self.surface.commit();
        true
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Claim the wanted rect through the style, once there is a pixel for it:
    /// a size claimed over no buffer is a size with nothing to stretch.
    fn send_rect(&mut self) -> bool {
        let (Some(rect), Some(_)) = (self.wanted, self.color) else {
            return false;
        };
        let scale = AppContext::fractional_scale() as f32;
        if self.sent == Some((rect, scale)) {
            return false;
        }
        self.sent = Some((rect, scale));
        if let Some(style) = self.surface.layer() {
            let px = |points: f32| (points * scale).round() as f64;
            style.set_size(px(rect.width()), px(rect.height()));
            style.set_position(px(rect.left), px(rect.top));
        }
        true
    }
}

impl Drop for Fill {
    /// A fill let go of is torn down: a subsurface merely forgotten stays
    /// mapped until the client exits.
    fn drop(&mut self) {
        self.surface.destroy();
    }
}
