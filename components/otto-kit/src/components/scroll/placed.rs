//! A painted surface the client places itself.
//!
//! What sits beside a scroll pane rather than in one: a status line over an
//! empty column, a preview card, a panel dragged clear of its window. A child
//! of some parent surface, placed with `wl_subsurface`, painted only when what
//! it shows changes, and never faster than the compositor shows it.
//!
//! Two rules it keeps so its hosts do not have to:
//!
//! - **A move lands with the parent.** A subsurface's position is its
//!   parent's state, applied when the parent commits, so a move commits the
//!   parent. Without that the surface stays where it was created until
//!   something else happens to commit the parent.
//! - **Hidden is off the screen and out of the pointer's way.** It is
//!   unmapped, and painted again when it comes back.
//!
//! A surface inside a pane that shows only part of its parent — a column's
//! status line in a stack panned half away — is cropped to what shows with
//! [`PlacedSurface::set_window`]: no compositor clips a subsurface to its
//! parent.

use skia_safe::{Canvas, Rect, Size};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;

use crate::app_runner::AppContext;
use crate::protocols::otto_surface_style_v1::OttoSurfaceStyleV1;
use crate::surfaces::{SubsurfaceSurface, SurfaceError};

use super::crop;

/// What [`PlacedSurface::paint`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    /// The surface already shows this key.
    Unchanged,
    /// The last frame has not reached the screen: nothing was painted, and the
    /// host has to come back — keep its update loop turning until it does.
    Held,
    Painted,
}

/// A painted child surface, placed by its client.
pub struct PlacedSurface {
    surface: SubsurfaceSurface,
    /// Where it is, in the parent's points.
    rect: Rect,
    /// The key of what the buffer shows.
    key: Option<u64>,
    /// The part of the parent that shows it, in the parent's points; `None`
    /// shows all of it.
    window: Option<Rect>,
    /// The size, in points, of the buffer last painted. Until a resize is
    /// painted, the old buffer is shown at its own size where the new rect
    /// starts, never stretched to it.
    painted: Option<(i32, i32)>,
    hidden: bool,
    takes_input: bool,
}

impl PlacedSurface {
    /// A surface under `parent` at `rect`, in the parent's points, taking no
    /// pointer input, with nothing on it until the first [`Self::paint`].
    pub fn new(parent: &WlSurface, rect: Rect) -> Result<Self, SurfaceError> {
        let surface = SubsurfaceSurface::new(
            parent,
            rect.left.round() as i32,
            rect.top.round() as i32,
            rect.width().max(1.0) as i32,
            rect.height().max(1.0) as i32,
        )?;
        set_input(surface.wl_surface(), false);
        surface.commit();
        surface.commit_parent();
        Ok(Self {
            surface,
            rect,
            key: None,
            window: None,
            painted: None,
            hidden: false,
            takes_input: false,
        })
    }

    pub fn wl_surface(&self) -> &WlSurface {
        self.surface.wl_surface()
    }

    /// The style object, for the material: corners, shadow, blend mode.
    pub fn style(&self) -> Option<&OttoSurfaceStyleV1> {
        self.surface.layer()
    }

    /// Where it is, in the parent's points.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Stack directly above `sibling`. Parent state: it lands with the
    /// parent's next commit.
    pub fn place_above(&self, sibling: &WlSurface) {
        self.surface.place_above(sibling);
    }

    /// Stack directly below `sibling`, like [`Self::place_above`].
    pub fn place_below(&self, sibling: &WlSurface) {
        self.surface.place_below(sibling);
    }

    /// Answer for the pointer over the whole surface, or for none of it.
    ///
    /// For a surface that hangs outside its window — events over it never
    /// reach the window. Hidden, it is unmapped and answers for nothing.
    pub fn set_takes_input(&mut self, yes: bool) {
        if self.takes_input == yes {
            return;
        }
        self.takes_input = yes;
        set_input(self.surface.wl_surface(), yes);
        self.surface.commit();
    }

    /// Put it at `rect`, in the parent's points, on whole points. Returns
    /// whether it was resized: the next [`Self::paint`] then paints whatever
    /// the key.
    pub fn set_rect(&mut self, rect: Rect) -> bool {
        if self.rect == rect {
            return false;
        }
        let resized = self.rect.width() != rect.width() || self.rect.height() != rect.height();
        self.rect = rect;
        if resized {
            self.surface
                .resize(rect.width().max(1.0) as i32, rect.height().max(1.0) as i32);
            self.key = None;
        }
        self.place(true);
        resized
    }

    /// Show only the part of it inside `window`, in the parent's points, or
    /// all of it with `None`. Returns whether that changed anything.
    pub fn set_window(&mut self, window: Option<Rect>) -> bool {
        if self.window == window {
            return false;
        }
        self.window = window;
        self.place(true);
        true
    }

    /// Crop the buffer last painted to the window and put it where the rect
    /// starts, committing it when `commit` says so and it has a buffer to
    /// commit. A surface the window does not reach is unmapped, and painted
    /// again by the first [`Self::paint`] once it shows. Returns whether any
    /// of it shows.
    fn place(&mut self, commit: bool) -> bool {
        let rect = self.rect;
        let Some((width, height)) = self.painted else {
            self.surface
                .set_position(rect.left.round() as i32, rect.top.round() as i32);
            self.surface.commit_parent();
            return true;
        };
        let content = Size::new(width as f32, height as f32);
        let extent = Rect::from_xywh(rect.left, rect.top, content.width, content.height);
        let window = self.window.unwrap_or(extent);
        let Some(shown) = crop::show(&mut self.surface, content, extent, window, scale()) else {
            self.key = None;
            return false;
        };
        if !self.surface.is_mapped() {
            self.key = None;
        } else if commit {
            self.surface.commit();
        }
        self.surface
            .set_position(shown.position.0, shown.position.1);
        self.surface.commit_parent();
        true
    }

    /// Paint it with `paint`, on a canvas in the surface's own points, unless
    /// it already shows `key` or its last frame is still on the way. A hidden
    /// surface is not painted.
    pub fn paint(&mut self, key: u64, paint: impl FnOnce(&Canvas)) -> Paint {
        if self.hidden || self.key == Some(key) {
            return Paint::Unchanged;
        }
        if AppContext::frame_in_flight(&self.surface.wl_surface().id()) {
            return Paint::Held;
        }
        // The crop for the new buffer is set before the paint commits it: a
        // source rect over a buffer it does not lie inside is a protocol
        // error. Outside the window there is nothing to paint.
        self.painted = Some(self.surface.dimensions());
        if !self.place(false) {
            return Paint::Unchanged;
        }
        self.key = Some(key);
        self.surface.draw(paint);
        Paint::Painted
    }

    /// Paint on the next [`Self::paint`] whatever its key.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// Take it out of sight — and out of the pointer's way — or bring it back.
    /// Brought back, it shows nothing until the next [`Self::paint`]. Returns
    /// whether that changed anything.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        if hidden {
            self.surface.unmap();
            self.key = None;
        }
        true
    }

    /// Ask the compositor where this surface's output is. The answer arrives
    /// a round trip later, through [`Self::output_frame`]; an older answer is
    /// forgotten first, so it cannot be mistaken for the new one.
    pub fn ask_output_frame(&self) {
        let Some(style) = self.surface.layer() else {
            return;
        };
        AppContext::clear_output_frame(&style.id());
        style.request_output_frame();
    }

    /// The output this surface is on, in the parent's points, once the
    /// compositor has answered [`Self::ask_output_frame`].
    pub fn output_frame(&self) -> Option<Rect> {
        let style = self.surface.layer()?;
        let (x, y, width, height) = AppContext::output_frame(&style.id())?;
        output_rect((x, y, width, height), scale())
    }
}

impl Drop for PlacedSurface {
    /// Torn down, not merely forgotten: a subsurface let go of stays mapped,
    /// with its buffer and its input region, until the client exits.
    fn drop(&mut self) {
        self.surface.destroy();
    }
}

fn scale() -> f32 {
    AppContext::fractional_scale() as f32
}

/// An output frame in physical pixels, in points.
fn output_rect((x, y, width, height): (f32, f32, f32, f32), scale: f32) -> Option<Rect> {
    if width <= 0.0 || height <= 0.0 || scale <= 0.0 {
        return None;
    }
    Some(Rect::from_xywh(
        x / scale,
        y / scale,
        width / scale,
        height / scale,
    ))
}

/// The whole surface for the pointer, or none of it. The compositor copies a
/// region on `set_input_region`, so an empty one is destroyed straight away.
fn set_input(surface: &WlSurface, yes: bool) {
    if yes {
        surface.set_input_region(None);
        return;
    }
    let region = AppContext::compositor_state()
        .wl_compositor()
        .create_region(AppContext::queue_handle(), ());
    surface.set_input_region(Some(&region));
    region.destroy();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_frame_is_read_in_points() {
        assert_eq!(
            output_rect((300.0, 0.0, 2880.0, 1920.0), 1.5),
            Some(Rect::from_xywh(200.0, 0.0, 1920.0, 1280.0))
        );
    }

    #[test]
    fn an_empty_output_frame_is_no_answer() {
        assert_eq!(output_rect((0.0, 0.0, 0.0, 1080.0), 1.0), None);
        assert_eq!(output_rect((0.0, 0.0, 1920.0, 1080.0), 0.0), None);
    }
}
