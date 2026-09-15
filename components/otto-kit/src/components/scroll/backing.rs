//! Wayland-backed scrolling: the content lives in its own subsurface and the
//! compositor does the cropping.
//!
//! A [`ScrollView`] on its own paints into whatever canvas it is given, which
//! means the host repaints its whole window on every frame of a scroll.
//! [`ScrollSurfaces`] takes that work off the client entirely: it puts the
//! content in a subsurface whose buffer is a *band* — longer than the viewport
//! along the scrolling axis — and scrolls it by moving that surface with
//! `otto_surface_style_v1`. The parent surface clips its children to the
//! viewport (`set_clip_children`), so nothing spills. Moving a surface is a
//! protocol request, not a paint, so a frame of scrolling costs no drawing, no
//! buffer and no upload; the client only paints when the scroll approaches the
//! edge of the rendered band and [`Band::refill`] asks for a new one.
//!
//! Three surfaces, because each is a thing that moves or clips independently:
//!
//! ```text
//! clip   — fixed at the viewport, paints the pane background, clips children
//!   band — the content, longer than the viewport, moved to scroll
//!   thumb — the scrollbar, above the band, moved and faded by the compositor
//! ```
//!
//! Either axis: a vertical pane moves its band up and down and keeps its
//! scrollbar on the right edge; a horizontal one moves it left and right with
//! the bar along the bottom.
//!
//! A pane can hold other panes instead of painting content: see
//! [`ScrollSurfaces::container`]. Its band spans the whole content and carries
//! no pixels of its own, and the panes inside are children of
//! [`ScrollSurfaces::band_surface`] placed in content coordinates — so
//! scrolling the container moves one surface, and everything inside it rides
//! along without being touched.
//!
//! Two compositor behaviours this depends on, both easy to get wrong:
//!
//! - A surface's layer position and size are re-derived from the surface tree
//!   on every commit *until the client claims them*, and it claims them by
//!   setting a size. So every surface here sets its style size before its
//!   style position means anything.
//! - Style geometry is in buffer (physical) pixels while subsurface geometry
//!   is in logical points, so everything crossing into the style protocol is
//!   multiplied by the scale.
//!
//! The pointer is hit-tested against the *subsurface* position, not the style
//! position, so the band keeps both in step — the style one exact for smooth
//! movement, the subsurface one rounded, which is all the pointer needs. The
//! thumb does not: it is decoration, and it takes no input at all. Its surface
//! carries an empty input region, so presses fall through to the band beneath
//! it and a host hit-tests the scrollbar the way it always did, against
//! [`ScrollRenderer::thumb_rect`] in pane coordinates.
//!
//! A host that hit-tests everything in its window's own coordinates can opt
//! out of pointer input on the pane altogether with
//! [`ScrollSurfaces::set_input_passthrough`]: every event over the pane then
//! reaches the window as though the pane were painted into it.

use std::time::{Duration, Instant};

use skia_safe::{Canvas, Color, Rect};
use wayland_client::protocol::wl_surface::WlSurface;

use crate::app_runner::AppContext;
use crate::protocols::otto_surface_style_v1::ClipMode;
use crate::surfaces::{SubsurfaceSurface, SurfaceError};
use crate::theme::Theme;

use super::band::{Band, BandView};
use super::renderer::ScrollRenderer;
use super::scroll::ScrollView;
use super::state::{Axis, ScrollState};

/// Thickness of the strip the scrollbar surface occupies, in points. Wide
/// enough for the thumb at its expanded thickness plus its margin.
const THUMB_STRIP: f32 = 16.0;

/// How far the thumb's length may be stretched from the length its buffer was
/// painted at, as a fraction, before it is painted again.
const THUMB_STRETCH: f32 = 0.25;

/// How long the highlight takes to slide onto a new item.
const HIGHLIGHT_SLIDE: Duration = Duration::from_millis(110);

/// The surfaces behind a [`ScrollView`], and the band currently painted into
/// them.
pub struct ScrollSurfaces {
    clip: SubsurfaceSurface,
    band_surface: SubsurfaceSurface,
    thumb: SubsurfaceSurface,
    axis: Axis,
    /// Holds panes rather than painting content; see [`Self::container`].
    container: bool,
    /// What the band surface's buffer currently holds.
    band: Band,
    /// Viewport in the parent surface's coordinates.
    viewport: Rect,
    /// The scale the clip's style geometry was last pushed with. The preferred
    /// scale arrives after these surfaces are built, so the first push is
    /// always the integer fallback and has to be redone once the real one
    /// lands.
    configured_scale: f32,
    /// Background painted into the clip surface, repainted only when it or the
    /// viewport changes.
    background: Color,
    /// Thumb thickness and length its buffer was last painted at, so it is
    /// only redrawn when it changes shape rather than every time it moves.
    thumb_size: (f32, f32),
    /// Last values pushed to the compositor, to skip redundant requests.
    last_band_offset: Option<f32>,
    /// Band-local start of the input region last pushed, in points.
    last_input_offset: Option<i32>,
    last_thumb_offset: Option<f32>,
    last_opacity: Option<f32>,
    /// The pane takes no pointer input; see [`Self::set_input_passthrough`].
    passthrough: bool,
    hidden: bool,
    /// The last move or paint has not reached the screen yet, so this sync
    /// was held back; see [`Self::waiting`].
    waiting: bool,
    /// The thumb length last claimed through the style, which may differ from
    /// the length its buffer was painted at while an overscroll squashes it.
    last_thumb_length: Option<f32>,
    /// The selection wash under the content; see [`Self::set_highlight`].
    highlight: Option<Highlight>,
}

/// A rounded wash under the content marking one item: a pixel of colour the
/// compositor stretches and rounds, sitting between the pane's ground and its
/// band, so moving the selection repaints nothing.
struct Highlight {
    surface: SubsurfaceSurface,
    /// Where it is going, in content coordinates.
    target: Rect,
    /// Where the slide towards `target` started, and when.
    from: Rect,
    started: Instant,
    color: Option<Color>,
    radius: f32,
    hidden: bool,
    /// The pane-local rect last sent, `None` when it has to be sent again.
    sent: Option<Rect>,
}

/// Where a slide from `from` to `to` has got to after `elapsed`, eased out.
fn slide(from: Rect, to: Rect, elapsed: Duration) -> Rect {
    let t = (elapsed.as_secs_f32() / HIGHLIGHT_SLIDE.as_secs_f32()).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    let lerp = |a: f32, b: f32| a + (b - a) * eased;
    Rect::from_ltrb(
        lerp(from.left, to.left),
        lerp(from.top, to.top),
        lerp(from.right, to.right),
        lerp(from.bottom, to.bottom),
    )
}

impl ScrollSurfaces {
    /// The output's fractional scale, read fresh every time.
    ///
    /// Not cached: `wp_fractional_scale_v1` delivers the preferred scale
    /// asynchronously, and these surfaces are usually built before the first
    /// event arrives — a value snapshotted in the constructor is the integer
    /// fallback, and stays wrong for the life of the pane.
    fn scale(&self) -> f32 {
        AppContext::fractional_scale() as f32
    }

    /// `points` in the style protocol's physical pixels, on the pixel grid.
    ///
    /// A band left at a fraction of a pixel has every one of its texels
    /// resampled by the compositor on every step of a scroll — filtered, and
    /// soft. On whole pixels the buffer lands 1:1 on the screen: exact, and
    /// the cheapest thing to draw. Half a pixel of placement is invisible at
    /// the display's rate.
    fn px(&self, points: f32) -> f64 {
        (points * self.scale()).round() as f64
    }

    /// A vertical pane under `parent`, occupying `viewport` in the parent's
    /// coordinate space.
    pub fn new(
        parent: &WlSurface,
        viewport: Rect,
        background: Color,
    ) -> Result<Self, SurfaceError> {
        Self::on_axis(parent, viewport, background, Axis::Vertical)
    }

    /// A pane scrolling along `axis`, under `parent`, occupying `viewport` in
    /// the parent's coordinate space.
    pub fn on_axis(
        parent: &WlSurface,
        viewport: Rect,
        background: Color,
        axis: Axis,
    ) -> Result<Self, SurfaceError> {
        let clip = SubsurfaceSurface::new(
            parent,
            viewport.left as i32,
            viewport.top as i32,
            viewport.width() as i32,
            viewport.height() as i32,
        )?;
        let band_surface = SubsurfaceSurface::new(
            clip.wl_surface(),
            0,
            0,
            viewport.width() as i32,
            viewport.height() as i32,
        )?;
        let thumb = match axis {
            Axis::Vertical => SubsurfaceSurface::new(
                clip.wl_surface(),
                (viewport.width() - THUMB_STRIP) as i32,
                0,
                THUMB_STRIP as i32,
                1,
            )?,
            Axis::Horizontal => SubsurfaceSurface::new(
                clip.wl_surface(),
                0,
                (viewport.height() - THUMB_STRIP) as i32,
                1,
                THUMB_STRIP as i32,
            )?,
        };
        thumb.place_above(band_surface.wl_surface());
        // The thumb is painted, not touched: an empty input region lets every
        // press through to the band, which keeps scrollbar hit-testing a
        // question about pane coordinates rather than about which surface the
        // pointer happened to land on.
        set_empty_input_region(thumb.wl_surface());
        thumb.commit();

        let mut surfaces = Self {
            clip,
            band_surface,
            thumb,
            axis,
            container: false,
            band: Band::empty(),
            viewport,
            configured_scale: 0.0,
            background,
            thumb_size: (0.0, 0.0),
            last_band_offset: None,
            last_input_offset: None,
            last_thumb_offset: None,
            last_opacity: None,
            passthrough: false,
            hidden: false,
            waiting: false,
            last_thumb_length: None,
            highlight: None,
        };
        surfaces.configure_clip();
        Ok(surfaces)
    }

    /// A pane that holds other panes rather than painting content.
    ///
    /// Its band spans the whole content, with a transparent buffer stretched
    /// to that size rather than one painted at it, and is never refilled.
    /// Panes placed inside it are created with [`Self::band_surface`] as their
    /// parent and a viewport in content coordinates; scrolling the container
    /// moves its band, and they ride along without a request of their own.
    pub fn container(parent: &WlSurface, viewport: Rect, axis: Axis) -> Result<Self, SurfaceError> {
        let mut surfaces = Self::on_axis(parent, viewport, Color::TRANSPARENT, axis)?;
        surfaces.container = true;
        Ok(surfaces)
    }

    /// Let every pointer event over the pane fall through to the parent.
    ///
    /// For a host that already hit-tests its content in its window's own
    /// coordinates: the pane then changes how the content is presented and
    /// nothing about how it is pointed at. Without it, events over the pane
    /// arrive on the clip and band surfaces instead, in their local
    /// coordinates, and a window-level pointer handler never sees them.
    pub fn set_input_passthrough(&mut self) {
        if self.passthrough {
            return;
        }
        self.passthrough = true;
        set_empty_input_region(self.clip.wl_surface());
        set_empty_input_region(self.band_surface.wl_surface());
        self.band_surface.commit();
        // Nothing points at the clip now, so its buffer need not cover the
        // pane: one pixel of the background, stretched to the viewport by its
        // style size, is the same solid ground at none of the cost — a buffer
        // the size of the pane is sampled under every step of the scroll.
        self.clip.resize(1, 1);
        self.configure_clip();
    }

    /// Whether the last [`Self::sync`] was held back because the previous step
    /// had not reached the screen. The compositor's answer wakes the host, and
    /// the next sync takes the step; a host deciding whether the scroll was
    /// dealt with should count this as dealt with.
    pub fn waiting(&self) -> bool {
        self.waiting
    }

    /// The axis this pane scrolls along.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// Stack the pane directly above `sibling`, another child of the same
    /// parent. Parent state: it lands with the parent's next commit.
    pub fn place_above(&self, sibling: &WlSurface) {
        self.clip.place_above(sibling);
    }

    /// The content-space coordinate, along the axis, of the start of the
    /// painted band. A host translating pointer events that land on the
    /// content surface adds this to the surface-local coordinate to get
    /// content coordinates.
    pub fn band_origin(&self) -> f32 {
        self.band.origin()
    }

    /// The surface carrying the content, for a host that needs to recognise
    /// pointer events arriving on it.
    pub fn content_surface(&self) -> &WlSurface {
        self.band_surface.wl_surface()
    }

    /// The parent for panes nested inside this one. Only meaningful for a
    /// [`Self::container`], whose band spans the content: a child placed at a
    /// content coordinate stays there however the container is scrolled.
    pub fn band_surface(&self) -> &WlSurface {
        self.band_surface.wl_surface()
    }

    /// The clip box the content and scrollbar sit inside.
    ///
    /// A host has to be able to recognise events on this one too: the band is
    /// only as long as there is content, so wherever it falls short of the
    /// viewport the clip is what the pointer is over. Its local coordinates
    /// are the pane's own, since it *is* the viewport — unlike the band, which
    /// moves under it to scroll.
    ///
    /// The scrollbar is deliberately not exposed: it has an empty input
    /// region and never receives a pointer event.
    pub fn clip_surface(&self) -> &WlSurface {
        self.clip.wl_surface()
    }

    /// Drop the painted band so the next [`Self::sync`] repaints it.
    ///
    /// The band is normally only repainted when a scroll runs off its edge —
    /// which is exactly the point — so anything else that changes what the
    /// content looks like has to say so: a toggle flipped, a slider dragged, a
    /// value arriving from elsewhere. Cheap to call spuriously; it costs one
    /// band paint.
    pub fn invalidate(&mut self) {
        self.band = Band::empty();
    }

    /// Change the pane background and repaint it. The background lives in the
    /// clip surface, which is painted once and then left alone, so a theme
    /// change has to be pushed in — otherwise the pane's ground keeps the old
    /// scheme while the content around it switches.
    pub fn set_background(&mut self, background: Color) {
        if background == self.background {
            return;
        }
        self.background = background;
        self.configure_clip();
    }

    /// Move or resize the pane.
    ///
    /// A move alone keeps everything painted: the band and the scrollbar ride
    /// inside the clip, so moving the clip moves them too. A resize repaints
    /// the background and, on the next [`Self::sync`], the band.
    pub fn set_viewport(&mut self, viewport: Rect) {
        if viewport == self.viewport {
            return;
        }
        let resized = viewport.width() != self.viewport.width()
            || viewport.height() != self.viewport.height();
        self.viewport = viewport;
        self.clip
            .set_position(viewport.left as i32, viewport.top as i32);
        if !resized {
            if let Some(style) = self.clip.layer() {
                style.set_position(self.px(viewport.left), self.px(viewport.top));
            }
            self.clip.commit();
            return;
        }
        if !self.passthrough {
            self.clip
                .resize(viewport.width() as i32, viewport.height() as i32);
        }
        self.band = Band::empty();
        self.last_band_offset = None;
        self.last_input_offset = None;
        self.last_thumb_offset = None;
        self.configure_clip();
    }

    /// Take the pane out of sight, or bring it back. The band and the
    /// scrollbar are children of the clip, so they go with it.
    ///
    /// Returns whether anything changed.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        if let Some(style) = self.clip.layer() {
            style.set_opacity(if hidden { 0.0 } else { 1.0 });
        }
        self.clip.commit();
        true
    }

    /// Mark `rect`, in content coordinates, with a rounded wash of `color`
    /// under the content, or take the mark away with `None`.
    ///
    /// The wash is a surface of its own between the pane's ground and its
    /// band: moving it — to follow the keyboard or the pointer — slides it
    /// there over [`HIGHLIGHT_SLIDE`] and repaints nothing, and a scroll moves
    /// it with the content. Takes effect on the next [`Self::sync`]; a host
    /// keeps syncing while [`Self::highlight_animating`].
    pub fn set_highlight(&mut self, rect: Option<Rect>, color: Color, radius: f32) {
        let Some(rect) = rect else {
            if let Some(highlight) = self.highlight.as_mut().filter(|h| !h.hidden) {
                highlight.hidden = true;
                if let Some(style) = highlight.surface.layer() {
                    style.set_opacity(0.0);
                }
                highlight.surface.commit();
            }
            return;
        };
        if self.highlight.is_none() {
            let Ok(surface) = SubsurfaceSurface::new(self.clip.wl_surface(), 0, 0, 1, 1) else {
                return;
            };
            set_empty_input_region(surface.wl_surface());
            surface.place_below(self.band_surface.wl_surface());
            // The order is the clip's pending state.
            self.clip.commit();
            self.highlight = Some(Highlight {
                surface,
                target: rect,
                from: rect,
                started: Instant::now(),
                color: None,
                radius: -1.0,
                hidden: true,
                sent: None,
            });
        }
        let Some(highlight) = self.highlight.as_mut() else {
            return;
        };
        if highlight.color != Some(color) {
            highlight.color = Some(color);
            highlight.surface.draw(|canvas| {
                canvas.clear(color);
            });
        }
        if highlight.radius != radius {
            highlight.radius = radius;
            if let Some(style) = highlight.surface.layer() {
                style.set_corner_radius(radius as f64);
            }
        }
        let now = Instant::now();
        if highlight.hidden {
            // Nothing on screen to slide from: it appears where it belongs.
            highlight.hidden = false;
            highlight.from = rect;
            highlight.sent = None;
        } else if highlight.target != rect {
            highlight.from = slide(
                highlight.from,
                highlight.target,
                now - highlight.started,
            );
        }
        if highlight.target != rect {
            highlight.target = rect;
            highlight.started = now;
        }
    }

    /// Whether the highlight is still sliding.
    pub fn highlight_animating(&self) -> bool {
        self.highlight.as_ref().is_some_and(|highlight| {
            !highlight.hidden
                && highlight.from != highlight.target
                && highlight.started.elapsed() < HIGHLIGHT_SLIDE
        })
    }

    /// Put the highlight where its slide has got to, moved with the content.
    fn position_highlight(&mut self, offset: f32) -> bool {
        let (axis, scale) = (self.axis, self.scale());
        let Some(highlight) = self.highlight.as_mut().filter(|h| !h.hidden) else {
            return false;
        };
        let rect = slide(
            highlight.from,
            highlight.target,
            highlight.started.elapsed(),
        );
        let local = match axis {
            Axis::Vertical => rect.with_offset((0.0, -offset)),
            Axis::Horizontal => rect.with_offset((-offset, 0.0)),
        };
        if highlight.sent == Some(local) {
            return false;
        }
        let appearing = highlight.sent.is_none();
        highlight.sent = Some(local);
        // Style geometry only: the wash takes no input, so where the pointer
        // would find it does not matter.
        if let Some(style) = highlight.surface.layer() {
            let px = |points: f32| (points * scale).round() as f64;
            style.set_size(px(local.width()), px(local.height()));
            style.set_position(px(local.left), px(local.top));
            if appearing {
                style.set_opacity(1.0);
            }
        }
        highlight.surface.commit();
        true
    }

    /// Bring the surfaces in line with the view: repaint the band if the scroll
    /// has reached its margin, then move it and the scrollbar to where the
    /// current offset puts them.
    ///
    /// `content` paints in content coordinates and is given the band's rect —
    /// the same contract as [`ScrollRenderer::draw`]'s closure, except it is
    /// called only on the rare frame that needs a new band. A container never
    /// calls it.
    ///
    /// Returns whether anything was sent to the compositor: a band painted, or
    /// the band or the scrollbar moved.
    pub fn sync<F>(&mut self, view: &ScrollView, theme: &Theme, content: F) -> bool
    where
        F: FnOnce(&Canvas, Rect),
    {
        self.sync_state(&view.state, view.velocity(), theme, content)
    }

    /// [`Self::sync`] for a host that holds the scroll's state and speed
    /// rather than the [`ScrollView`] itself — a frame snapshot, say.
    pub fn sync_state<F>(
        &mut self,
        state: &ScrollState,
        velocity: f32,
        theme: &Theme,
        content: F,
    ) -> bool
    where
        F: FnOnce(&Canvas, Rect),
    {
        // One step per presented frame. A move asks to hear when it reaches
        // the screen; until it has, a further step would only be a position
        // the compositor never shows, and a host loop woken by anything else
        // would spin through them.
        {
            use wayland_client::Proxy;
            self.waiting = AppContext::frame_in_flight(&self.band_surface.wl_surface().id());
            if self.waiting {
                return false;
            }
        }

        // `wp_fractional_scale_v1` reports the output's scale asynchronously,
        // so the geometry pushed when the pane was built used the integer
        // fallback. Re-push everything that scaled by it — otherwise the pane
        // keeps the position and size of a 2x output on a 1.25x one, sitting
        // too far right and reaching past the window.
        if self.configured_scale != self.scale() {
            self.configure_clip();
            self.band = Band::empty();
            self.last_band_offset = None;
            self.last_thumb_offset = None;
        }

        debug_assert_eq!(
            state.axis(),
            self.axis,
            "a pane's scroll state must scroll along the pane's axis"
        );

        let mut changed = false;
        if self.container {
            // The whole content, always: nothing to refill, only a length that
            // changes when the content does.
            let whole = Band::new(0.0, state.content_length());
            if whole != self.band {
                self.band = whole;
                self.size_container_band();
                changed = true;
            }
        } else {
            let band_view = BandView {
                offset: state.offset(),
                viewport_length: self.axis.length(self.viewport),
                content_length: state.content_length(),
                velocity,
            };
            if let Some(next) = self.band.refill(&band_view) {
                self.band = next;
                self.paint_band(content);
                changed = true;
            }
        }

        changed |= self.position_band(state.offset());
        changed |= self.position_highlight(state.offset());
        changed |= self.position_thumb(state, theme);
        changed
    }

    /// The pane's extent across its axis.
    fn cross_extent(&self) -> f32 {
        match self.axis {
            Axis::Vertical => self.viewport.width(),
            Axis::Horizontal => self.viewport.height(),
        }
    }

    /// `(width, height)` of something `length` long along the axis and
    /// `cross` across it.
    fn oriented(&self, length: f32, cross: f32) -> (f32, f32) {
        match self.axis {
            Axis::Vertical => (cross, length),
            Axis::Horizontal => (length, cross),
        }
    }

    /// The clip box: claims its bounds so the compositor stops re-deriving
    /// them, clips whatever moves inside it, and paints the pane background.
    fn configure_clip(&mut self) {
        self.configured_scale = self.scale();
        if let Some(style) = self.clip.layer() {
            style.set_size(
                self.px(self.viewport.width()),
                self.px(self.viewport.height()),
            );
            // Claiming the size stops the compositor deriving *both* size and
            // position from the surface tree, so the position the subsurface
            // was created with no longer reaches the layer — without this the
            // pane is drawn at the window's origin, on top of the chrome.
            style.set_position(self.px(self.viewport.left), self.px(self.viewport.top));
            style.set_clip_children(ClipMode::Enabled);
        }
        let background = self.background;
        self.clip.draw(|canvas| {
            canvas.clear(background);
        });
    }

    /// Paint the current band into the content surface, resizing its buffer to
    /// match. The canvas is translated so the closure draws in content space.
    fn paint_band<F>(&mut self, content: F)
    where
        F: FnOnce(&Canvas, Rect),
    {
        let (width, height) = self.oriented(self.band.length(), self.cross_extent());
        self.band_surface.resize(width as i32, height as i32);
        if let Some(style) = self.band_surface.layer() {
            style.set_size(self.px(width), self.px(height));
        }

        let rect = self.band.rect(self.axis, 0.0, self.cross_extent());
        let shift = match self.axis {
            Axis::Vertical => (0.0, -self.band.origin()),
            Axis::Horizontal => (-self.band.origin(), 0.0),
        };
        self.band_surface.draw(|canvas| {
            canvas.clear(Color::TRANSPARENT);
            canvas.save();
            canvas.translate(shift);
            content(canvas, rect);
            canvas.restore();
        });
        // A fresh buffer starts at the surface's own origin; wherever it was
        // standing before means nothing now.
        self.last_band_offset = None;
    }

    /// A container's band: the whole content, as a transparent pixel the
    /// compositor stretches. It has to have a buffer — a surface without one
    /// is unmapped, and so are the panes inside it — but painting a buffer
    /// the size of the content would cost the memory of every column at once
    /// for pixels nobody sees.
    fn size_container_band(&mut self) {
        let (width, height) = self.oriented(self.band.length(), self.cross_extent());
        self.band_surface.resize(1, 1);
        if let Some(style) = self.band_surface.layer() {
            style.set_size(
                (width.max(1.0) * self.scale()) as f64,
                (height.max(1.0) * self.scale()) as f64,
            );
        }
        self.band_surface.draw(|canvas| {
            canvas.clear(Color::TRANSPARENT);
        });
        self.last_band_offset = None;
    }

    /// Move the band to where this offset puts it. This is the whole cost of a
    /// frame of scrolling. Returns whether it moved.
    fn position_band(&mut self, offset: f32) -> bool {
        let along = self.band.surface_offset(offset);
        if self.last_band_offset == Some(along) {
            return false;
        }
        self.last_band_offset = Some(along);

        let (x, y) = self.oriented(along, 0.0);
        if let Some(style) = self.band_surface.layer() {
            style.set_position(self.px(x), self.px(y));
        }
        // The pointer is hit-tested against the subsurface position, so it has
        // to follow — rounded, which is under a point out and invisible to a
        // hit test.
        let rounded = along.round() as i32;
        let (sx, sy) = match self.axis {
            Axis::Vertical => (0, rounded),
            Axis::Horizontal => (rounded, 0),
        };
        self.band_surface.set_position(sx, sy);
        self.clip_band_input(rounded);
        // A move is not a paint, so nothing else asks to hear when it reaches
        // the screen — and that answer is what paces the next step of a
        // glide. Without it a host whose window has stopped repainting has
        // nothing to wake it at the display's rate.
        AppContext::request_throttled_frame(self.band_surface.wl_surface());
        self.band_surface.commit();
        true
    }

    /// Cut the band's input region down to the slice of it the viewport shows.
    ///
    /// The band is longer than the viewport and hangs out of the clip surface
    /// at both ends — by up to [`MIN_OVERDRAW`](super::band) points, which on a
    /// pane that reaches the window's edge is a strip of live surface hanging
    /// outside the window itself. `set_clip_children` crops what is *drawn*;
    /// the pointer knows nothing about it, and a surface with no input region
    /// of its own takes input over the whole buffer. So the band carries a
    /// region covering exactly the part of it inside the clip, moved with it:
    /// without this the window swallows clicks past its own edge, and the
    /// content beside the pane gets events meant for the chrome.
    fn clip_band_input(&mut self, along: i32) {
        if self.passthrough || self.last_input_offset == Some(along) {
            return;
        }
        self.last_input_offset = Some(along);

        let compositor = AppContext::compositor_state();
        let qh = AppContext::queue_handle();
        let region = compositor.wl_compositor().create_region(qh, ());
        // Band-local: the viewport's leading edge sits at `-along` in the
        // band's own coordinates. Rounded outwards so no row along either edge
        // is dead.
        let (x, y) = match self.axis {
            Axis::Vertical => (0, -along),
            Axis::Horizontal => (-along, 0),
        };
        region.add(
            x,
            y,
            self.viewport.width().ceil() as i32,
            self.viewport.height().ceil() as i32,
        );
        self.band_surface
            .wl_surface()
            .set_input_region(Some(&region));
        region.destroy();
    }

    /// The scrollbar moves and fades entirely through its style node; it is
    /// only repainted when the thumb changes shape, which happens when the
    /// content's length changes or the pointer expands it — not while
    /// scrolling. Returns whether anything was sent.
    fn position_thumb(&mut self, state: &ScrollState, theme: &Theme) -> bool {
        let Some(rect) = ScrollRenderer::thumb_rect(state) else {
            return self.hide_thumb();
        };
        let opacity = state.scrollbar_opacity();
        if opacity <= 0.0 {
            return self.hide_thumb();
        }

        let (thickness, length) = match self.axis {
            Axis::Vertical => (rect.width(), rect.height()),
            Axis::Horizontal => (rect.height(), rect.width()),
        };
        let thickness = thickness.round();
        let length = length.round().max(1.0);

        let mut changed = false;
        // The thumb squashes continuously while an overscroll bounces. Its
        // buffer is stretched to follow — the style size scales what is
        // already there — and only painted again when the thickness changes
        // or the length has drifted far enough that the stretch would show in
        // the rounded ends.
        let (painted_thickness, painted_length) = self.thumb_size;
        let repaint = thickness != painted_thickness
            || painted_length <= 0.0
            || (length / painted_length - 1.0).abs() > THUMB_STRETCH;
        if repaint {
            self.thumb_size = (thickness, length);
            let (width, height) = self.oriented(length, THUMB_STRIP);
            self.thumb.resize(width as i32, height as i32);
            let color = theme.fill_secondary;
            let pill = match self.axis {
                Axis::Vertical => Rect::from_xywh(THUMB_STRIP - thickness, 0.0, thickness, length),
                Axis::Horizontal => {
                    Rect::from_xywh(0.0, THUMB_STRIP - thickness, length, thickness)
                }
            };
            self.thumb.draw(|canvas| {
                canvas.clear(Color::TRANSPARENT);
                let mut paint = skia_safe::Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(color);
                let radius = thickness / 2.0;
                canvas.draw_rrect(skia_safe::RRect::new_rect_xy(pill, radius, radius), &paint);
            });
            self.last_thumb_offset = None;
            self.last_thumb_length = None;
            changed = true;
        }
        if self.last_thumb_length != Some(length) {
            self.last_thumb_length = Some(length);
            let (width, height) = self.oriented(length, THUMB_STRIP);
            if let Some(style) = self.thumb.layer() {
                style.set_size(
                    (width * self.scale()) as f64,
                    (height * self.scale()) as f64,
                );
            }
            changed = true;
        }

        // The thumb is placed in the view's own coordinates; the clip is the
        // viewport, so the thumb's place inside it is measured from the
        // viewport's leading edge rather than from wherever the view sits.
        let along = match self.axis {
            Axis::Vertical => rect.top - state.viewport().top,
            Axis::Horizontal => rect.left - state.viewport().left,
        };
        if let Some(style) = self.thumb.layer() {
            if self.last_thumb_offset != Some(along) {
                self.last_thumb_offset = Some(along);
                let (x, y) = match self.axis {
                    Axis::Vertical => (self.viewport.width() - THUMB_STRIP, along),
                    Axis::Horizontal => (along, self.viewport.height() - THUMB_STRIP),
                };
                style.set_position(self.px(x), self.px(y));
                changed = true;
            }
            if self.last_opacity != Some(opacity) {
                self.last_opacity = Some(opacity);
                style.set_opacity(opacity as f64);
                changed = true;
            }
        }
        if changed {
            self.thumb.commit();
        }
        changed
    }

    fn hide_thumb(&mut self) -> bool {
        if self.last_opacity == Some(0.0) {
            return false;
        }
        self.last_opacity = Some(0.0);
        if let Some(style) = self.thumb.layer() {
            style.set_opacity(0.0);
        }
        self.thumb.commit();
        true
    }
}

/// Give `surface` an empty input region, so the pointer passes through it.
/// The compositor copies the region on `set_input_region`, so it is destroyed
/// straight away.
fn set_empty_input_region(surface: &WlSurface) {
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
    fn a_slide_eases_out_from_where_it_started_to_where_it_is_going() {
        let from = Rect::from_xywh(0.0, 0.0, 100.0, 20.0);
        let to = Rect::from_xywh(0.0, 40.0, 100.0, 20.0);
        assert_eq!(slide(from, to, Duration::ZERO), from);
        assert_eq!(slide(from, to, HIGHLIGHT_SLIDE), to);
        assert_eq!(slide(from, to, HIGHLIGHT_SLIDE * 3), to);
        // Eased out: past halfway by half the time.
        let half = slide(from, to, HIGHLIGHT_SLIDE / 2);
        assert!(half.top > 20.0 && half.top < 40.0, "{half:?}");
        assert_eq!(half.height(), 20.0);
    }
}
