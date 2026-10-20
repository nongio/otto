//! Wayland-backed scrolling: the content lives in its own subsurface, cropped
//! to the viewport with `wp_viewport`.
//!
//! A [`ScrollView`] on its own paints into whatever canvas it is given, which
//! means the host repaints its whole window on every frame of a scroll.
//! [`ScrollSurfaces`] takes that work off the client entirely: it puts the
//! content in a subsurface whose buffer is a *band* — longer than the viewport
//! along the scrolling axis — and scrolls it by moving the band's viewport
//! source over that buffer. A frame of scrolling is a commit with a new source
//! rect: no drawing, no buffer and no upload; the client only paints when the
//! scroll approaches the edge of the rendered band and [`Band::refill`] asks
//! for a new one.
//!
//! Three surfaces, because each is a thing that moves or clips independently:
//!
//! ```text
//! clip   — fixed at the viewport, paints the pane background
//!   band — the content, longer than the viewport, cropped to it
//!   thumb — the scrollbar, above the band, moved and faded by the client
//! ```
//!
//! Either axis: a vertical pane scrolls its band up and down and keeps its
//! scrollbar on the right edge; a horizontal one scrolls it left and right
//! with the bar along the bottom.
//!
//! No compositor clips a subsurface to its parent, so every surface here is
//! cropped by the client to the part of the pane that shows — see
//! [`crop`](super::crop). That part is the viewport itself, or less of it when
//! the pane sits inside another one that shows only some of it: a host says
//! so with [`ScrollSurfaces::set_clip_rect`]. A surface with nothing left to
//! show is unmapped. Positions are whole points, sent before the surface's
//! own commit and followed by the parent's, since compositors disagree on
//! which of the two applies them; the fraction a scroll stands at is in the
//! band's source rect.
//!
//! A pane can hold other panes instead of painting content: see
//! [`ScrollSurfaces::container`]. Its band spans the whole content and carries
//! no pixels of its own, and the panes inside are children of
//! [`ScrollSurfaces::band_surface`] placed in content coordinates — so
//! scrolling the container moves one surface, and everything inside it rides
//! along. Its band is not cropped (that would move its children); each pane
//! inside crops itself to [`ScrollSurfaces::shown_content`] instead.
//!
//! The pointer is hit-tested against the cropped band: its local coordinates
//! start at the first point of content it shows, which
//! [`ScrollSurfaces::band_origin`] gives in content coordinates. The thumb
//! takes no input at all. Its surface carries an empty input region, so
//! presses fall through to the band beneath it and a host hit-tests the
//! scrollbar the way it always did, against [`ScrollRenderer::thumb_rect`] in
//! pane coordinates.
//!
//! A host that hit-tests everything in its window's own coordinates can opt
//! out of pointer input on the pane altogether with
//! [`ScrollSurfaces::set_input_passthrough`]: every event over the pane then
//! reaches the window as though the pane were painted into it.

use std::time::{Duration, Instant};

use skia_safe::{Canvas, Color, Rect, Size};
use wayland_client::protocol::wl_surface::WlSurface;

use crate::app_runner::AppContext;
use crate::surfaces::{SubsurfaceSurface, SurfaceError};
use crate::theme::Theme;

use super::band::{Band, BandView};
use super::crop;
use super::fill::Fill;
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
    /// The part of the parent that shows, in its coordinates, when the parent
    /// shows less than the whole viewport; see [`Self::set_clip_rect`].
    clip_rect: Option<Rect>,
    /// Where the clip surface's origin sits in pane coordinates: the corner
    /// of the part of the viewport that shows. `None` while nothing shows and
    /// the clip is unmapped.
    clip_origin: Option<(i32, i32)>,
    /// The scale the pane was last laid out at. The preferred scale arrives
    /// after these surfaces are built, so the first layout is always at the
    /// integer fallback and has to be redone once the real one lands.
    configured_scale: f32,
    /// Background painted into the clip surface, repainted only when it or the
    /// viewport changes.
    background: Color,
    /// Thumb thickness and length its buffer was last painted at, so it is
    /// only redrawn when it changes shape rather than every time it moves.
    thumb_size: (f32, f32),
    /// The scroll offset the band was last placed for.
    last_band_offset: Option<f32>,
    /// The content coordinate, along the axis, of the band surface's first
    /// shown point; see [`Self::band_origin`].
    band_local_origin: f32,
    /// The input region last pushed to a container's band, in band points.
    last_input_window: Option<Rect>,
    /// Where the thumb was last placed, in pane coordinates.
    last_thumb_rect: Option<Rect>,
    last_opacity: Option<f32>,
    /// The pane takes no pointer input; see [`Self::set_input_passthrough`].
    passthrough: bool,
    hidden: bool,
    /// The last move or paint has not reached the screen yet, so this sync
    /// was held back; see [`Self::waiting`].
    waiting: bool,
    /// The selection wash under the content; see [`Self::set_highlight`].
    highlight: Option<Highlight>,
}

/// A rounded wash under the content marking one item: a [`Fill`] sitting
/// between the pane's ground and its band, so moving the selection repaints
/// nothing.
struct Highlight {
    fill: Fill,
    /// Where it is going, in content coordinates.
    target: Rect,
    /// Where the slide towards `target` started, and when.
    from: Rect,
    started: Instant,
    /// Whether it is meant to be showing. The fill itself stays hidden until
    /// it has been placed, so it never appears where it was last.
    shown: bool,
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
            clip_rect: None,
            clip_origin: None,
            configured_scale: 0.0,
            background,
            thumb_size: (0.0, 0.0),
            last_band_offset: None,
            band_local_origin: 0.0,
            last_input_window: None,
            last_thumb_rect: None,
            last_opacity: None,
            passthrough: false,
            hidden: false,
            waiting: false,
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
    /// No compositor clips them to it: each one is handed
    /// [`Self::shown_content`] through its [`Self::set_clip_rect`] and crops
    /// itself.
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
        // pane: one pixel of the background, stretched to the viewport, is the
        // same solid ground at none of the cost — a buffer the size of the
        // pane is sampled under every step of the scroll.
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

    /// The content-space coordinate, along the axis, of the first point the
    /// content surface shows. A host translating pointer events that land on
    /// the content surface adds this to the surface-local coordinate to get
    /// content coordinates.
    pub fn band_origin(&self) -> f32 {
        self.band_local_origin
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
    /// start at the corner of the part of the viewport that shows, which is
    /// the viewport's own corner unless [`Self::set_clip_rect`] cut it.
    ///
    /// The scrollbar is deliberately not exposed: it has an empty input
    /// region and never receives a pointer event.
    pub fn clip_surface(&self) -> &WlSurface {
        self.clip.wl_surface()
    }

    /// The part of the content a [`Self::container`] shows, in content
    /// coordinates — which are its band's, and so the coordinates its nested
    /// panes are placed in. Hand it to each of them through
    /// [`Self::set_clip_rect`] after a sync, so they crop to what the
    /// container shows. `None` while it shows nothing.
    pub fn shown_content(&self) -> Option<Rect> {
        let offset = self.last_band_offset?;
        let window = self.window()?;
        Some(match self.axis {
            Axis::Vertical => window.with_offset((0.0, offset)),
            Axis::Horizontal => window.with_offset((offset, 0.0)),
        })
    }

    /// Show only the part of the pane inside `rect`, in the parent's
    /// coordinates, or the whole viewport with `None`.
    ///
    /// For a pane inside another one that shows only some of it — a column
    /// in a container panned half out of view: no compositor clips a
    /// subsurface to its parent, so the pane crops itself. Returns whether
    /// anything changed.
    pub fn set_clip_rect(&mut self, rect: Option<Rect>) -> bool {
        if self.clip_rect == rect {
            return false;
        }
        // A pan moves the rect on every step, but a pane wholly inside it
        // shows the same either way: only a change in what shows is sent.
        let before = self.window();
        self.clip_rect = rect;
        if self.window() == before {
            return false;
        }
        self.place_clip();
        true
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
        if !resized {
            self.place_clip();
            return;
        }
        if !self.passthrough {
            self.clip
                .resize(viewport.width() as i32, viewport.height() as i32);
        }
        self.band = Band::empty();
        self.forget_placement();
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
        self.place_clip();
        true
    }

    /// Mark `rect`, in content coordinates, with a rounded wash of `color`
    /// under the content, or take the mark away with `None`.
    ///
    /// The wash is a surface of its own between the pane's ground and its
    /// band: moving it — to follow the keyboard or the pointer — slides it
    /// there over [`HIGHLIGHT_SLIDE`] and repaints nothing, and a scroll moves
    /// it with the content. While the content is scrolling it does not slide:
    /// it goes straight to the item it marks. Takes effect on the next [`Self::sync`]; a host
    /// keeps syncing while [`Self::highlight_animating`].
    pub fn set_highlight(&mut self, rect: Option<Rect>, color: Color, radius: f32) {
        let Some(rect) = rect else {
            if let Some(highlight) = self.highlight.as_mut() {
                highlight.shown = false;
                highlight.fill.set_hidden(true);
            }
            return;
        };
        if self.highlight.is_none() {
            let Ok(mut fill) = Fill::new(self.clip.wl_surface()) else {
                return;
            };
            fill.set_hidden(true);
            fill.place_below(self.band_surface.wl_surface());
            // The order is the clip's pending state.
            self.clip.commit();
            self.highlight = Some(Highlight {
                fill,
                target: rect,
                from: rect,
                started: Instant::now(),
                shown: false,
            });
        }
        let Some(highlight) = self.highlight.as_mut() else {
            return;
        };
        highlight.fill.set_style(color, radius);
        let now = Instant::now();
        if !highlight.shown {
            // Nothing on screen to slide from: it appears where it belongs.
            highlight.shown = true;
            highlight.from = rect;
        } else if highlight.target != rect {
            highlight.from = slide(highlight.from, highlight.target, now - highlight.started);
        }
        if highlight.target != rect {
            highlight.target = rect;
            highlight.started = now;
        }
    }

    /// Whether the highlight is still sliding.
    pub fn highlight_animating(&self) -> bool {
        self.highlight.as_ref().is_some_and(|highlight| {
            highlight.shown
                && highlight.from != highlight.target
                && highlight.started.elapsed() < HIGHLIGHT_SLIDE
        })
    }

    /// Put the highlight where its slide has got to, moved with the content.
    /// `scrolled` says the content moved on this step.
    fn position_highlight(&mut self, offset: f32, scrolled: bool) -> bool {
        let axis = self.axis;
        let (Some(window), Some(origin)) = (self.window(), self.clip_origin) else {
            return false;
        };
        let Some(highlight) = self.highlight.as_mut().filter(|h| h.shown) else {
            return false;
        };
        // A slide is measured in the content, so while the content itself is
        // moving it would trail behind the item it marks — out of the pane,
        // on a fast fling. The mark goes straight to its item instead.
        if scrolled {
            highlight.from = highlight.target;
        }
        let rect = slide(
            highlight.from,
            highlight.target,
            highlight.started.elapsed(),
        );
        // From content coordinates to the clip's own.
        let (ox, oy) = (origin.0 as f32, origin.1 as f32);
        let shift = match axis {
            Axis::Vertical => (-ox, -offset - oy),
            Axis::Horizontal => (-offset - ox, -oy),
        };
        let window = window.with_offset((-ox, -oy));
        // Placed before it is shown, so it never appears where it last was.
        let placed = highlight.fill.set_window(Some(window))
            | highlight.fill.set_rect(rect.with_offset(shift));
        placed | highlight.fill.set_hidden(false)
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
        // so the layout made when the pane was built used the integer
        // fallback. Lay it out again: the band's source sits on the pixel grid
        // of the scale it was placed at.
        if self.configured_scale != self.scale() {
            self.configure_clip();
            self.band = Band::empty();
            self.forget_placement();
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
            // A band unmapped while the pane showed none of it has no buffer
            // left to show: it is painted afresh.
            if !self.band_surface.is_mapped() {
                self.band = Band::empty();
            }
            let band_view = BandView {
                offset: state.offset(),
                viewport_length: self.axis.length(self.viewport),
                content_length: state.content_length(),
                velocity,
            };
            if let Some(next) = self.band.refill(&band_view) {
                self.band = next;
                self.paint_band(state.offset(), content);
                changed = true;
            }
        }

        let band_moved = self.position_band(state.offset());
        changed |= band_moved;
        changed |= self.position_highlight(state.offset(), band_moved);
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

    /// The part of the pane that shows, in pane coordinates: the viewport,
    /// cut down to the clip rect when there is one. `None` when nothing of it
    /// shows, hidden or clipped away.
    fn window(&self) -> Option<Rect> {
        if self.hidden {
            return None;
        }
        let mut window = Rect::from_wh(self.viewport.width(), self.viewport.height());
        if let Some(clip) = self.clip_rect {
            let clip = clip.with_offset((-self.viewport.left, -self.viewport.top));
            if !window.intersect(clip) {
                return None;
            }
        }
        Some(window)
    }

    /// Forget where the band, the scrollbar and the input region were placed,
    /// so the next sync places them again: after the clip's origin or window
    /// changed, every child's crop and position inside it is stale.
    fn forget_placement(&mut self) {
        self.last_band_offset = None;
        self.last_input_window = None;
        self.last_thumb_rect = None;
    }

    /// The clip box: paints the pane background and places itself.
    fn configure_clip(&mut self) {
        self.configured_scale = self.scale();
        self.lay_out_clip(true);
    }

    /// Crop the clip to the part of the viewport that shows and put it there,
    /// or unmap it — and everything inside it — when none of it does.
    fn place_clip(&mut self) {
        self.lay_out_clip(false);
    }

    /// [`Self::place_clip`], painting the background as well when `paint`
    /// says so or the clip has no buffer to show. The crop is set before the
    /// paint, so it goes out with the buffer it was worked out for: a source
    /// rect committed over a buffer it does not lie inside is a protocol
    /// error.
    fn lay_out_clip(&mut self, paint: bool) {
        let Some(window) = self.window() else {
            self.clip.unmap();
            self.clip_origin = None;
            return;
        };
        // Unmapped, it lost its buffer: only a paint brings it back.
        let paint = paint || !self.clip.is_mapped();
        let (width, height) = self.clip.dimensions();
        let content = Size::new(width as f32, height as f32);
        let extent = Rect::from_wh(self.viewport.width(), self.viewport.height());
        let scale = self.scale();
        let parent_origin = (-(self.viewport.left as i32), -(self.viewport.top as i32));
        let Some(shown) = crop::show(
            &mut self.clip,
            content,
            extent,
            window,
            scale,
            parent_origin,
        ) else {
            self.clip_origin = None;
            return;
        };
        // Everything inside is cropped to the window and placed from the
        // clip's origin, so a change in either leaves every child stale. The
        // band is cropped again here, to go out with the clip: left for the
        // next sync, a pane panning into view would show only the sliver it
        // first showed, and one panning out would spill past its window,
        // until the band next moved.
        let band_offset = self.last_band_offset;
        self.clip_origin = Some(shown.position);
        self.forget_placement();
        if let Some(offset) = band_offset.filter(|_| !self.container) {
            if self.place_band(offset) {
                self.band_surface.commit();
                self.last_band_offset = Some(offset);
            }
        }
        if paint {
            let background = self.background;
            self.clip.draw(|canvas| {
                canvas.clear(background);
            });
        } else {
            self.clip.commit();
        }
        self.clip.commit_parent();
    }

    /// Where the whole band stands in pane coordinates for `offset`.
    fn band_extent(&self, offset: f32) -> Rect {
        let along = self.band.surface_offset(offset);
        let (width, height) = self.oriented(self.band.length(), self.cross_extent());
        let (x, y) = self.oriented(along, 0.0);
        Rect::from_xywh(x, y, width, height)
    }

    /// Crop the band to the window for `offset` and put it there, as pending
    /// state for the band's next commit and the clip's after it. Returns
    /// whether there was anywhere to put it.
    fn place_band(&mut self, offset: f32) -> bool {
        let (Some(window), Some(origin)) = (self.window(), self.clip_origin) else {
            return false;
        };
        let (width, height) = self.band_surface.dimensions();
        let content = Size::new(width as f32, height as f32);
        let extent = self.band_extent(offset);
        let scale = self.scale();
        let Some(shown) = crop::show(
            &mut self.band_surface,
            content,
            extent,
            window,
            scale,
            origin,
        ) else {
            return false;
        };
        // The band's first shown point, in content coordinates.
        self.band_local_origin = self.band.origin()
            + match self.axis {
                Axis::Vertical => shown.source.top,
                Axis::Horizontal => shown.source.left,
            };
        true
    }

    /// Paint the current band into the content surface, resizing its buffer to
    /// match. The canvas is translated so the closure draws in content space.
    fn paint_band<F>(&mut self, offset: f32, content: F)
    where
        F: FnOnce(&Canvas, Rect),
    {
        let (width, height) = self.oriented(self.band.length(), self.cross_extent());
        self.band_surface.resize(width as i32, height as i32);

        let rect = self.band.rect(self.axis, 0.0, self.cross_extent());
        let shift = match self.axis {
            Axis::Vertical => (0.0, -self.band.origin()),
            Axis::Horizontal => (-self.band.origin(), 0.0),
        };
        // The crop goes out with the buffer it fits: a source rect committed
        // over a buffer it does not lie inside is a protocol error.
        if self.place_band(offset) {
            self.clip.commit();
        }
        self.band_surface.draw(|canvas| {
            canvas.clear(Color::TRANSPARENT);
            canvas.save();
            canvas.translate(shift);
            content(canvas, rect);
            canvas.restore();
        });
        self.last_band_offset = Some(offset);
    }

    /// A container's band: the whole content, as a transparent pixel stretched
    /// to it. It has to have a buffer — a surface without one is unmapped, and
    /// so are the panes inside it — but painting a buffer the size of the
    /// content would cost the memory of every column at once for pixels
    /// nobody sees.
    ///
    /// It is never cropped: its origin is where the panes inside it are
    /// placed from, and a crop would move it.
    fn size_container_band(&mut self) {
        let (width, height) = self.oriented(self.band.length(), self.cross_extent());
        self.band_surface.resize(1, 1);
        let destination = (
            width.max(1.0).round() as i32,
            height.max(1.0).round() as i32,
        );
        self.band_surface
            .set_view(Some((Rect::from_wh(1.0, 1.0), destination)));
        self.band_surface.draw(|canvas| {
            canvas.clear(Color::TRANSPARENT);
        });
        self.last_band_offset = None;
    }

    /// Move the band to where this offset puts it. This is the whole cost of a
    /// frame of scrolling. Returns whether it moved.
    fn position_band(&mut self, offset: f32) -> bool {
        if self.last_band_offset == Some(offset) {
            return false;
        }
        let Some(origin) = self.clip_origin else {
            return false;
        };
        self.last_band_offset = Some(offset);

        let placed = if self.container {
            // Whole and uncropped: only its position moves, in whole points.
            let along = self.band.surface_offset(offset).round() as i32;
            let (x, y) = match self.axis {
                Axis::Vertical => (0, along),
                Axis::Horizontal => (along, 0),
            };
            self.band_surface.set_position(x - origin.0, y - origin.1);
            self.clip_container_input();
            true
        } else {
            self.place_band(offset)
        };
        // A move is not a paint, so nothing else asks to hear when it reaches
        // the screen — and that answer is what paces the next step of a
        // glide. Without it a host whose window has stopped repainting has
        // nothing to wake it at the display's rate.
        AppContext::request_throttled_frame(self.band_surface.wl_surface());
        self.band_surface.commit();
        if placed {
            self.clip.commit();
        }
        true
    }

    /// Cut a container band's input region down to the part of it that shows.
    ///
    /// The band spans the whole content and is not cropped, so it reaches far
    /// past the pane — on a pane at the window's edge, past the window itself.
    /// A surface with no input region of its own takes input over its whole
    /// extent: without this the window swallows clicks past its own edge, and
    /// the content beside the pane gets events meant for the chrome.
    fn clip_container_input(&mut self) {
        if self.passthrough {
            return;
        }
        let Some(shown) = self.shown_content() else {
            return;
        };
        if self.last_input_window == Some(shown) {
            return;
        }
        self.last_input_window = Some(shown);
        let compositor = AppContext::compositor_state();
        let qh = AppContext::queue_handle();
        let region = compositor.wl_compositor().create_region(qh, ());
        // Rounded outwards so no row along either edge is dead.
        region.add(
            shown.left.floor() as i32,
            shown.top.floor() as i32,
            shown.width().ceil() as i32,
            shown.height().ceil() as i32,
        );
        self.band_surface
            .wl_surface()
            .set_input_region(Some(&region));
        region.destroy();
    }

    /// The scrollbar moves and fades in the client: its surface is cropped to
    /// the window and placed, and only repainted when the thumb changes
    /// shape, which happens when the content's length changes or the pointer
    /// expands it — not while scrolling. Returns whether anything was sent.
    fn position_thumb(&mut self, state: &ScrollState, theme: &Theme) -> bool {
        let Some(rect) = ScrollRenderer::thumb_rect(state) else {
            return self.hide_thumb();
        };
        let opacity = state.scrollbar_opacity();
        if opacity <= 0.0 {
            return self.hide_thumb();
        }
        let (Some(window), Some(origin)) = (self.window(), self.clip_origin) else {
            return false;
        };

        let (thickness, length) = match self.axis {
            Axis::Vertical => (rect.width(), rect.height()),
            Axis::Horizontal => (rect.height(), rect.width()),
        };
        let thickness = thickness.round();
        let length = length.round().max(1.0);
        // The thumb is placed in the view's own coordinates; the clip is the
        // viewport, so its place inside it is measured from the viewport's
        // leading edge rather than from wherever the view sits.
        let along = match self.axis {
            Axis::Vertical => rect.top - state.viewport().top,
            Axis::Horizontal => rect.left - state.viewport().left,
        };
        let extent = match self.axis {
            Axis::Vertical => Rect::from_xywh(
                self.viewport.width() - THUMB_STRIP,
                along,
                THUMB_STRIP,
                length,
            ),
            Axis::Horizontal => Rect::from_xywh(
                along,
                self.viewport.height() - THUMB_STRIP,
                length,
                THUMB_STRIP,
            ),
        };

        let mut changed = false;
        // The thumb squashes continuously while an overscroll bounces. Its
        // buffer is stretched to follow — the viewport scales what is already
        // there — and only painted again when the thickness changes, the
        // length has drifted far enough that the stretch would show in the
        // rounded ends, or it was unmapped and has no buffer.
        let (painted_thickness, painted_length) = self.thumb_size;
        let repaint = thickness != painted_thickness
            || painted_length <= 0.0
            || (length / painted_length - 1.0).abs() > THUMB_STRETCH
            || !self.thumb.is_mapped();
        if repaint {
            self.thumb_size = (thickness, length);
            let (width, height) = self.oriented(length, THUMB_STRIP);
            self.thumb.resize(width as i32, height as i32);
            // Placed before the paint, so the crop goes out with the buffer
            // it was worked out for.
            self.last_thumb_rect = None;
            self.place_thumb(extent, window, origin);
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
            // The draw committed the thumb; its position is the clip's.
            self.clip.commit();
            changed = true;
        } else if self.place_thumb(extent, window, origin) {
            self.thumb.commit();
            self.clip.commit();
            changed = true;
        }

        // A fade is the style's to show; without one the thumb is simply
        // there until it is gone.
        if self.last_opacity != Some(opacity) {
            self.last_opacity = Some(opacity);
            if let Some(style) = self.thumb.layer() {
                style.set_opacity(opacity as f64);
                self.thumb.commit();
                changed = true;
            }
        }
        changed
    }

    /// Crop the thumb to the window and put it at `extent`, in pane
    /// coordinates, as pending state. Returns whether anything was sent.
    fn place_thumb(&mut self, extent: Rect, window: Rect, origin: (i32, i32)) -> bool {
        if self.last_thumb_rect == Some(extent) {
            return false;
        }
        self.last_thumb_rect = Some(extent);
        let (width, height) = self.thumb.dimensions();
        let content = Size::new(width as f32, height as f32);
        let scale = self.scale();
        crop::show(&mut self.thumb, content, extent, window, scale, origin);
        true
    }

    /// Take the scrollbar away: faded out through the style where there is
    /// one, and unmapped either way once it is gone.
    fn hide_thumb(&mut self) -> bool {
        if self.last_opacity == Some(0.0) {
            return false;
        }
        self.last_opacity = Some(0.0);
        if let Some(style) = self.thumb.layer() {
            style.set_opacity(0.0);
        }
        self.last_thumb_rect = None;
        self.thumb.unmap();
        true
    }
}

impl Drop for ScrollSurfaces {
    /// A pane let go of is torn down, children before the clip they hang
    /// from: a subsurface merely forgotten stays mapped, with its buffer and
    /// its input region, until the client exits.
    ///
    /// Panes nested in a container are the host's to drop, and belong before
    /// the container — once its band is gone they have nowhere to be shown.
    fn drop(&mut self) {
        self.highlight = None;
        self.thumb.destroy();
        self.band_surface.destroy();
        self.clip.destroy();
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
