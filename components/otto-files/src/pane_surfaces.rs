//! Miller columns as their own Wayland subsurfaces.
//!
//! The browser normally paints every column into the window's single buffer,
//! which means a scroll in one column repaints the whole window and — because
//! otto-kit damages the entire buffer on commit — tells the compositor that
//! everything changed. Measured on the `scroll_ab` example, that is 82 full
//! window repaints across one fling.
//!
//! Here each column gets a subsurface the size of the column. The client still
//! does all the work — it renders, translates and clips the rows itself, in
//! that subsurface's own buffer — so the painting is unchanged. What changes is
//! the *scope*: a scroll damages one column, not the window, and the toplevel
//! is left alone entirely.
//!
//! This is deliberately the cheaper of the two subsurface designs. The other,
//! [`otto_kit::components::scroll::ScrollSurfaces`], keeps a band taller than
//! the viewport and scrolls it with `otto_surface_style_v1`, so a frame of
//! scrolling costs no paint at all — but it needs band management, the style
//! protocol and pointer routing per column. This one reuses the existing
//! drawing untouched and still gets the damage confinement.
//!
//! **Input is not routed here.** Every column surface carries an empty input
//! region, so pointer events fall straight through to the toplevel and the
//! browser keeps hit-testing in window coordinates exactly as it always has.
//! Without that, `Window::on_pointer_event` — which filters to the toplevel's
//! own surface — would silently stop seeing anything over the columns.
//!
//! Off by default; set `OTTO_FILES_PANE_SUBS=1` to turn it on.

use otto_kit::app_runner::AppContext;
use otto_kit::components::scroll::ScrollRenderer;
use otto_kit::surfaces::SubsurfaceSurface;
use skia_safe::Rect;
use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::quickview;
use crate::scene;
use crate::view::{self, Frame, ViewMode};

/// Whether Quick View is centred on the display rather than on the window.
///
/// The panel is a subsurface, so its position is relative to the browser's
/// window — and a client is never told where its own window sits, so it cannot
/// place itself anywhere else on its own. `set_output_placement` asks the
/// compositor, which knows both, to resolve the position against the output.
///
/// On by default: a preview is a thing you look at, and where the *window*
/// happens to sit is no reason for it to open off to one side of the display.
/// A window pushed to a screen edge otherwise puts its preview there too.
///
/// This is independent of [`enabled`] — Quick View gets a surface of its own
/// whether or not the columns do, because it is the only part of this module
/// that needs one to be positioned at all. `OTTO_FILES_QV_CENTER=0` opts out
/// and goes back to centring on the window.
pub fn quickview_centered() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        !matches!(
            std::env::var("OTTO_FILES_QV_CENTER").as_deref(),
            Ok("0") | Ok("false")
        )
    })
}

/// Whether Quick View is presented on a subsurface rather than painted into
/// the window's own canvas.
///
/// Centring on the display requires one: a client is never told where its
/// window is, so the panel has to be a surface the compositor can place.
pub fn quickview_on_surface() -> bool {
    enabled() || quickview_centered()
}

/// Whether the subsurface path is enabled for this process.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OTTO_FILES_PANE_SUBS").is_some())
}

/// One column's surface and what it currently holds.
struct PaneSurface {
    surface: SubsurfaceSurface,
    /// Where it sits and how big it is, in window points.
    rect: Rect,
    /// The scroll offset the painted content was drawn at.
    scroll: f32,
    /// The scrollbar opacity it was drawn at. The bar fades in on a scroll and
    /// out again when the glide stops, and that fade is the column's own to
    /// draw now — so it has to be able to make the column repaint on its own,
    /// with the offset unchanged.
    bar: f32,
    /// Identity of everything else that feeds the drawing.
    key: u64,
    hidden: bool,
    /// The compositor owns this surface's position, so nothing here may set
    /// it: a parent-relative position sent after the centring would simply
    /// undo it.
    output_centered: bool,
}

/// The per-column subsurfaces, pooled the way the scene pools its pane layers.
pub struct PaneSurfaces {
    panes: Vec<PaneSurface>,
    /// The stack's horizontal bar, in a surface of its own over the columns.
    pan: Option<PaneSurface>,
    /// The Quick View panel, in a surface of its own over everything.
    quickview: Option<PaneSurface>,
    /// Which panel, and which direction, [`Self::quickview_resting`] was
    /// worked out for: the session's generation and whether it is closing.
    /// `Some(closing)` once the output has been asked about for the panel
    /// currently up. Deliberately *not* keyed on the decode generation:
    /// arrow-keying to the next file does not move the window or resize the
    /// panel, so the compositor's answer cannot have changed, and re-asking
    /// per file made the panel fall back to the window's centre for the frame
    /// or two before the new answer landed — a visible jump on every file.
    quickview_placement: Option<bool>,
    /// A fresh answer has been asked for and has not arrived. The previous
    /// resting rect stays in use until it does.
    quickview_awaiting: bool,
    /// Where the panel rests, frozen for the length of one opening or one
    /// closing.
    ///
    /// Recomputed only at those two moments, and never in between. The
    /// compositor's answer is relative to the *window*, so anything that moves
    /// the window — tiling it, dragging it — changes what it means. Following
    /// that continuously would drag the panel across the screen mid-animation
    /// and, when the window moves far enough, off it.
    quickview_resting: Option<Rect>,
    /// Set when a paint was wanted but the surface still had a frame in
    /// flight. The throttle is only safe while something else keeps calling
    /// `sync`: a surface whose content has stopped changing is never asked
    /// again, so a skipped paint at the end of an animation would be skipped
    /// for good — the panel would simply never appear.
    pending: bool,
    /// Set when a surface has been created, which is the only thing that can
    /// disturb the sibling order: a new subsurface arrives on top of every
    /// one of its siblings, including the overlays that must stay above them.
    stack_dirty: bool,
    scale: f32,
}

/// How tall a slice of the viewport the horizontal bar can touch: the gutter
/// it sits in, plus room for the widening it does on hover.
const PAN_BAR_STRIP: f32 = 16.0;

impl PaneSurfaces {
    pub fn new(scale: f32) -> Self {
        Self {
            panes: Vec::new(),
            pan: None,
            quickview: None,
            quickview_placement: None,
            quickview_awaiting: false,
            quickview_resting: None,
            pending: false,
            stack_dirty: false,
            scale,
        }
    }

    /// Bring the column surfaces in line with the frame: create, move, resize
    /// and repaint them as needed.
    ///
    /// Returns whether anything was repainted, so a caller can tell a frame
    /// that did real work from one that did nothing.
    pub fn sync(
        &mut self,
        parent: &WlSurface,
        f: &Frame,
        quickview: Option<(&quickview::Session, u64)>,
    ) -> bool {
        self.pending = false;
        // The column half is still opt-in. When it is off this module exists
        // only to carry Quick View's surface, so the scene keeps drawing the
        // columns and nothing here touches them.
        if f.mode != ViewMode::Columns || !enabled() {
            let mut changed = self.hide_all();
            changed |= self.sync_quickview(parent, f, quickview);
            self.restack(parent);
            return changed;
        }
        let viewport = view::content_viewport(f.width, f.height, ViewMode::Columns);
        let mut painted = false;

        for depth in 0..f.panes.len() {
            let full = view::miller_pane_rect(depth, f.height, f.pan, f.miller_w);
            // A column panned entirely off the content area has nothing to
            // show; leaving its surface mapped would put it under the sidebar.
            let visible = full.right > viewport.left && full.left < viewport.right;

            if depth >= self.panes.len() {
                match Self::create(parent, full) {
                    Some(pane) => {
                        self.panes.push(pane);
                        self.stack_dirty = true;
                    }
                    None => continue,
                }
            }

            let scale = self.scale;
            let pane = &mut self.panes[depth];
            if !visible {
                painted |= pane.hide();
                continue;
            }
            painted |= pane.show();

            // Crop the surface to the content area rather than letting it
            // spill. The scene clipped columns with `content.set_clip_children`
            // — a subsurface has no such parent, it is a child of the toplevel,
            // so a column panned past the sidebar would simply draw over it.
            // Shrinking the surface to the visible slice *is* the clip, and the
            // content is then shifted by however much was cropped off the left.
            let mut clipped = full;
            if !clipped.intersect(viewport) {
                painted |= pane.hide();
                continue;
            }
            let dx = full.left - clipped.left;
            pane.place(clipped, scale);

            let key = column_key(f, depth);
            let scroll = f.panes[depth].scroll;
            let bar = f.panes[depth]
                .bar
                .map(|state| state.scrollbar_opacity())
                .unwrap_or(0.0);
            if pane.key == key && pane.scroll == scroll && pane.bar == bar {
                continue;
            }
            // Frame-callback throttled, like the toplevel: painting again
            // while the last buffer has not been presented only queues work
            // the compositor has not asked for.
            use wayland_client::Proxy;
            if AppContext::frame_in_flight(&pane.surface.wl_surface().id()) {
                self.pending = true;
                continue;
            }
            pane.key = key;
            pane.scroll = scroll;
            pane.bar = bar;
            // The rows are laid out against the column's full width, then
            // slid by the cropped-off amount, so a half-visible column shows
            // the correct half rather than a squeezed whole.
            let width = full.width();
            let origin = (clipped.left, clipped.top);
            pane.surface.draw(|canvas| {
                canvas.save();
                canvas.translate((dx, 0.0));
                scene::paint_column(canvas, f, depth, width);
                canvas.restore();

                // The column's own scrollbar. It used to be drawn over the
                // stack from `view::draw_miller`, but the window is no longer
                // repainted for a scroll — which is the point — so the bar
                // would fade in and never move. It belongs to the column, so
                // it rides in the column's surface, shifted out of window
                // coordinates into this surface's own.
                if let Some(state) = f.panes[depth].bar {
                    canvas.save();
                    canvas.translate((-origin.0, -origin.1));
                    ScrollRenderer::draw(canvas, state, f.theme, |_, _| {});
                    canvas.restore();
                }
            });
            painted = true;
        }

        for pane in self.panes.iter_mut().skip(f.panes.len()) {
            painted |= pane.hide();
        }
        painted |= self.sync_pan_bar(parent, f, viewport);
        painted |= self.sync_quickview(parent, f, quickview);
        self.restack(parent);
        painted
    }

    /// Put the sibling surfaces back into a known order, bottom to top:
    /// columns in depth order, then the stack's bar, then Quick View.
    ///
    /// Stacking each surface against the one below it states the whole order
    /// rather than assuming one. Placing an overlay above "the last column"
    /// does not: `wl_subsurface.place_above` is relative to one named sibling,
    /// so it only lifts the overlay above *that* surface, and the last column
    /// in this pool is not always the topmost sibling — a hidden one still
    /// holds its place in the stack. That is how a third column ended up over
    /// the panel.
    ///
    /// Only after a surface has been created, since nothing else reorders
    /// siblings, and only relative to surfaces that exist — the requests are
    /// double-buffered on the *parent*, so they land with the toplevel's next
    /// commit rather than this one.
    fn restack(&mut self, parent: &WlSurface) {
        // Only when a surface was created, which is the only thing that
        // reaches into the sibling order: a pooled column is hidden by going
        // transparent, never destroyed, so it keeps its place in the stack
        // and coming back does not disturb anyone.
        if !std::mem::take(&mut self.stack_dirty) {
            return;
        }
        let trace = std::env::var_os("OTTO_FILES_QV_TRACE").is_some();
        let mut order: Vec<String> = Vec::new();
        let mut below: Option<WlSurface> = None;
        let overlays = [self.pan.as_ref(), self.quickview.as_ref()];
        for (index, pane) in self.panes.iter().map(Some).chain(overlays).enumerate() {
            let Some(pane) = pane else { continue };
            let surface = pane.surface.wl_surface().clone();
            if trace {
                use wayland_client::Proxy;
                order.push(format!("{index}:{}", surface.id().protocol_id()));
            }
            if let Some(below) = &below {
                pane.surface.place_above(below);
            }
            below = Some(surface);
        }
        if trace {
            eprintln!("qv restack: {}", order.join(" < "));
        }
        // `place_above` is part of the *parent's* pending state, so committing
        // the children does nothing for it. Without this the new order waits
        // for whatever else happens to commit the toplevel — and when a column
        // appears while Quick View is up, nothing does, so the column that
        // arrived on top stays on top.
        parent.commit();
    }

    /// Where this panel rests, worked out once per opening and once per
    /// closing and held steady in between.
    fn resting_for(&mut self, session: &quickview::Session) -> Option<Rect> {
        if !quickview_centered() {
            return None;
        }
        let pane = self.quickview.as_ref()?;
        let style = pane.surface.layer()?;

        use wayland_client::Proxy;
        // Once per open, and again when the exit starts — the two moments the
        // answer can actually differ.
        let placement = session.closing.is_some();
        if self.quickview_placement != Some(placement) {
            self.quickview_placement = Some(placement);
            self.quickview_awaiting = true;
            // Clear before asking, so a stale answer cannot be mistaken for
            // the new one.
            AppContext::clear_output_frame(&style.id());
            style.request_output_frame();
        }
        // The *old* rect stays in force until the new answer lands. Nulling it
        // here is what made the panel snap to the window's centre and back.
        if self.quickview_awaiting {
            if let Some(rect) = centered_resting(pane, self.scale) {
                self.quickview_resting = Some(rect);
                self.quickview_awaiting = false;
            }
        }
        self.quickview_resting
    }

    /// The Quick View panel.
    ///
    /// Drawn into the window it would be buried: the column surfaces sit over
    /// the toplevel, so a panel painted underneath them is a panel nobody can
    /// see. It gets a surface of its own, stacked above every column and above
    /// the horizontal bar — the topmost thing this window puts on screen.
    ///
    /// Its input region stays empty like the columns'. The panel already owns
    /// the pointer through the host's own routing, which works in window
    /// coordinates and does not care which surface the pixels came from.
    fn sync_quickview(
        &mut self,
        parent: &WlSurface,
        f: &Frame,
        quickview: Option<(&quickview::Session, u64)>,
    ) -> bool {
        let Some((session, generation)) = quickview else {
            // Nothing is up, so the next open asks afresh. The last known
            // resting rect is kept: if the window has not moved it is still
            // right, and starting from it beats starting from the window's
            // centre and correcting.
            self.quickview_placement = None;
            self.quickview_awaiting = false;
            return self
                .quickview
                .as_mut()
                .map(PaneSurface::hide)
                .unwrap_or(false);
        };
        // Centred on the display when the compositor has told us where the
        // display is, and centred on the window until it has. Everything below
        // stays in window coordinates either way, which is what lets the
        // entrance keep growing out of the file's icon: the anchor and the
        // resting place are in the same space.
        let resting = self
            .resting_for(session)
            .unwrap_or_else(|| quickview::panel_rect(f.width, f.height + f.footer));
        // Wherever the panel is *now* — part way in, at rest, or part way
        // back to its file. Asking for the entrance alone left the exit out
        // of the surface entirely: once open, `entrance_t` is pinned at 1, so
        // through the whole close this rect never moved and the key below
        // never changed, and the card sat frozen at full size until the
        // session was retired out from under it.
        let panel = session.panel(resting);
        let mut rect = panel.with_outset((quickview::SURFACE_MARGIN, quickview::SURFACE_MARGIN));
        // A panel centred on the display may legitimately reach past the
        // window it belongs to, so it is only clipped to the window when it is
        // the window it is centred on.
        if !quickview_centered() && !rect.intersect(Rect::from_wh(f.width, f.height)) {
            return self
                .quickview
                .as_mut()
                .map(PaneSurface::hide)
                .unwrap_or(false);
        }

        if self.quickview.is_none() {
            self.quickview = Self::create(parent, rect);
            self.stack_dirty = true;
            if let Some(pane) = self.quickview.as_mut() {
                pane.output_centered = quickview_centered();
                Self::style_quickview(pane, self.scale);
                // Undo the empty input region `create` sets. A panel centred
                // on the display hangs outside the toplevel, and the pointer
                // never reports those coordinates to this client — so the
                // close button would be dead exactly when the panel is where
                // it is supposed to be. `None` means "the whole surface".
                pane.surface.wl_surface().set_input_region(None);
                pane.surface.commit();
            }
        }
        let scale = self.scale;
        let Some(pane) = self.quickview.as_mut() else {
            return false;
        };
        let mut painted = pane.show();
        let resized = pane.place(rect, scale);
        // Everything the panel's pixels depend on has to be in here or the
        // repaint is skipped: the card's rect, which file it is showing, how
        // far its content is scrolled — and now how far its picture is zoomed
        // and dragged, which changes what is drawn without moving the card an
        // inch.
        let key = quickview_key(panel, generation, session);
        if pane.key == key {
            return painted;
        }
        use wayland_client::Proxy;
        if AppContext::frame_in_flight(&pane.surface.wl_surface().id()) {
            self.pending = true;
            return painted;
        }
        pane.key = key;
        let origin = (rect.left, rect.top);
        let started = qv_trace::now();
        pane.surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            canvas.save();
            canvas.translate((-origin.0, -origin.1));
            view::draw_quickview(canvas, f, session, resting);
            canvas.restore();
        });
        qv_trace::frame(session, rect, resized, started);
        painted = true;
        painted
    }

    /// The stack's horizontal bar.
    ///
    /// It spans the whole stack, so unlike the vertical bars it belongs to no
    /// column and cannot ride in one. Drawn into the window it would be buried:
    /// subsurfaces sit over the toplevel, so the columns would cover it. It
    /// gets a surface of its own instead, a strip along the bottom of the
    /// content area, stacked above every column.
    fn sync_pan_bar(&mut self, parent: &WlSurface, f: &Frame, viewport: Rect) -> bool {
        let Some(state) = f.pan_bar.filter(|s| s.scrollbar_opacity() > 0.0) else {
            return self.pan.as_mut().map(PaneSurface::hide).unwrap_or(false);
        };
        let bar_viewport = state.viewport();
        let mut strip = Rect::from_ltrb(
            bar_viewport.left,
            bar_viewport.bottom - PAN_BAR_STRIP,
            bar_viewport.right,
            bar_viewport.bottom,
        );
        if !strip.intersect(viewport) {
            return self.pan.as_mut().map(PaneSurface::hide).unwrap_or(false);
        }

        if self.pan.is_none() {
            self.pan = Self::create(parent, strip);
            self.stack_dirty = true;
        }
        let scale = self.scale;
        let Some(pane) = self.pan.as_mut() else {
            return false;
        };
        let mut painted = pane.show();
        pane.place(strip, scale);

        // The thumb slides as the stack pans and widens on hover, and neither
        // shows up in the opacity, so the geometry is what decides a repaint.
        let thumb = ScrollRenderer::thumb_rect(state).unwrap_or(Rect::new_empty());
        let key = hash_rect(thumb);
        let bar = state.scrollbar_opacity();
        if pane.key == key && pane.bar == bar {
            return painted;
        }
        use wayland_client::Proxy;
        if AppContext::frame_in_flight(&pane.surface.wl_surface().id()) {
            self.pending = true;
            return painted;
        }
        pane.key = key;
        pane.bar = bar;
        let origin = (strip.left, strip.top);
        let theme = f.theme;
        pane.surface.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            canvas.save();
            canvas.translate((-origin.0, -origin.1));
            ScrollRenderer::draw(canvas, state, theme, |_, _| {});
            canvas.restore();
        });
        painted = true;
        painted
    }

    /// Whether a paint is still owed, because the throttle turned one away.
    /// The caller has to keep the frame loop turning until this clears.
    pub fn pending(&self) -> bool {
        self.pending
    }

    fn hide_all(&mut self) -> bool {
        let mut changed = false;
        for pane in &mut self.panes {
            changed |= pane.hide();
        }
        if let Some(pan) = self.pan.as_mut() {
            changed |= pan.hide();
        }
        changed
    }

    /// Quick View's surface and where its card sits *within* that surface,
    /// for the pointer callback.
    ///
    /// The panel is centred on the display, so it routinely reaches outside
    /// the toplevel — and a pointer event over that part is never delivered
    /// to the toplevel at all. So the panel takes its own input, and is
    /// hit-tested in surface-local coordinates rather than the window's.
    ///
    /// The card's rect rather than the close button's, because the callback
    /// needs both that button and the content box under it: everything else
    /// the panel's own geometry is derived from the card, and deriving it
    /// twice from two published rects is how the two drift apart.
    pub fn quickview_target(&self) -> Option<(ObjectId, Rect)> {
        use wayland_client::Proxy;
        let pane = self.quickview.as_ref()?;
        let panel = Rect::from_xywh(
            quickview::SURFACE_MARGIN,
            quickview::SURFACE_MARGIN,
            pane.rect.width() - quickview::SURFACE_MARGIN * 2.0,
            pane.rect.height() - quickview::SURFACE_MARGIN * 2.0,
        );
        Some((pane.surface.wl_surface().id(), panel))
    }

    /// Where Quick View's panel actually rests, once it has been worked out.
    ///
    /// `None` until the compositor has answered with the output's geometry,
    /// and always `None` when the panel is centred on the window — the caller
    /// can compute that one itself. Hit-testing must use this rather than
    /// re-deriving a rect from the window size: a panel centred on the
    /// *display* is nowhere near the window's own centre.
    pub fn quickview_resting(&self) -> Option<Rect> {
        self.quickview_resting
    }

    /// The panel's material, handed to the compositor once.
    ///
    /// The shadow and the blur both belong here rather than in the card's own
    /// drawing. Painted client-side they would be re-recorded on every frame
    /// of an animation that resizes the surface as it goes — a full-card
    /// Gaussian per frame — and the blur could not see past this surface
    /// anyway. The compositor already has the pixels behind the panel and
    /// composites the shadow outside its bounds, so both are free here and
    /// impossible there.
    ///
    /// `material_popup` is 0xD8 — the toolkit's popup material is translucent
    /// by design, expecting exactly this blur behind it.
    fn style_quickview(pane: &PaneSurface, scale: f32) {
        let Some(style) = pane.surface.layer() else {
            return;
        };
        // Physical pixels, like every other measurement this protocol takes.
        let scale = scale as f64;
        // Matches the radius the card paints itself with, so the blur and the
        // shadow follow the corners instead of squaring them off.
        style.set_corner_radius(12.0 * scale);
        style.set_shadow(0.30, 28.0 * scale, 0.0, 10.0 * scale, 0.0, 0.0, 0.0);
        style.set_blend_mode(otto_kit::protocols::otto_surface_style_v1::BlendMode::BackgroundBlur);

        // Nothing here asks where the display is: that is worked out once per
        // opening and once per closing, in `resting_for`, because the answer is
        // relative to the window and the window moves.
        //
        // And it is asked for, rather than handed to the compositor to act on.
        // `set_output_placement` moves the *layer*, and for a subsurface the
        // layer carries the material — the blur, the shadow, the rounded
        // corners — while the client's pixels are put where
        // `wl_subsurface.set_position` says. Moving one without the other
        // leaves the card's frame in the middle of the screen and its contents
        // back over the window. Positioning both, as this client already does
        // for its columns, moves them together — and keeps the entrance, since
        // the icon it grows from is in the same coordinates as the answer.
    }

    fn create(parent: &WlSurface, rect: Rect) -> Option<PaneSurface> {
        let surface = SubsurfaceSurface::new(
            parent,
            rect.left as i32,
            rect.top as i32,
            rect.width().max(1.0) as i32,
            rect.height().max(1.0) as i32,
        )
        .ok()?;
        // Presentation only: input belongs to the toplevel, which is where all
        // of the browser's hit-testing already happens. Quick View is the
        // exception and undoes this — see `accept_input`.
        surface.wl_surface().set_input_region(Some(
            &AppContext::compositor_state()
                .wl_compositor()
                .create_region(AppContext::queue_handle(), ()),
        ));
        surface.commit();
        Some(PaneSurface {
            surface,
            rect: Rect::new_empty(),
            scroll: f32::NAN,
            bar: f32::NAN,
            key: 0,
            hidden: false,
            output_centered: false,
        })
    }
}

impl PaneSurface {
    /// Returns whether the surface had to be reallocated, which is the
    /// expensive half of a move.
    fn place(&mut self, rect: Rect, scale: f32) -> bool {
        if self.rect == rect {
            return false;
        }
        let resized = self.rect.width() != rect.width() || self.rect.height() != rect.height();
        self.rect = rect;
        if resized {
            self.surface
                .resize(rect.width().max(1.0) as i32, rect.height().max(1.0) as i32);
            // A resize invalidates what was painted.
            self.scroll = f32::NAN;
        }
        // A move does too, now that the bar is drawn in window coordinates
        // shifted by this rect's origin.
        self.bar = f32::NAN;
        self.surface.set_position(rect.left as i32, rect.top as i32);
        if let Some(style) = self.surface.layer() {
            // Claiming the size stops the compositor re-deriving both size and
            // position from the surface tree — see `ScrollSurfaces`, which
            // depends on the same rule.
            style.set_size(
                (rect.width() * scale) as f64,
                (rect.height() * scale) as f64,
            );
            style.set_position((rect.left * scale) as f64, (rect.top * scale) as f64);
        }
        self.surface.commit();
        resized
    }

    fn hide(&mut self) -> bool {
        if self.hidden {
            return false;
        }
        self.hidden = true;
        if let Some(style) = self.surface.layer() {
            style.set_opacity(0.0);
        }
        self.surface.commit();
        true
    }

    fn show(&mut self) -> bool {
        if !self.hidden {
            return false;
        }
        self.hidden = false;
        if let Some(style) = self.surface.layer() {
            style.set_opacity(1.0);
        }
        self.surface.commit();
        true
    }
}

/// Everything other than the scroll offset that decides what a column draws.
fn column_key(f: &Frame, depth: usize) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let pane = &f.panes[depth];
    let mut hasher = DefaultHasher::new();
    pane.entries.len().hash(&mut hasher);
    for entry in pane.entries.iter() {
        entry.name.hash(&mut hasher);
        entry.is_dir.hash(&mut hasher);
    }
    for index in 0..pane.entries.len() {
        pane.is_selected(index).hash(&mut hasher);
    }
    pane.cursor.hash(&mut hasher);
    pane.loading.hash(&mut hasher);
    pane.error.hash(&mut hasher);
    (depth == f.active).hash(&mut hasher);
    f.renaming.map(|(d, i)| (d == depth, i)).hash(&mut hasher);
    view::is_dark().hash(&mut hasher);
    hasher.finish()
}

/// Everything the panel's pixels depend on, as a repaint key.
///
/// It all has to be in here or the repaint is skipped: the card's rect, which
/// file it is showing, how far its content is scrolled, how far its picture is
/// zoomed and dragged — which changes what is drawn without moving the card an
/// inch — how its bars are presented, since they fade in and out over a
/// picture that is not moving, and whether the content has landed at all.
fn quickview_key(panel: Rect, generation: u64, session: &quickview::Session) -> u64 {
    hash_rect(panel)
        ^ generation.rotate_left(17)
        ^ (session.first_row as u64) << 1
        ^ hash_zoom(session.zoom).rotate_left(33)
        ^ hash_bars(session).rotate_left(7)
        // A decode landing does not change the generation — that was bumped
        // when it was asked for — so without this the content arriving is
        // invisible to the key, and the panel never repaints out of its
        // waiting state.
        ^ if session.loading { LOADING_KEY } else { 0 }
}

/// The content key's contribution for a panel that is still waiting for its
/// decode. Any value with a good spread of bits does; this is the golden-ratio
/// constant the hashers in this file use for the same purpose.
const LOADING_KEY: u64 = 0x9E37_79B9_7F4A_7C15;

/// A rect as a repaint key. Positions are the whole of what a bar draws, so
/// its geometry is its identity.
/// The zoom, as a repaint key contribution. Bit patterns rather than values,
/// like [`hash_rect`]: a float has no `Hash`, and rounding one to compare it
/// would let a slow pinch stall.
/// How the pan's scrollbars are presented — how faded in each is, and how
/// far each has widened under the pointer.
fn hash_bars(session: &quickview::Session) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let (horizontal, vertical) = session.pan_bars();
    let mut hasher = DefaultHasher::new();
    for state in [horizontal, vertical] {
        for value in [
            state.scrollbar_opacity(),
            state.scrollbar_expansion(),
            state.offset(),
        ] {
            value.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn hash_zoom(zoom: otto_kit::preview::Zoom) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for value in [
        zoom.scale,
        zoom.offset.0,
        zoom.offset.1,
        zoom.band.0,
        zoom.band.1,
    ] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_rect(rect: Rect) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for value in [rect.left, rect.top, rect.right, rect.bottom] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Per-frame timing for the Quick View entrance and exit.
///
/// The two animations run the same geometry in opposite directions, so if one
/// is smooth and the other is not, the difference is either in what a frame
/// costs or in how far apart the frames land — and those are two different
/// bugs. `OTTO_FILES_QV_TRACE=1` prints a line per painted frame.
mod qv_trace {
    use std::cell::Cell;
    use std::time::Instant;

    use skia_safe::Rect;

    use crate::quickview::Session;

    thread_local! {
        static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    fn enabled() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("OTTO_FILES_QV_TRACE").is_some())
    }

    pub fn now() -> Option<Instant> {
        enabled().then(Instant::now)
    }

    pub fn frame(session: &Session, rect: Rect, resized: bool, started: Option<Instant>) {
        let Some(started) = started else { return };
        let gap = LAST.with(|last| {
            let gap = last.get().map(|prev| started.duration_since(prev));
            last.set(Some(started));
            gap
        });
        let (direction, t) = match session.closing {
            Some(_) => ("out", session.exit_t()),
            None => ("in ", session.entrance_t()),
        };
        // The pause between one animation and the next is not a gap worth
        // reading, so it is left blank rather than reported as a stall.
        let gap_ms = match gap {
            Some(gap) if gap.as_millis() < 2000 => format!("{:>6.1}", gap.as_secs_f32() * 1000.0),
            _ => "     -".to_string(),
        };
        eprintln!(
            "qv {direction} t={t:.2} @({:>6.0},{:>5.0}) {:>4.0}x{:<4.0} {} paint {:>5}us  gap {gap_ms}ms",
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            if resized { "resize" } else { "      " },
            started.elapsed().as_micros(),
        );
    }
}

/// Where the panel rests when it is centred on the display, in window points.
///
/// `None` until the compositor has answered — the request goes out when the
/// surface is created, and the reply lands a round trip later, so the first
/// frame or two of an entrance are still centred on the window. Those frames
/// are the smallest ones, at the file's icon, so the correction is invisible.
fn centered_resting(pane: &PaneSurface, scale: f32) -> Option<Rect> {
    use wayland_client::Proxy;

    let style = pane.surface.layer()?;
    let frame = AppContext::output_frame(&style.id())?;
    if std::env::var_os("OTTO_FILES_QV_TRACE").is_some() {
        eprintln!("qv output_frame(px) = {frame:?} scale={scale}");
    }
    let (x, y, width, height) = frame;
    if width <= 0.0 || height <= 0.0 || scale <= 0.0 {
        return None;
    }
    // The answer is in the same pixels the positions are set in; the rest of
    // this module works in points.
    let (x, y, width, height) = (x / scale, y / scale, width / scale, height / scale);

    // The panel's share of the display, floored the same way it is floored
    // against a window, and never quite touching the edges.
    let panel_width = (width * quickview::PANEL_FRACTION)
        .max(quickview::PANEL_MIN.0)
        .min(width - 32.0);
    let panel_height = (height * quickview::PANEL_FRACTION)
        .max(quickview::PANEL_MIN.1)
        .min(height - 32.0);

    Some(Rect::from_xywh(
        x + (width - panel_width) / 2.0,
        y + (height - panel_height) / 2.0,
        panel_width.max(1.0),
        panel_height.max(1.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel repaints only when its key changes, and the decode landing
    /// changes nothing else: same file, same request, same rect, same zoom.
    /// If it does not move the key, the card stays on "Opening preview…" for
    /// as long as the user does not touch anything.
    #[test]
    fn a_decode_landing_moves_the_panel_key() {
        let resting = quickview::panel_rect(1100.0, 700.0);
        let anchor = Rect::new_empty();
        let opened_at = std::time::Instant::now();
        let generation = 7;

        let waiting = quickview::Session::waiting("notes.txt".into(), false, anchor, opened_at);
        let landed = quickview::Session::new(
            otto_kit::preview::Preview::Text {
                lines: vec!["hello".into()],
                truncated: false,
                language: String::new(),
            },
            "notes.txt".into(),
            anchor,
            opened_at,
        );

        assert_ne!(
            quickview_key(resting, generation, &waiting),
            quickview_key(resting, generation, &landed),
        );
    }
}
