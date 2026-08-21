//! Edge and corner resizing for client-decorated windows.
//!
//! A window that draws its own decoration also owns its resize affordances:
//! the compositor has no titlebar or border to grab on the client's behalf.
//! This module supplies the two halves an app needs — which edge a point is
//! on, and the cursor that edge should show — so every otto-kit window
//! behaves the same way rather than each app inventing its own margins.
//!
//! Follows the toolkit convention: a pure geometry helper with no client
//! runtime in it, and a thin client-side action on [`Window`].

use skia_safe::Rect;
use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge as XdgResizeEdge;

use crate::CursorShape;

/// How far inside the window edge a press still counts as a resize.
///
/// Generous enough to hit without precision, small enough not to swallow
/// presses meant for content sitting near the edge.
pub const GRAB: f32 = 6.0;

/// Which edge or corner a point is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeEdge {
    /// The cursor that tells the user which way this edge will move.
    pub fn cursor(self) -> CursorShape {
        match self {
            ResizeEdge::Top => CursorShape::NResize,
            ResizeEdge::Bottom => CursorShape::SResize,
            ResizeEdge::Left => CursorShape::WResize,
            ResizeEdge::Right => CursorShape::EResize,
            ResizeEdge::TopLeft => CursorShape::NwResize,
            ResizeEdge::TopRight => CursorShape::NeResize,
            ResizeEdge::BottomLeft => CursorShape::SwResize,
            ResizeEdge::BottomRight => CursorShape::SeResize,
        }
    }

    pub(crate) fn to_xdg(self) -> XdgResizeEdge {
        match self {
            ResizeEdge::Top => XdgResizeEdge::Top,
            ResizeEdge::Bottom => XdgResizeEdge::Bottom,
            ResizeEdge::Left => XdgResizeEdge::Left,
            ResizeEdge::Right => XdgResizeEdge::Right,
            ResizeEdge::TopLeft => XdgResizeEdge::TopLeft,
            ResizeEdge::TopRight => XdgResizeEdge::TopRight,
            ResizeEdge::BottomLeft => XdgResizeEdge::BottomLeft,
            ResizeEdge::BottomRight => XdgResizeEdge::BottomRight,
        }
    }
}

/// The edge a window-local point is on, if any.
///
/// Corners win over edges: within `GRAB` of two sides, the diagonal is what
/// the user meant. Points outside the window, or further in than the grab
/// margin, return `None` and belong to the content.
pub fn edge_at(size: Rect, x: f32, y: f32) -> Option<ResizeEdge> {
    if x < 0.0 || y < 0.0 || x > size.width() || y > size.height() {
        return None;
    }

    let left = x <= GRAB;
    let right = x >= size.width() - GRAB;
    let top = y <= GRAB;
    let bottom = y >= size.height() - GRAB;

    match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, ..) => Some(ResizeEdge::Top),
        (_, true, ..) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::from_wh(200.0, 100.0)
    }

    #[test]
    fn corners_beat_edges() {
        assert_eq!(edge_at(window(), 1.0, 1.0), Some(ResizeEdge::TopLeft));
        assert_eq!(
            edge_at(window(), 199.0, 99.0),
            Some(ResizeEdge::BottomRight)
        );
        assert_eq!(edge_at(window(), 199.0, 1.0), Some(ResizeEdge::TopRight));
        assert_eq!(edge_at(window(), 1.0, 99.0), Some(ResizeEdge::BottomLeft));
    }

    #[test]
    fn edges_away_from_corners() {
        assert_eq!(edge_at(window(), 100.0, 1.0), Some(ResizeEdge::Top));
        assert_eq!(edge_at(window(), 100.0, 99.0), Some(ResizeEdge::Bottom));
        assert_eq!(edge_at(window(), 1.0, 50.0), Some(ResizeEdge::Left));
        assert_eq!(edge_at(window(), 199.0, 50.0), Some(ResizeEdge::Right));
    }

    #[test]
    fn the_middle_is_content() {
        assert_eq!(edge_at(window(), 100.0, 50.0), None);
        // Just inside the grab margin is already content.
        assert_eq!(edge_at(window(), GRAB + 0.5, 50.0), None);
    }

    #[test]
    fn outside_is_nothing() {
        assert_eq!(edge_at(window(), -1.0, 50.0), None);
        assert_eq!(edge_at(window(), 201.0, 50.0), None);
    }
}
