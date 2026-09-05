//! Resize borders for server-side decorated windows.
//!
//! A client that draws its own decoration owns its resize affordances (see
//! otto-kit's `components::window::resize`); a client Otto decorates has none —
//! it never asks for a resize, because from its point of view it has no frame
//! to grab. The compositor has to offer the border itself, which is what this
//! view is: a strip along the window's own edges that is hit-tested ahead of
//! the client's surfaces and starts a resize grab on press.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use smithay::{
    backend::input::ButtonState,
    input::pointer::{CursorIcon, CursorImageStatus},
    utils::{IsAlive, Point, Size},
};

use crate::{
    interactive_view::ViewInteractions,
    shell::{ResizeEdge, WindowElement},
};

const BTN_LEFT: u32 = 0x110;

/// How far inside a window edge a press still starts a resize, in logical
/// points. Wide enough to hit without aiming, narrow enough that it does not
/// eat the client's own edge controls (scrollbars sit further in).
const RESIZE_MARGIN: f64 = 6.0;

/// Which edges a window-local point grabs, if any. `size` is the window rect
/// including the titlebar — the same space `element_under` reports positions
/// in.
pub fn resize_edges_at(
    local: Point<f64, smithay::utils::Logical>,
    size: Size<i32, smithay::utils::Logical>,
) -> Option<ResizeEdge> {
    // A window narrower than two margins has no interior left; resizing it
    // from both sides at once is not a thing, so leave it to the titlebar.
    let (w, h) = (size.w as f64, size.h as f64);
    if w <= RESIZE_MARGIN * 2.0 || h <= RESIZE_MARGIN * 2.0 {
        return None;
    }
    // The border is a strip just *inside* the window's own rect. A point can
    // reach here from outside it — the space attributes a point to a window
    // whenever its bounding box contains it, and that box grows to cover the
    // window's popups — and `>= w - MARGIN` would then read every point to the
    // right of the window as its right border, swallowing the part of a menu
    // that hangs off the edge.
    if local.x < 0.0 || local.y < 0.0 || local.x >= w || local.y >= h {
        return None;
    }
    let mut edges = ResizeEdge::NONE;
    if local.x < RESIZE_MARGIN {
        edges |= ResizeEdge::LEFT;
    } else if local.x >= w - RESIZE_MARGIN {
        edges |= ResizeEdge::RIGHT;
    }
    if local.y < RESIZE_MARGIN {
        edges |= ResizeEdge::TOP;
    } else if local.y >= h - RESIZE_MARGIN {
        edges |= ResizeEdge::BOTTOM;
    }
    (edges != ResizeEdge::NONE).then_some(edges)
}

/// The cursor that names what a border grab would do.
fn cursor_for(edges: ResizeEdge) -> CursorIcon {
    match edges {
        ResizeEdge::TOP => CursorIcon::NResize,
        ResizeEdge::BOTTOM => CursorIcon::SResize,
        ResizeEdge::LEFT => CursorIcon::WResize,
        ResizeEdge::RIGHT => CursorIcon::EResize,
        ResizeEdge::TOP_LEFT => CursorIcon::NwResize,
        ResizeEdge::TOP_RIGHT => CursorIcon::NeResize,
        ResizeEdge::BOTTOM_LEFT => CursorIcon::SwResize,
        ResizeEdge::BOTTOM_RIGHT => CursorIcon::SeResize,
        _ => CursorIcon::default(),
    }
}

/// One window's resize border, as an input target.
#[derive(Clone)]
pub struct WindowResizeView {
    pub window: WindowElement,
    pub edges: ResizeEdge,
    /// Set while a press that started on the border is still held, so the
    /// cursor is not reset out from under the grab.
    pressed: Arc<AtomicBool>,
}

impl WindowResizeView {
    pub fn new(window: WindowElement, edges: ResizeEdge) -> Self {
        Self {
            window,
            edges,
            pressed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl PartialEq for WindowResizeView {
    fn eq(&self, other: &Self) -> bool {
        self.window == other.window && self.edges == other.edges
    }
}

impl std::fmt::Debug for WindowResizeView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowResizeView")
            .field("window", &self.window.id())
            .field("edges", &self.edges)
            .finish()
    }
}

impl<Backend: crate::state::Backend> ViewInteractions<Backend> for WindowResizeView {
    fn id(&self) -> Option<usize> {
        // The edges are part of the identity: crossing from one border to the
        // next has to look like a new target, or the cursor would keep the
        // shape it entered with.
        let base: usize = self.window.base_layer().id.0.into();
        Some(base.rotate_left(32) ^ self.edges.bits() as usize)
    }

    fn is_alive(&self) -> bool {
        self.window.alive()
    }

    fn on_enter(&self, _event: &smithay::input::pointer::MotionEvent) {}

    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        _event: &smithay::input::pointer::MotionEvent,
    ) {
        data.set_cursor(&CursorImageStatus::Named(cursor_for(self.edges)));
    }

    fn on_leave_with_data(
        &self,
        data: &mut crate::Otto<Backend>,
        _serial: smithay::utils::Serial,
        _time: u32,
    ) {
        if !self.pressed.load(Ordering::SeqCst) {
            data.set_cursor(&CursorImageStatus::Named(CursorIcon::default()));
        }
    }

    fn on_button(
        &self,
        seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        if event.button != BTN_LEFT {
            return;
        }
        match event.state {
            ButtonState::Pressed => {
                self.pressed.store(true, Ordering::SeqCst);
                // Deferred for the same reason as the titlebar's move grab:
                // this runs inside `PointerHandle::button`, which holds the
                // pointer's inner lock, and installing a grab takes it again.
                let window = self.window.clone();
                let seat = seat.clone();
                let serial = event.serial;
                let edges = self.edges;
                data.handle.insert_idle(move |state| {
                    state.resize_request_ssd(&window, &seat, serial, edges);
                });
            }
            ButtonState::Released => {
                self.pressed.store(false, Ordering::SeqCst);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges_at(x: f64, y: f64) -> Option<ResizeEdge> {
        resize_edges_at((x, y).into(), (200, 100).into())
    }

    #[test]
    fn interior_grabs_nothing() {
        assert_eq!(edges_at(100.0, 50.0), None);
        // Just inside the margin on every side.
        assert_eq!(edges_at(RESIZE_MARGIN, RESIZE_MARGIN), None);
        assert_eq!(edges_at(200.0 - RESIZE_MARGIN - 1.0, 50.0), None);
    }

    #[test]
    fn borders_grab_their_own_edge() {
        assert_eq!(edges_at(0.0, 50.0), Some(ResizeEdge::LEFT));
        assert_eq!(edges_at(199.0, 50.0), Some(ResizeEdge::RIGHT));
        assert_eq!(edges_at(100.0, 0.0), Some(ResizeEdge::TOP));
        assert_eq!(edges_at(100.0, 99.0), Some(ResizeEdge::BOTTOM));
    }

    #[test]
    fn corners_grab_both() {
        assert_eq!(edges_at(0.0, 0.0), Some(ResizeEdge::TOP_LEFT));
        assert_eq!(edges_at(199.0, 0.0), Some(ResizeEdge::TOP_RIGHT));
        assert_eq!(edges_at(0.0, 99.0), Some(ResizeEdge::BOTTOM_LEFT));
        assert_eq!(edges_at(199.0, 99.0), Some(ResizeEdge::BOTTOM_RIGHT));
    }

    #[test]
    fn a_window_too_small_for_two_margins_has_no_border() {
        assert_eq!(resize_edges_at((1.0, 1.0).into(), (8, 8).into()), None);
    }
}
