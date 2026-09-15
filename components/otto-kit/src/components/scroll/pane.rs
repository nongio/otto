//! One scrolling viewport, and the gestures that drive a set of them.
//!
//! [`ScrollPane`] is what an application holds per scrolling thing: the
//! physics ([`ScrollView`]), the surfaces the compositor moves
//! ([`ScrollSurfaces`]), and the bookkeeping between them — when the content
//! changed, when the band has to be repainted, how a point on the window maps
//! into the content. The application supplies what to draw through
//! [`ScrollContent`] and calls [`ScrollPane::update`] once per pass of its
//! update loop.
//!
//! Input stays with the host's window: a pane's surfaces pass the pointer
//! through, so the window's handler sees every event in its own coordinates
//! and converts with [`ScrollPane::parent_to_content`]. Wheel and touchpad
//! gestures go through a [`ScrollGroup`], which decides which pane a gesture
//! belongs to and keeps it there until it ends.
//!
//! See `docs/developer/scroll-pane.md` for the rules this is built on.

use skia_safe::{Canvas, Color, Contains, Point, Rect};
use wayland_client::protocol::wl_surface::WlSurface;

use crate::surfaces::SurfaceError;
use crate::theme::Theme;

use super::backing::ScrollSurfaces;
use super::scroll::ScrollView;
use super::state::Axis;

/// What a pane shows. Implemented by the application.
pub trait ScrollContent {
    /// Extent along the pane's axis, in points, for `cross` — the pane's
    /// extent across it (a grid's rows depend on its width).
    fn length(&self, cross: f32) -> f32;

    /// Changes whenever [`Self::paint`] would draw something different: a
    /// selection, a rename, a thumbnail landing. The pane repaints its band
    /// when this moves, and never otherwise except to refill.
    fn revision(&self) -> u64;

    /// Paint `band` — a rect in content coordinates — on a transparent canvas
    /// already translated into content space. Called when the band is
    /// refilled or the revision moves, never per step of a scroll.
    fn paint(&self, canvas: &Canvas, band: Rect);
}

/// One scrolling viewport.
pub struct ScrollPane {
    view: ScrollView,
    surfaces: ScrollSurfaces,
    /// Where the pane sits, in its parent surface's coordinates.
    viewport: Rect,
    /// The content revision the band was last painted from.
    revision: Option<u64>,
    container: bool,
}

impl ScrollPane {
    /// A pane scrolling along `axis` under `parent`, occupying `viewport` in
    /// the parent's coordinates. Transparent, and passing the pointer through.
    pub fn new(parent: &WlSurface, viewport: Rect, axis: Axis) -> Result<Self, SurfaceError> {
        let mut surfaces = ScrollSurfaces::on_axis(parent, viewport, Color::TRANSPARENT, axis)?;
        surfaces.set_input_passthrough();
        Ok(Self::from_parts(surfaces, viewport, axis, false))
    }

    /// A pane that holds panes rather than painting content. The panes inside
    /// it are created with [`Self::band_surface`] as their parent and a
    /// viewport in this pane's content coordinates.
    pub fn container(parent: &WlSurface, viewport: Rect, axis: Axis) -> Result<Self, SurfaceError> {
        let mut surfaces = ScrollSurfaces::container(parent, viewport, axis)?;
        surfaces.set_input_passthrough();
        Ok(Self::from_parts(surfaces, viewport, axis, true))
    }

    fn from_parts(surfaces: ScrollSurfaces, viewport: Rect, axis: Axis, container: bool) -> Self {
        Self {
            view: ScrollView::on_axis(axis, Rect::from_wh(viewport.width(), viewport.height())),
            surfaces,
            viewport,
            revision: None,
            container,
        }
    }

    pub fn axis(&self) -> Axis {
        self.view.axis()
    }

    /// The parent for panes nested in a [`Self::container`].
    pub fn band_surface(&self) -> &WlSurface {
        self.surfaces.band_surface()
    }

    /// Stack this pane directly above `sibling`, another child of the same
    /// parent.
    pub fn place_above(&self, sibling: &WlSurface) {
        self.surfaces.place_above(sibling);
    }

    /// The pane's clip surface, for stacking a sibling above it.
    pub fn surface(&self) -> &WlSurface {
        self.surfaces.clip_surface()
    }

    /// The physics, for a host that reads them.
    pub fn view(&self) -> &ScrollView {
        &self.view
    }

    /// The physics, for a host that drives something the pane does not wrap —
    /// a thumb drag, a content drag.
    pub fn view_mut(&mut self) -> &mut ScrollView {
        &mut self.view
    }

    /// Where the pane sits, in its parent's coordinates.
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// Move or resize the pane. A move keeps the band; a resize refills it.
    pub fn set_viewport(&mut self, viewport: Rect) {
        if viewport == self.viewport {
            return;
        }
        self.viewport = viewport;
        self.view
            .set_viewport(Rect::from_wh(viewport.width(), viewport.height()));
        self.surfaces.set_viewport(viewport);
    }

    /// Take the pane out of sight, or bring it back. Returns whether that
    /// changed anything.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        self.surfaces.set_hidden(hidden)
    }

    /// Repaint the band on the next update even if the revision did not move.
    pub fn invalidate(&mut self) {
        self.revision = None;
        self.surfaces.invalidate();
    }

    pub fn offset(&self) -> f32 {
        self.view.offset()
    }

    /// Scroll to `offset`, dropping any fling.
    pub fn scroll_to(&mut self, offset: f32) -> bool {
        self.view.scroll_to(offset)
    }

    /// Scroll as little as possible so `lo..hi` along the axis, in content
    /// coordinates, is in view — the keyboard cursor's row, say. Returns
    /// whether the offset changed.
    pub fn reveal(&mut self, lo: f32, hi: f32) -> bool {
        let length = self.view.state.viewport_length();
        let offset = self.view.offset();
        let target = if lo < offset {
            lo
        } else if hi > offset + length {
            hi - length
        } else {
            return false;
        };
        self.view.scroll_to(target)
    }

    /// The part of the content on screen, in content coordinates.
    pub fn visible(&self) -> Rect {
        let offset = self.view.offset();
        let (w, h) = (self.viewport.width(), self.viewport.height());
        match self.axis() {
            Axis::Vertical => Rect::from_xywh(0.0, offset, w, h),
            Axis::Horizontal => Rect::from_xywh(offset, 0.0, w, h),
        }
    }

    /// Whether `point`, in the parent's coordinates, is over the pane.
    pub fn contains(&self, point: Point) -> bool {
        self.viewport.contains(point)
    }

    /// `point`, in the parent's coordinates, in the content's.
    pub fn parent_to_content(&self, point: Point) -> Point {
        let local = Point::new(point.x - self.viewport.left, point.y - self.viewport.top);
        match self.axis() {
            Axis::Vertical => Point::new(local.x, local.y + self.view.offset()),
            Axis::Horizontal => Point::new(local.x + self.view.offset(), local.y),
        }
    }

    /// `point`, in the content's coordinates, in the parent's — where to draw
    /// something over the content in the window (a rename field, a ring).
    pub fn content_to_parent(&self, point: Point) -> Point {
        let offset = self.view.offset();
        let local = match self.axis() {
            Axis::Vertical => Point::new(point.x, point.y - offset),
            Axis::Horizontal => Point::new(point.x - offset, point.y),
        };
        Point::new(local.x + self.viewport.left, local.y + self.viewport.top)
    }

    /// A content-coordinate rect, in the parent's coordinates.
    pub fn content_rect_to_parent(&self, rect: Rect) -> Rect {
        let origin = self.content_to_parent(Point::new(rect.left, rect.top));
        Rect::from_xywh(origin.x, origin.y, rect.width(), rect.height())
    }

    /// Whether the pane still has motion, a bounce, a scrollbar fade or a
    /// highlight slide to run: a host keeps calling [`Self::update`] while it
    /// does.
    pub fn is_animating(&self) -> bool {
        self.view.is_animating() || self.surfaces.highlight_animating()
    }

    /// Mark `rect`, in content coordinates, with a rounded wash under the
    /// content — the selected row, the cell under the pointer — or clear it
    /// with `None`. Moving it slides it there and repaints nothing.
    pub fn set_highlight(&mut self, rect: Option<Rect>, color: Color, radius: f32) {
        self.surfaces.set_highlight(rect, color, radius);
    }

    /// Bring the pane up to date with `content`: its length, its revision,
    /// one step of the physics, and the surfaces — a band repainted only on a
    /// refill or a revision change, otherwise only moved.
    ///
    /// Returns whether the pane has the scroll in hand: it sent something, is
    /// waiting on the last step to be presented, or still has motion to run.
    /// A host that gets `false` has nothing to repaint for this pane.
    pub fn update(&mut self, content: &dyn ScrollContent, theme: &Theme) -> bool {
        let cross = match self.axis() {
            Axis::Vertical => self.viewport.width(),
            Axis::Horizontal => self.viewport.height(),
        };
        self.view.set_content_length(content.length(cross));
        let revision = content.revision();
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.surfaces.invalidate();
        }
        self.step(theme, |canvas, band| content.paint(canvas, band))
    }

    /// [`Self::update`] for a [`Self::container`]: it paints nothing, so all it
    /// needs is its content's length.
    pub fn update_container(&mut self, length: f32, theme: &Theme) -> bool {
        debug_assert!(self.container, "update_container on a pane with content");
        self.view.set_content_length(length);
        self.step(theme, |_, _| {})
    }

    fn step(&mut self, theme: &Theme, paint: impl FnOnce(&Canvas, Rect)) -> bool {
        let animating = self.view.is_animating();
        // One step per presented frame: while the last move is still on its
        // way the physics are not advanced either, so the step taken when the
        // answer comes covers the whole interval in one go.
        if animating && !self.surfaces.waiting() {
            self.view.tick();
        }
        let sent = self.surfaces.sync(&self.view, theme, paint);
        sent || self.surfaces.waiting() || animating || self.surfaces.highlight_animating()
    }

    /// A wheel or touchpad delta along the pane's axis, in points. `discrete`
    /// for a notched wheel, `stop` when the fingers lift. Returns whether the
    /// offset changed.
    pub fn wheel(&mut self, delta: f32, discrete: bool, stop: bool) -> bool {
        if stop {
            self.view.on_wheel_end();
            true
        } else if discrete {
            self.view.on_wheel_discrete(delta)
        } else {
            self.view.on_wheel(delta)
        }
    }

    /// Catch an in-flight fling: a hand laid on the touchpad, a press.
    pub fn stop(&mut self) {
        self.view.stop();
    }

    /// A press at `point` in the parent's coordinates. Returns whether it
    /// landed on the scrollbar and started a thumb drag.
    pub fn pointer_down(&mut self, point: Point) -> bool {
        let local = self.local(point);
        self.view.on_pointer_down(local.x, local.y)
    }

    /// Pointer motion at `point` in the parent's coordinates: drags the thumb
    /// when one is held, otherwise tracks hovering the scrollbar. Returns
    /// whether anything changed.
    pub fn pointer_motion(&mut self, point: Point) -> bool {
        let local = self.local(point);
        self.view.on_pointer_drag(local.x, local.y) | self.view.on_pointer_move(local.x, local.y)
    }

    pub fn pointer_up(&mut self) {
        self.view.on_pointer_up();
    }

    pub fn pointer_leave(&mut self) {
        self.view.on_pointer_leave();
    }

    fn local(&self, point: Point) -> Point {
        Point::new(point.x - self.viewport.left, point.y - self.viewport.top)
    }
}

/// One wheel or touchpad event, as a host's pointer handler receives it.
#[derive(Debug, Clone, Copy, Default)]
pub struct AxisEvent {
    /// The pointer, in the coordinates the candidate panes' viewports use.
    pub pointer: Point,
    pub dx: f32,
    pub dy: f32,
    /// A notched wheel step rather than a continuous delta.
    pub discrete: bool,
    /// The fingers lifted: the gesture ends here.
    pub stop: bool,
}

/// Which pane a scroll gesture belongs to.
///
/// A gesture is routed to the pane under the pointer that scrolls along the
/// axis its first delta favours, and stays with that pane until it ends —
/// so a column stack pans or a column scrolls, never both by turns, and a
/// gesture that drifts over a neighbour keeps scrolling the pane it started
/// in.
#[derive(Debug, Default)]
pub struct ScrollGroup {
    /// The pane the running gesture belongs to, by its index in the slice the
    /// host passes, and the axis it was locked to.
    target: Option<(usize, Axis)>,
}

impl ScrollGroup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Route `event` to one of `panes`, listed innermost first (a column
    /// before the stack it sits in). Returns the index of the pane that took
    /// it, if any.
    pub fn axis(&mut self, event: AxisEvent, panes: &mut [&mut ScrollPane]) -> Option<usize> {
        if self.target.is_none() {
            let favoured = if event.dx.abs() > event.dy.abs() {
                Axis::Horizontal
            } else {
                Axis::Vertical
            };
            let under = |axis: Axis, panes: &[&mut ScrollPane]| {
                panes
                    .iter()
                    .position(|pane| pane.axis() == axis && pane.contains(event.pointer))
            };
            let fallback = match favoured {
                Axis::Vertical => Axis::Horizontal,
                Axis::Horizontal => Axis::Vertical,
            };
            self.target = under(favoured, panes)
                .map(|index| (index, favoured))
                .or_else(|| under(fallback, panes).map(|index| (index, fallback)));
        }
        let (index, axis) = self.target?;
        let pane = panes.get_mut(index)?;
        let delta = match axis {
            Axis::Vertical => event.dy,
            Axis::Horizontal => event.dx,
        };
        pane.wheel(delta, event.discrete, event.stop);
        if event.stop || event.discrete {
            // Nothing is in flight to keep a pane for: the next event picks
            // afresh.
            self.target = None;
        }
        Some(index)
    }

    /// A hand laid on the touchpad: every pane stops where it is, and the
    /// next gesture picks its pane afresh.
    pub fn hold(&mut self, panes: &mut [&mut ScrollPane]) {
        self.target = None;
        for pane in panes.iter_mut() {
            pane.stop();
        }
    }
}
