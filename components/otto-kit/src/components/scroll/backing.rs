//! Wayland-backed scrolling: the content lives in its own subsurface and the
//! compositor does the cropping.
//!
//! A [`ScrollView`] on its own paints into whatever canvas it is given, which
//! means the host repaints its whole window on every frame of a scroll.
//! [`ScrollSurfaces`] takes that work off the client entirely: it puts the
//! content in a subsurface whose buffer is a *band* — taller than the viewport
//! — and scrolls it by moving that surface with `otto_surface_style_v1`. The
//! parent surface clips its children to the viewport (`set_clip_children`), so
//! nothing spills. Moving a surface is a protocol request, not a paint, so a
//! frame of scrolling costs no drawing, no buffer and no upload; the client
//! only paints when the scroll approaches the edge of the rendered band and
//! [`Band::refill`] asks for a new one.
//!
//! Three surfaces, because each is a thing that moves or clips independently:
//!
//! ```text
//! clip   — fixed at the viewport, paints the pane background, clips children
//!   band — the content, taller than the viewport, moved to scroll
//!   thumb — the scrollbar, above the band, moved and faded by the compositor
//! ```
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

use skia_safe::{Canvas, Color, Rect};
use wayland_client::protocol::wl_surface::WlSurface;

use crate::protocols::otto_surface_style_v1::ClipMode;
use crate::surfaces::{SubsurfaceSurface, SurfaceError};
use crate::theme::Theme;

use super::band::{Band, BandView};
use super::renderer::ScrollRenderer;
use super::scroll::ScrollView;
use super::state::Axis;

/// Width of the strip the scrollbar surface occupies, in points. Wide enough
/// for the thumb at its expanded width plus its margin.
const THUMB_STRIP_W: f32 = 16.0;

/// The surfaces behind a [`ScrollView`], and the band currently painted into
/// them.
pub struct ScrollSurfaces {
    clip: SubsurfaceSurface,
    band_surface: SubsurfaceSurface,
    thumb: SubsurfaceSurface,
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
    /// Size of the thumb last painted, so it is only redrawn when it changes
    /// shape rather than every time it moves.
    thumb_size: (f32, f32),
    /// Last values pushed to the compositor, to skip redundant requests.
    last_top: Option<f32>,
    /// Band-local top of the input region last pushed, in points.
    last_input_top: Option<i32>,
    last_thumb_top: Option<f32>,
    last_opacity: Option<f32>,
}

impl ScrollSurfaces {
    /// The output's fractional scale, read fresh every time.
    ///
    /// Not cached: `wp_fractional_scale_v1` delivers the preferred scale
    /// asynchronously, and these surfaces are usually built before the first
    /// event arrives — a value snapshotted in the constructor is the integer
    /// fallback, and stays wrong for the life of the pane.
    fn scale(&self) -> f32 {
        crate::app_runner::AppContext::fractional_scale() as f32
    }

    /// Build the surfaces under `parent`, with the pane occupying `viewport`
    /// in the parent's coordinate space.
    pub fn new(
        parent: &WlSurface,
        viewport: Rect,
        background: Color,
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
        let thumb = SubsurfaceSurface::new(
            clip.wl_surface(),
            (viewport.width() - THUMB_STRIP_W) as i32,
            0,
            THUMB_STRIP_W as i32,
            1,
        )?;
        thumb.place_above(band_surface.wl_surface());
        // The thumb is painted, not touched: an empty input region lets every
        // press through to the band, which keeps scrollbar hit-testing a
        // question about pane coordinates rather than about which surface the
        // pointer happened to land on.
        thumb.wl_surface().set_input_region(Some(
            &crate::app_runner::AppContext::compositor_state()
                .wl_compositor()
                .create_region(crate::app_runner::AppContext::queue_handle(), ()),
        ));
        thumb.commit();

        let mut surfaces = Self {
            clip,
            band_surface,
            thumb,
            band: Band::empty(),
            viewport,
            configured_scale: 0.0,
            background,
            thumb_size: (0.0, 0.0),
            last_top: None,
            last_input_top: None,
            last_thumb_top: None,
            last_opacity: None,
        };
        surfaces.configure_clip();
        Ok(surfaces)
    }

    /// The content-space y of the top of the painted band. A host translating
    /// pointer events that land on the content surface adds this to the
    /// surface-local y to get content coordinates.
    pub fn band_origin(&self) -> f32 {
        self.band.origin()
    }

    /// The surface carrying the content, for a host that needs to recognise
    /// pointer events arriving on it.
    pub fn content_surface(&self) -> &WlSurface {
        self.band_surface.wl_surface()
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

    /// Move or resize the pane. Forces a repaint of the background and, on the
    /// next [`Self::sync`], of the band.
    pub fn set_viewport(&mut self, viewport: Rect) {
        if viewport == self.viewport {
            return;
        }
        self.viewport = viewport;
        self.clip
            .resize(viewport.width() as i32, viewport.height() as i32);
        self.clip
            .set_position(viewport.left as i32, viewport.top as i32);
        self.band = Band::empty();
        self.last_top = None;
        self.last_input_top = None;
        self.last_thumb_top = None;
        self.configure_clip();
    }

    /// Bring the surfaces in line with the view: repaint the band if the scroll
    /// has reached its margin, then move it and the scrollbar to where the
    /// current offset puts them.
    ///
    /// `content` paints in content coordinates and is given the band's rect —
    /// the same contract as [`ScrollRenderer::draw`]'s closure, except it is
    /// called only on the rare frame that needs a new band.
    pub fn sync<F>(&mut self, view: &ScrollView, theme: &Theme, content: F)
    where
        F: FnOnce(&Canvas, Rect),
    {
        // `wp_fractional_scale_v1` reports the output's scale asynchronously,
        // so the geometry pushed when the pane was built used the integer
        // fallback. Re-push everything that scaled by it — otherwise the pane
        // keeps the position and size of a 2x output on a 1.25x one, sitting
        // too far right and reaching past the window.
        if self.configured_scale != self.scale() {
            self.configure_clip();
            self.band = Band::empty();
            self.last_top = None;
            self.last_thumb_top = None;
        }

        let state = &view.state;
        debug_assert_eq!(
            state.axis(),
            Axis::Vertical,
            "surface-backed panes band vertically only"
        );
        let band_view = BandView {
            offset: state.offset(),
            viewport_height: self.viewport.height(),
            content_height: state.content_length(),
            velocity: view.velocity(),
        };

        if let Some(next) = self.band.refill(&band_view) {
            self.band = next;
            self.paint_band(content);
        }

        self.position_band(state.offset());
        self.position_thumb(view, theme);
    }

    /// The clip box: claims its bounds so the compositor stops re-deriving
    /// them, clips whatever moves inside it, and paints the pane background.
    fn configure_clip(&mut self) {
        self.configured_scale = self.scale();
        if let Some(style) = self.clip.layer() {
            style.set_size(
                (self.viewport.width() * self.scale()) as f64,
                (self.viewport.height() * self.scale()) as f64,
            );
            // Claiming the size stops the compositor deriving *both* size and
            // position from the surface tree, so the position the subsurface
            // was created with no longer reaches the layer — without this the
            // pane is drawn at the window's origin, on top of the chrome.
            style.set_position(
                (self.viewport.left * self.scale()) as f64,
                (self.viewport.top * self.scale()) as f64,
            );
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
        let width = self.viewport.width();
        let height = self.band.height();
        self.band_surface.resize(width as i32, height as i32);
        if let Some(style) = self.band_surface.layer() {
            style.set_size(
                (width * self.scale()) as f64,
                (height * self.scale()) as f64,
            );
        }

        let rect = self.band.rect(0.0, width);
        let origin = self.band.origin();
        if std::env::var_os("OTTO_PANE_DEBUG").is_some() {
            eprintln!(
                "[banddbg] paint band origin={origin:.0} h={height:.0} w={width:.0} scale={} viewport={:?}",
                self.scale(), self.viewport
            );
        }
        self.band_surface.draw(|canvas| {
            canvas.clear(Color::TRANSPARENT);
            canvas.save();
            canvas.translate((0.0, -origin));
            content(canvas, rect);
            canvas.restore();
        });
        // A fresh buffer starts at the surface's own origin; wherever it was
        // standing before means nothing now.
        self.last_top = None;
    }

    /// Move the band to where this offset puts it. This is the whole cost of a
    /// frame of scrolling.
    fn position_band(&mut self, offset: f32) {
        let top = self.band.surface_top(offset);
        if self.last_top == Some(top) {
            return;
        }
        self.last_top = Some(top);
        if std::env::var_os("OTTO_PANE_DEBUG").is_some() {
            eprintln!(
                "[banddbg] position top={top:.1} (offset {offset:.1}, origin {:.1})",
                self.band.origin()
            );
        }

        if let Some(style) = self.band_surface.layer() {
            style.set_position(0.0, (top * self.scale()) as f64);
        }
        // The pointer is hit-tested against the subsurface position, so it has
        // to follow — rounded, which is under a point out and invisible to a
        // hit test.
        let top = top.round() as i32;
        self.band_surface.set_position(0, top);
        self.clip_band_input(top);
        self.band_surface.commit();
    }

    /// Cut the band's input region down to the slice of it the viewport shows.
    ///
    /// The band is taller than the viewport and hangs out of the clip surface
    /// at both ends — by up to [`MIN_OVERDRAW`](super::band) points, which on a
    /// pane that reaches the bottom of the window is a strip of live surface
    /// hanging below the window itself. `set_clip_children` crops what is
    /// *drawn*; the pointer knows nothing about it, and a surface with no
    /// input region of its own takes input over the whole buffer. So the band
    /// carries a region covering exactly the part of it inside the clip, moved
    /// with it: without this the window swallows clicks below its own edge, and
    /// the content above the pane gets events meant for the chrome.
    fn clip_band_input(&mut self, top: i32) {
        if self.last_input_top == Some(top) {
            return;
        }
        self.last_input_top = Some(top);

        let compositor = crate::app_runner::AppContext::compositor_state();
        let qh = crate::app_runner::AppContext::queue_handle();
        let region = compositor.wl_compositor().create_region(qh, ());
        // Band-local: the viewport's top edge sits at `-top` in the band's own
        // coordinates. Rounded outwards so no row along either edge is dead.
        region.add(
            0,
            -top,
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
    /// scrolling.
    fn position_thumb(&mut self, view: &ScrollView, theme: &Theme) {
        let Some(rect) = ScrollRenderer::thumb_rect(&view.state) else {
            self.hide_thumb();
            return;
        };
        let opacity = view.state.scrollbar_opacity();
        if opacity <= 0.0 {
            self.hide_thumb();
            return;
        }

        let size = (rect.width(), rect.height());
        if size != self.thumb_size {
            self.thumb_size = size;
            self.thumb
                .resize(THUMB_STRIP_W as i32, rect.height().max(1.0) as i32);
            if let Some(style) = self.thumb.layer() {
                style.set_size(
                    (THUMB_STRIP_W * self.scale()) as f64,
                    (rect.height() * self.scale()) as f64,
                );
            }
            let color = theme.fill_secondary;
            let width = rect.width();
            let height = rect.height();
            self.thumb.draw(|canvas| {
                canvas.clear(Color::TRANSPARENT);
                let mut paint = skia_safe::Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(color);
                let radius = width / 2.0;
                canvas.draw_rrect(
                    skia_safe::RRect::new_rect_xy(
                        Rect::from_xywh(THUMB_STRIP_W - width, 0.0, width, height),
                        radius,
                        radius,
                    ),
                    &paint,
                );
            });
            self.last_thumb_top = None;
        }

        if let Some(style) = self.thumb.layer() {
            if self.last_thumb_top != Some(rect.top) {
                self.last_thumb_top = Some(rect.top);
                style.set_position(
                    ((self.viewport.width() - THUMB_STRIP_W) * self.scale()) as f64,
                    (rect.top * self.scale()) as f64,
                );
            }
            if self.last_opacity != Some(opacity) {
                self.last_opacity = Some(opacity);
                style.set_opacity(opacity as f64);
            }
        }
        self.thumb.commit();
    }

    fn hide_thumb(&mut self) {
        if self.last_opacity == Some(0.0) {
            return;
        }
        self.last_opacity = Some(0.0);
        if let Some(style) = self.thumb.layer() {
            style.set_opacity(0.0);
        }
        self.thumb.commit();
    }
}
