//! A rect of one colour on a surface of its own.
//!
//! One pixel, stretched by its viewport: moving, resizing or recolouring it is
//! a request, never a paint. What a scroll pane's highlight is made of, and
//! what a host uses for anything flat that has to move with a pane rather than
//! be painted into one — a divider down a column's edge, a tint behind it.
//!
//! Rounded corners are the compositor's where it has the surface style; where
//! it has not, a rounded fill paints its shape at full size instead, and is
//! painted again when that size changes.

use skia_safe::{Color, Rect, Size};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;

use crate::app_runner::AppContext;
use crate::surfaces::{SubsurfaceSurface, SurfaceError};

use super::crop;

/// A rect of one colour, a child of some parent surface.
pub struct Fill {
    surface: SubsurfaceSurface,
    color: Option<Color>,
    radius: f32,
    /// Where it is wanted, in the parent's points.
    wanted: Option<Rect>,
    /// The part of the parent that shows it, in the parent's points; `None`
    /// shows the whole fill.
    window: Option<Rect>,
    /// The rect, window and scale last placed, to skip repeating them.
    sent: Option<(Rect, Option<Rect>, f32)>,
    /// The size its buffer was last painted at, in points.
    painted: Option<(i32, i32)>,
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
            window: None,
            sent: None,
            painted: None,
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

    /// Its colour and corner radius, in points. Returns whether anything was
    /// sent.
    pub fn set_style(&mut self, color: Color, radius: f32) -> bool {
        if self.color == Some(color) && self.radius == radius {
            return false;
        }
        self.color = Some(color);
        if self.radius != radius {
            self.radius = radius;
            if let Some(style) = self.surface.layer() {
                style.set_corner_radius(radius as f64);
            }
        }
        self.painted = None;
        self.update();
        true
    }

    /// Where it is, in the parent's points. Placed on whole points. Returns
    /// whether anything was sent.
    pub fn set_rect(&mut self, rect: Rect) -> bool {
        self.wanted = Some(rect);
        self.update()
    }

    /// Show only the part of it inside `window`, in the parent's points, or
    /// all of it with `None`: for a fill in a pane that shows only part of
    /// its parent. Returns whether anything was sent.
    pub fn set_window(&mut self, window: Option<Rect>) -> bool {
        if self.window == window {
            return false;
        }
        self.window = window;
        self.update()
    }

    /// Take it out of sight, or bring it back. Returns whether that changed
    /// anything.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        if hidden {
            self.surface.unmap();
        } else {
            self.sent = None;
            self.update();
        }
        true
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// The buffer size it is painted at, in points: one pixel, stretched,
    /// unless it is rounded and nothing else will round it.
    fn buffer_size(&self, rect: Rect) -> (i32, i32) {
        if self.radius > 0.0 && self.surface.layer().is_none() {
            (
                rect.width().round().max(1.0) as i32,
                rect.height().round().max(1.0) as i32,
            )
        } else {
            (1, 1)
        }
    }

    /// Place, crop and paint whatever the wanted rect, window and colour call
    /// for. A fill with no colour or no rect yet waits for both. Returns
    /// whether anything was sent.
    fn update(&mut self) -> bool {
        let (Some(rect), Some(color)) = (self.wanted, self.color) else {
            return false;
        };
        if self.hidden {
            return false;
        }
        // The parent's: this surface is on the outputs its parent is on.
        let scale = AppContext::surface_scale(&self.surface.parent_surface().id()) as f32;
        let size = self.buffer_size(rect);
        let paint = self.painted != Some(size) || !self.surface.is_mapped();
        if !paint && self.sent == Some((rect, self.window, scale)) {
            return false;
        }
        self.sent = Some((rect, self.window, scale));
        if paint {
            self.surface.resize(size.0, size.1);
        }
        let content = Size::new(size.0 as f32, size.1 as f32);
        let window = self.window.unwrap_or(rect);
        // The crop is set before the paint, so it goes out with the buffer
        // it fits.
        let Some(shown) = crop::show(&mut self.surface, content, rect, window, scale) else {
            self.painted = None;
            return true;
        };
        if paint {
            self.painted = Some(size);
            let radius = self.radius;
            self.surface.draw(|canvas| {
                if size == (1, 1) {
                    canvas.clear(color);
                    return;
                }
                canvas.clear(Color::TRANSPARENT);
                let mut fill = skia_safe::Paint::default();
                fill.set_anti_alias(true);
                fill.set_color(color);
                let shape = Rect::from_wh(size.0 as f32, size.1 as f32);
                canvas.draw_rrect(skia_safe::RRect::new_rect_xy(shape, radius, radius), &fill);
            });
        } else {
            self.surface.commit();
        }
        self.surface
            .set_position(shown.position.0, shown.position.1);
        self.surface.commit_parent();
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
