//! Telling a surface where it is on the desktop.
//!
//! A Wayland client is never told where its window sits, which is deliberate
//! and almost always right: a client that knew would start placing itself. It
//! is wrong for exactly one thing, and that thing is accessibility. An
//! assistive technology reads the desktop by position — it asks an application
//! what is at a screen coordinate, and it draws the focus highlight and moves
//! the magnifier from the rects the application hands back — so an application
//! answering in its own coordinates claims a rectangle belonging to whatever is
//! drawn at the same offset from the desktop's origin. In practice that is the
//! top-left corner of the screen: the dock, or the menu bar.
//!
//! So the compositor says. The answer is taken from the scene layer the
//! surface is actually drawn into rather than from window geometry, because
//! that is the one number that cannot disagree with what is on screen: it
//! already carries the decoration offset, the workspace scroll, and every
//! animation in flight.
//!
//! Sent by diffing rather than from the places that move a window, because
//! there are a dozen of those — mapping, dragging, maximising, a workspace
//! scrolling, an output moving, a mode change — and a new one is added every
//! few months. A sweep that compares against what was last sent cannot be
//! forgotten by the thirteenth.

use smithay::reexports::wayland_server::Resource;
use smithay::wayland::compositor::get_parent;

use crate::state::Backend;
use crate::Otto;

/// The interface version that first carried `desktop_frame`.
const DESKTOP_FRAME_SINCE: u32 = 4;

/// A change smaller than this is not worth a round trip. Half a physical pixel:
/// below it nothing an assistive technology draws would land differently.
const EPSILON: f32 = 0.5;

/// Send `desktop_frame` to every style surface whose rect has changed.
///
/// Called once per pass of the event loop, before the clients are flushed.
pub fn send_desktop_frames<BackendData: Backend + 'static>(state: &mut Otto<BackendData>) {
    let Otto {
        surfaces_style,
        surface_layers,
        ..
    } = state;

    for (surface_id, styles) in surfaces_style.iter_mut() {
        // Only a surface that is a window in its own right. A subsurface moves
        // with its parent and reports its accessibility against the parent's
        // origin, so telling it where it is would be an invitation to use the
        // wrong one of the two.
        let Some(first) = styles.first() else {
            continue;
        };
        if get_parent(&first.surface).is_some() {
            continue;
        }

        let Some(layer) = surface_layers.get(surface_id) else {
            continue;
        };
        let bounds = layer.render_bounds_transformed();
        // An unmapped surface has no rect yet. Saying it is at the origin with
        // no size would be worse than saying nothing: the origin is a real
        // place, and something is drawn there.
        if bounds.is_empty() {
            continue;
        }
        let frame = (bounds.left, bounds.top, bounds.width(), bounds.height());

        for style in styles.iter_mut() {
            if style.wl_style.version() < DESKTOP_FRAME_SINCE {
                continue;
            }
            if style
                .last_desktop_frame
                .is_some_and(|last| unchanged(last, frame))
            {
                continue;
            }
            style.last_desktop_frame = Some(frame);
            style.wl_style.desktop_frame(
                f64::from(frame.0),
                f64::from(frame.1),
                f64::from(frame.2),
                f64::from(frame.3),
            );
        }
    }
}

fn unchanged(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    (a.0 - b.0).abs() < EPSILON
        && (a.1 - b.1).abs() < EPSILON
        && (a.2 - b.2).abs() < EPSILON
        && (a.3 - b.3).abs() < EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sweep runs on every pass of the event loop, so the thing that keeps
    /// it from being a per-frame broadcast is this comparison.
    #[test]
    fn a_frame_that_has_not_moved_is_not_resent() {
        assert!(unchanged(
            (10.0, 20.0, 300.0, 200.0),
            (10.0, 20.0, 300.0, 200.0)
        ));
    }

    /// Animated positions land on fractional pixels and jitter in the last
    /// digits; a window that has come to rest must stop sending.
    #[test]
    fn a_sub_pixel_difference_is_not_a_move() {
        assert!(unchanged(
            (10.0, 20.0, 300.0, 200.0),
            (10.2, 19.9, 300.1, 200.0)
        ));
    }

    #[test]
    fn a_move_of_a_whole_pixel_is() {
        assert!(!unchanged(
            (10.0, 20.0, 300.0, 200.0),
            (11.0, 20.0, 300.0, 200.0)
        ));
        assert!(!unchanged(
            (10.0, 20.0, 300.0, 200.0),
            (10.0, 20.0, 300.0, 201.0)
        ));
    }
}
