//! A painted surface the client places itself.
//!
//! What sits beside a scroll pane rather than in one: a status line over an
//! empty column, a preview card, a panel dragged clear of its window. A child
//! of some parent surface, placed and sized through `otto_surface_style_v1`,
//! painted only when what it shows changes, and never faster than the
//! compositor shows it.
//!
//! Three rules it keeps so its hosts do not have to:
//!
//! - **A size is claimed behind the buffer that fits it.** The style applies a
//!   size the moment it arrives; claimed ahead of the paint, the old pixels are
//!   drawn stretched to the new bounds until the paint lands. A resize waits
//!   for [`PlacedSurface::paint`], and a move that arrives while one waits
//!   joins it.
//! - **Geometry uses the output's fractional scale**, read fresh, snapped to
//!   whole pixels — the preferred scale arrives after most surfaces are built,
//!   and whatever claimed the integer fallback is claimed again.
//! - **Hidden is out of the pointer's way too.** Opacity alone leaves a mapped
//!   surface answering for every event over it; a surface that takes input
//!   withdraws its region while hidden.

use skia_safe::{Canvas, Rect};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;

use crate::app_runner::AppContext;
use crate::protocols::otto_surface_style_v1::OttoSurfaceStyleV1;
use crate::surfaces::{BufferClaim, SubsurfaceSurface, SurfaceError};

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
    /// The rect and scale last claimed through the style.
    claimed: Option<(Rect, f32)>,
    /// A resize waiting for a buffer that fits it.
    claim_waiting: bool,
    /// The key of what the buffer shows.
    key: Option<u64>,
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
        Ok(Self {
            surface,
            rect,
            claimed: None,
            claim_waiting: true,
            key: None,
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
    /// reach the window. Withdrawn while the surface is hidden.
    pub fn set_takes_input(&mut self, yes: bool) {
        if self.takes_input == yes {
            return;
        }
        self.takes_input = yes;
        if !self.hidden {
            set_input(self.surface.wl_surface(), yes);
            self.surface.commit();
        }
    }

    /// Put it at `rect`, in the parent's points. Returns whether it was
    /// resized: the next [`Self::paint`] then paints whatever the key.
    pub fn set_rect(&mut self, rect: Rect) -> bool {
        let scale = scale();
        if self.rect == rect && self.claimed.is_some_and(|(_, s)| s == scale) {
            return false;
        }
        let resized = self.rect.width() != rect.width() || self.rect.height() != rect.height();
        self.rect = rect;
        // The pointer is hit-tested against the subsurface position. Parent
        // state, so it lands with the parent's commit whatever the buffer is.
        self.surface
            .set_position(rect.left.round() as i32, rect.top.round() as i32);
        if resized {
            self.surface
                .resize(rect.width().max(1.0) as i32, rect.height().max(1.0) as i32);
            self.key = None;
            self.claim_waiting = true;
        }
        if !self.claim_waiting {
            self.claim();
            self.surface.commit();
        }
        resized
    }

    /// Paint it with `paint`, on a canvas in the surface's own points, unless
    /// it already shows `key` or its last frame is still on the way.
    pub fn paint(&mut self, key: u64, paint: impl FnOnce(&Canvas)) -> Paint {
        if self.key == Some(key) && !self.claim_waiting {
            return Paint::Unchanged;
        }
        if AppContext::frame_in_flight(&self.surface.wl_surface().id()) {
            return Paint::Held;
        }
        self.key = Some(key);
        if let Some(claim) = self.take_claim() {
            self.surface.claim_with_next_buffer(claim);
        }
        self.surface.draw(paint);
        self.claim_waiting = false;
        Paint::Painted
    }

    /// Paint on the next [`Self::paint`] whatever its key.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// Take it out of sight — and out of the pointer's way — or bring it back.
    /// Returns whether that changed anything.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        if let Some(style) = self.surface.layer() {
            style.set_opacity(if hidden { 0.0 } else { 1.0 });
        }
        if self.takes_input {
            set_input(self.surface.wl_surface(), !hidden);
        }
        self.surface.commit();
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

    /// Claim the current rect for the buffer already shown, at the current
    /// scale.
    fn claim(&mut self) {
        let Some(claim) = self.take_claim() else {
            return;
        };
        if let Some(style) = self.surface.layer() {
            style.set_size(claim.width, claim.height);
            if let Some((x, y)) = claim.position {
                style.set_position(x, y);
            }
        }
    }

    /// The rect to claim, in pixels, unless it is claimed already. Records it
    /// as claimed.
    fn take_claim(&mut self) -> Option<BufferClaim> {
        let scale = scale();
        if self.claimed == Some((self.rect, scale)) {
            return None;
        }
        self.claimed = Some((self.rect, scale));
        let px = |points: f32| (points * scale).round() as f64;
        Some(BufferClaim {
            width: px(self.rect.width()),
            height: px(self.rect.height()),
            position: Some((px(self.rect.left), px(self.rect.top))),
        })
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
