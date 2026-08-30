//! Keyboard focus inside a window.
//!
//! A kit window already knows whether *it* is the focused window
//! (`specs/otto-kit-window-focus.md`); this is the level below — which control
//! inside it the keyboard is talking to. Nothing else in the toolkit could
//! provide it: widgets are drawn, not retained, so the traversal order has to
//! come from the same pass that draws them.
//!
//! An application declares its focusables in draw order, once per build:
//!
//! ```no_run
//! # use otto_kit::focus::{FocusId, FocusRing};
//! # use skia_safe::Rect;
//! # let mut ring = FocusRing::default();
//! # let save_bounds = Rect::new(0.0, 0.0, 10.0, 10.0);
//! ring.begin();
//! ring.add(FocusId::new("save"), save_bounds, true);
//! if ring.is_focused(FocusId::new("save")) {
//!     // draw the ring around it
//! }
//! ```
//!
//! Focus survives a rebuild as long as the same id comes back, which is what
//! makes a stable id — not a position — the thing to key on. It is also the
//! id an assistive technology is told about, so the accessible tree and the
//! keyboard agree on what is focused without either being derived from the
//! other.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use skia_safe::{Canvas, Color, Paint, RRect, Rect};

/// A control's identity, stable across rebuilds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FocusId(u64);

impl FocusId {
    /// Derives an id from a name. Two controls in the same window must not
    /// share a name — a list makes its rows unique by index, `row-3`.
    pub fn new(key: impl AsRef<str>) -> Self {
        let mut hasher = DefaultHasher::new();
        key.as_ref().hash(&mut hasher);
        Self(hasher.finish())
    }

    /// An id from a number chosen by hand, for a control that is a fixed part
    /// of a window rather than one of a list. Usable in a `const`, which
    /// [`FocusId::new`] is not.
    ///
    /// Pick something distinctive: the value shares its space with hashed ids,
    /// so a small number is no safer than a large one, and a collision means
    /// two controls the keyboard cannot tell apart.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for handing to something that keys on a number — an
    /// accessible tree's node ids, for instance.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// One entry in the traversal order.
#[derive(Debug, Clone, Copy)]
pub struct Focusable {
    pub id: FocusId,
    /// Window-local, in points: where to draw the ring, and what an assistive
    /// technology is told the control occupies.
    pub bounds: Rect,
    /// A disabled control keeps its place in the order but is skipped.
    pub enabled: bool,
}

/// Where to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMove {
    Next,
    Previous,
    First,
    Last,
}

/// The focusables of one window, in traversal order, and which of them has the
/// keyboard.
#[derive(Debug, Default, Clone)]
pub struct FocusRing {
    order: Vec<Focusable>,
    focused: Option<FocusId>,
}

impl FocusRing {
    /// Starts a rebuild: the order is emptied, the focused id is kept. A
    /// control that does not come back loses the focus at [`FocusRing::end`].
    pub fn begin(&mut self) {
        self.order.clear();
    }

    /// Ends a rebuild, dropping a focus that no longer belongs to anything.
    pub fn end(&mut self) {
        if self
            .focused
            .is_some_and(|id| !self.order.iter().any(|entry| entry.id == id))
        {
            self.focused = None;
        }
    }

    /// Adds a control at this point in the traversal order.
    pub fn add(&mut self, id: FocusId, bounds: Rect, enabled: bool) {
        self.order.push(Focusable {
            id,
            bounds,
            enabled,
        });
    }

    /// Everything declared this build, in order.
    pub fn entries(&self) -> &[Focusable] {
        &self.order
    }

    pub fn focused(&self) -> Option<FocusId> {
        self.focused
    }

    pub fn is_focused(&self, id: FocusId) -> bool {
        self.focused == Some(id)
    }

    /// Where the focused control is, if anything is focused and it was declared
    /// this build.
    pub fn focused_bounds(&self) -> Option<Rect> {
        let focused = self.focused?;
        self.order
            .iter()
            .find(|entry| entry.id == focused)
            .map(|entry| entry.bounds)
    }

    /// Focuses a control by id, if it is present and enabled. Returns whether
    /// the focus actually moved, so a caller knows whether to redraw.
    pub fn focus(&mut self, id: FocusId) -> bool {
        let focusable = self
            .order
            .iter()
            .any(|entry| entry.id == id && entry.enabled);
        if !focusable || self.focused == Some(id) {
            return false;
        }
        self.focused = Some(id);
        true
    }

    /// Gives up the focus entirely — the window still has the keyboard, but no
    /// control in it does.
    pub fn clear(&mut self) -> bool {
        self.focused.take().is_some()
    }

    /// Moves the focus, wrapping at either end. Returns the newly focused id,
    /// or `None` when there is nothing to focus at all.
    ///
    /// With nothing focused yet, `Next` starts at the first control and
    /// `Previous` at the last, so the first Tab into a window lands somewhere
    /// sensible from either direction.
    pub fn move_focus(&mut self, direction: FocusMove) -> Option<FocusId> {
        let enabled: Vec<FocusId> = self
            .order
            .iter()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.id)
            .collect();
        if enabled.is_empty() {
            self.focused = None;
            return None;
        }

        let next = match direction {
            FocusMove::First => enabled[0],
            FocusMove::Last => enabled[enabled.len() - 1],
            FocusMove::Next | FocusMove::Previous => {
                let current = self
                    .focused
                    .and_then(|id| enabled.iter().position(|entry| *entry == id));
                match (current, direction) {
                    (Some(index), FocusMove::Next) => enabled[(index + 1) % enabled.len()],
                    (Some(index), _) => enabled[(index + enabled.len() - 1) % enabled.len()],
                    (None, FocusMove::Next) => enabled[0],
                    (None, _) => enabled[enabled.len() - 1],
                }
            }
        };

        self.focused = Some(next);
        Some(next)
    }
}

/// The ring drawn around the focused control.
///
/// One routine for every widget, so focus looks the same wherever it lands —
/// and so a theme change moves all of it at once. Drawn *outside* `bounds`,
/// like a macOS focus ring, rather than inset over the control's own edge.
pub fn draw_focus_ring(canvas: &Canvas, bounds: Rect, corner_radius: f32) {
    const WIDTH: f32 = 3.0;
    const GAP: f32 = 1.0;

    let color = crate::accent::current_accent().unwrap_or(Color::from_argb(255, 0, 122, 255));

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(skia_safe::paint::Style::Stroke);
    paint.set_stroke_width(WIDTH);
    // Half the stroke sits either side of the path, so the ring clears the
    // control by GAP exactly.
    paint.set_color(color.with_a(0x99));

    let outset = GAP + WIDTH / 2.0;
    let outer = Rect::new(
        bounds.left - outset,
        bounds.top - outset,
        bounds.right + outset,
        bounds.bottom + outset,
    );
    let radius = corner_radius + outset;
    canvas.draw_rrect(RRect::new_rect_xy(outer, radius, radius), &paint);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::new(0.0, 0.0, 10.0, 10.0)
    }

    fn ring(controls: &[(&str, bool)]) -> FocusRing {
        let mut ring = FocusRing::default();
        ring.begin();
        for (key, enabled) in controls {
            ring.add(FocusId::new(key), bounds(), *enabled);
        }
        ring.end();
        ring
    }

    #[test]
    fn tab_walks_the_order_and_wraps() {
        let mut ring = ring(&[("a", true), ("b", true), ("c", true)]);

        assert_eq!(ring.move_focus(FocusMove::Next), Some(FocusId::new("a")));
        assert_eq!(ring.move_focus(FocusMove::Next), Some(FocusId::new("b")));
        assert_eq!(ring.move_focus(FocusMove::Next), Some(FocusId::new("c")));
        assert_eq!(ring.move_focus(FocusMove::Next), Some(FocusId::new("a")));
        assert_eq!(
            ring.move_focus(FocusMove::Previous),
            Some(FocusId::new("c"))
        );
    }

    #[test]
    fn backwards_into_an_unfocused_window_lands_on_the_last_control() {
        let mut ring = ring(&[("a", true), ("b", true)]);
        assert_eq!(
            ring.move_focus(FocusMove::Previous),
            Some(FocusId::new("b"))
        );
    }

    #[test]
    fn disabled_controls_are_skipped_but_keep_their_place() {
        let mut ring = ring(&[("a", true), ("b", false), ("c", true)]);
        assert_eq!(ring.move_focus(FocusMove::Next), Some(FocusId::new("a")));
        assert_eq!(ring.move_focus(FocusMove::Next), Some(FocusId::new("c")));
        assert!(!ring.focus(FocusId::new("b")));
    }

    #[test]
    fn nothing_focusable_focuses_nothing() {
        let mut ring = ring(&[("a", false)]);
        assert_eq!(ring.move_focus(FocusMove::Next), None);
        assert_eq!(ring.focused(), None);
    }

    #[test]
    fn focus_survives_a_rebuild_that_keeps_the_control() {
        let mut ring = ring(&[("a", true), ("b", true)]);
        ring.move_focus(FocusMove::Last);
        assert!(ring.is_focused(FocusId::new("b")));

        ring.begin();
        ring.add(FocusId::new("a"), bounds(), true);
        ring.add(FocusId::new("b"), bounds(), true);
        ring.end();
        assert!(ring.is_focused(FocusId::new("b")));
    }

    #[test]
    fn focus_is_dropped_when_its_control_goes_away() {
        let mut ring = ring(&[("a", true), ("b", true)]);
        ring.move_focus(FocusMove::Last);

        ring.begin();
        ring.add(FocusId::new("a"), bounds(), true);
        ring.end();
        assert_eq!(ring.focused(), None);
    }

    #[test]
    fn focused_bounds_follow_the_control() {
        let mut ring = FocusRing::default();
        ring.begin();
        ring.add(FocusId::new("a"), Rect::new(4.0, 8.0, 20.0, 30.0), true);
        ring.end();
        ring.move_focus(FocusMove::First);

        assert_eq!(ring.focused_bounds(), Some(Rect::new(4.0, 8.0, 20.0, 30.0)));
    }
}
