//! Miller columns as their own Wayland subsurfaces.
//!
//! The browser normally paints every column into the window's single buffer,
//! which means a scroll in one column repaints the whole window and — because
//! otto-kit damages the entire buffer on commit — tells the compositor that
//! everything changed. Measured on the `scroll_ab` example, that is 82 full
//! window repaints across one fling.
//!
//! Here the stack is a horizontal container
//! ([`otto_kit::components::scroll::ScrollSurfaces::container`]) clipped to the
//! file area, and each column is a vertical pane inside it, placed once in the
//! stack's content coordinates: a clip surface the column's size, and inside it
//! a band of rows taller than the column. A frame of scrolling moves a band
//! with `otto_surface_style_v1` and paints nothing; panning the stack moves the
//! container's band, and every column rides along untouched. Rows are painted
//! again only when a glide nears the edge of a band, or when what a column
//! shows changes. The docked preview's player sits in the stack too, beside the
//! last column. The stack's bar is the container's own.
//!
//! **Input is not routed here.** Every column's surfaces pass the pointer
//! through, so events fall straight to the toplevel and the browser keeps
//! hit-testing in window coordinates exactly as it always has. Without that,
//! `Window::on_pointer_event` — which filters to the toplevel's own surface —
//! would silently stop seeing anything over the columns.
//!
//! The rows are all a band carries, on a transparent ground. The file area's
//! paper is the window's, underneath, and does not move. Everything that moves
//! with the stack is in it: the active column's tint is the colour of that
//! column's clip, and the hairline down each column's edge, the line an empty
//! or loading column shows and the docked preview are surfaces of their own —
//! so a pan moves the stack and the window repaints nothing.

use otto_kit::app_runner::AppContext;
use otto_kit::components::scroll::{Axis, Fill, Paint, PlacedSurface, ScrollSurfaces};
use otto_kit::prelude::*;
use otto_kit::theme::Theme;
use otto_kit::typography::styles;
use skia_safe::{Color, Rect};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::quickview;
use crate::scene;
use crate::view::{self, Frame, PaneData, ViewMode};

/// Height of the box a column's status line is painted into.
const STATUS_H: f32 = 40.0;

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
/// `OTTO_FILES_QV_CENTER=0` opts out and goes back to centring on the window.
pub fn quickview_centered() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        !matches!(
            std::env::var("OTTO_FILES_QV_CENTER").as_deref(),
            Ok("0") | Ok("false")
        )
    })
}

/// What the palette's surface needs in order to paint and place itself.
///
/// The card is measured in window points on both sides of this seam — the
/// geometry helpers in [`view`] all measure from the window's edges, and the
/// host hit-tests in the same space — so the surface's own coordinates are
/// arrived at by a single translate rather than by a second set of rects.
pub struct PaletteFrame {
    /// Where the card rests before any drag.
    pub resting: Rect,
    /// Where it actually is, once dragged.
    pub card: Rect,
    /// The non-editable prefix in front of the field, while an argument is
    /// being answered.
    pub prompt: Option<String>,
    /// Shown in place of the list.
    pub message: Option<String>,
    pub rows: Vec<crate::app::PaletteRowData>,
    /// Where the list has scrolled to, and the state of its bar.
    pub scroll: otto_kit::components::scroll::ScrollState,
    /// How fast the list is gliding, so its band is painted ahead of it.
    pub velocity: f32,
    /// Changes whenever the field would paint differently: its text, caret,
    /// selection, focus or blink. The field paints itself, so this side
    /// cannot tell by looking.
    pub field_key: u64,
}

impl PaletteFrame {
    /// Everything the card paints, less the rows and the scroll: what decides
    /// whether the card has to be painted again.
    fn card_key(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.prompt.hash(&mut hasher);
        self.message.hash(&mut hasher);
        self.rows.is_empty().hash(&mut hasher);
        hash_rect(self.resting).hash(&mut hasher);
        self.field_key.hash(&mut hasher);
        view::is_dark().hash(&mut hasher);
        otto_kit::frosting::enabled().hash(&mut hasher);
        hasher.finish()
    }

    /// Everything the rows paint, less the scroll.
    fn list_key(&self, width: f32) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        for row in &self.rows {
            std::mem::discriminant(&row.kind).hash(&mut hasher);
            if let view::PaletteRowKind::Preview { conflict, excluded } = row.kind {
                (conflict, excluded).hash(&mut hasher);
            }
            row.title.hash(&mut hasher);
            row.badge.hash(&mut hasher);
            row.subtitle.hash(&mut hasher);
            row.shortcut.hash(&mut hasher);
            row.highlighted.hash(&mut hasher);
        }
        width.to_bits().hash(&mut hasher);
        view::is_dark().hash(&mut hasher);
        hasher.finish()
    }

    /// Borrow the owned text as the view wants it.
    fn data(&self) -> view::PaletteData<'_> {
        view::PaletteData {
            prompt: self.prompt.as_deref(),
            rows: self
                .rows
                .iter()
                .map(|row| view::PaletteRow {
                    kind: row.kind,
                    title: &row.title,
                    badge: row.badge.as_deref(),
                    subtitle: row.subtitle.as_deref(),
                    shortcut: row.shortcut.as_deref(),
                    highlighted: row.highlighted,
                })
                .collect(),
            message: self.message.as_deref(),
            scroll: Some(self.scroll),
        }
    }
}

/// One column's surfaces, and what its band was last painted from.
struct ColumnPane {
    surfaces: ScrollSurfaces,
    /// Identity of everything other than the scroll that the rows draw.
    key: u64,
    /// The hairline down the column's trailing edge.
    divider: Option<Fill>,
    /// The loading, empty or error line, in place of rows.
    status: Option<PlacedSurface>,
}

impl ColumnPane {
    /// Take the column and everything beside it out of sight.
    fn hide(&mut self) -> bool {
        let mut changed = self.surfaces.set_hidden(true);
        if let Some(divider) = self.divider.as_mut() {
            changed |= divider.set_hidden(true);
        }
        if let Some(status) = self.status.as_mut() {
            changed |= status.set_hidden(true);
        }
        changed
    }
}

/// The per-column subsurfaces, pooled the way the scene pools its pane layers.
#[derive(Default)]
pub struct PaneSurfaces {
    /// The column stack: a container clipped to the file area, panned
    /// sideways, holding the columns and the preview's player.
    stack: Option<ScrollSurfaces>,
    columns: Vec<ColumnPane>,
    /// The Quick View panel, in a surface of its own over everything.
    quickview: Option<PlacedSurface>,
    /// The command palette, in a surface of its own so it can be dragged clear
    /// of the window.
    palette: Option<PlacedSurface>,
    /// The palette's rows, a scroll pane inside its card, and what they were
    /// last painted from.
    palette_list: Option<ScrollSurfaces>,
    palette_list_key: u64,
    /// The display the palette may be dragged around, in window points, and
    /// whether an answer has been asked for and not yet arrived.
    ///
    /// The client is never told where its own window is, so this is the only
    /// way to know how far the card may go before it leaves the screen — the
    /// clamp the window's own edges used to stand in for.
    palette_display: Option<Rect>,
    palette_asked: bool,
    /// The palette's pointer, on a surface that does not move.
    ///
    /// Transparent, input-only, the size of the display, and stacked over the
    /// card while the palette is up. Every press and every motion the palette
    /// cares about arrives here, in a frame that stays put for the length of
    /// a drag — which is what makes the drag exact. See [`Self::sync_palette`]
    /// for why the card cannot take its own pointer.
    catcher: Option<PlacedSurface>,
    /// The docked preview column's video player, in a surface of its own so
    /// its per-frame repaints never touch the toplevel. A child of the stack,
    /// sized to the video's shape and placed over the column's stage; see
    /// [`Self::sync_preview_video`].
    preview_video: Option<PlacedSurface>,
    /// The docked preview column itself — its paper, picture and caption — in
    /// the stack beside the last column. See [`Self::sync_preview_pane`].
    preview_pane: Option<PlacedSurface>,
    preview_divider: Option<Fill>,
    /// A surface was created in the stack, which puts it on top of its
    /// siblings there.
    stack_children_dirty: bool,
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
    /// The display the panel is centred on, in window points, frozen for the
    /// length of one opening or one closing.
    ///
    /// Recomputed only at those two moments, and never in between. The
    /// compositor's answer is relative to the *window*, so anything that moves
    /// the window — tiling it, dragging it — changes what it means. Following
    /// that continuously would drag the panel across the screen mid-animation
    /// and, when the window moves far enough, off it. The panel's own rect is
    /// derived from this on every pass, since expanding it changes the rect
    /// without changing where the display is.
    quickview_display: Option<Rect>,
    /// Where the panel was last placed to rest, whichever space it rests in.
    /// What the pointer handler measures against.
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
}

impl PaneSurfaces {
    pub fn new() -> Self {
        Self::default()
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
        // Outside column view there are no columns: what is left here is Quick
        // View's surface. The preview's player is in the stack, and goes with
        // it.
        if f.mode != ViewMode::Columns {
            let mut changed = self.hide_all();
            changed |= self.sync_quickview(parent, f, quickview);
            self.restack(parent);
            return changed;
        }
        let viewport = view::content_viewport(f.width, f.height, ViewMode::Columns);
        let mut painted = false;

        if self.stack.is_none() {
            match ScrollSurfaces::container(parent, viewport, Axis::Horizontal) {
                Ok(mut stack) => {
                    stack.set_input_passthrough();
                    self.stack = Some(stack);
                    self.stack_dirty = true;
                }
                Err(_) => return false,
            }
        }
        let Some(stack) = self.stack.as_mut() else {
            return false;
        };
        painted |= stack.set_hidden(false);
        stack.set_viewport(viewport);
        if let Some(pan) = f.pan_bar {
            // A container has no content to paint and nothing to paint ahead
            // of, so the pan's speed is no use to it.
            painted |= stack.sync_state(pan, 0.0, f.theme, |_, _| {}) || stack.waiting();
        }
        let band = stack.band_surface().clone();

        // What of the stack the file area shows, along it.
        let (shown_from, shown_to) = (f.pan, f.pan + viewport.width());
        for depth in 0..f.panes.len() {
            // Placed once, in the stack's own coordinates: the pan moves the
            // stack, never a column.
            let rect = Rect::from_xywh(
                depth as f32 * f.miller_w,
                0.0,
                f.miller_w,
                viewport.height(),
            );
            if depth >= self.columns.len() {
                match ScrollSurfaces::new(&band, rect, skia_safe::Color::TRANSPARENT) {
                    Ok(mut surfaces) => {
                        surfaces.set_input_passthrough();
                        self.columns.push(ColumnPane {
                            surfaces,
                            key: 0,
                            divider: None,
                            status: None,
                        });
                        self.stack_children_dirty = true;
                    }
                    Err(_) => break,
                }
            }

            // A column panned out of the file area is taken out of sight, so
            // nothing about it is composited or kept painted.
            let column = &mut self.columns[depth];
            if rect.right <= shown_from || rect.left >= shown_to {
                painted |= column.hide();
                continue;
            }
            painted |= column.surfaces.set_hidden(false);
            column.surfaces.set_viewport(rect);
            // The active column is a shade off the ground: the colour of its
            // clip, one pixel stretched, so which column has the keyboard
            // repaints neither the rows nor the window.
            column.surfaces.set_background(if depth == f.active {
                f.theme.fill_quaternary
            } else {
                Color::TRANSPARENT
            });
            painted |= sync_divider(
                &mut column.divider,
                &band,
                rect,
                f.theme,
                &mut self.stack_children_dirty,
            );
            painted |= sync_status(
                &mut column.status,
                &band,
                rect,
                &f.panes[depth],
                f.theme,
                &mut self.pending,
                &mut self.stack_children_dirty,
            );

            // Anything the rows show other than where they are scrolled to
            // repaints the band; the scroll itself only moves it.
            let key = column_key(f, depth);
            if column.key != key {
                column.key = key;
                column.surfaces.invalidate();
            }
            let Some(state) = f.panes[depth].bar else {
                continue;
            };
            // A step held back until the last one is on screen still counts:
            // the column has the scroll in hand, and the window has nothing
            // to repaint for it.
            let width = rect.width();
            painted |= column.surfaces.sync_state(
                state,
                f.panes[depth].velocity,
                f.theme,
                |canvas, band| scene::paint_column_band(canvas, f, depth, width, band),
            ) || column.surfaces.waiting();
        }

        for column in self.columns.iter_mut().skip(f.panes.len()) {
            painted |= column.hide();
        }
        painted |= self.sync_preview_pane(&band, f, viewport, (shown_from, shown_to));
        painted |= self.sync_preview_video(&band, f, viewport, quickview.is_some());
        painted |= self.sync_quickview(parent, f, quickview);
        self.restack(parent);
        painted
    }

    /// Put the sibling surfaces back into a known order, bottom to top: in the
    /// window the stack, then the palette and Quick View; in the stack the
    /// columns in depth order, then the preview's player.
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
        if std::mem::take(&mut self.stack_children_dirty) {
            if let Some(stack) = self.stack.as_ref() {
                // Columns, then what sits on them, then the preview and its
                // player, and the hairlines over everything — a hairline runs
                // down the seam between two neighbours and belongs to neither.
                let mut below: Option<WlSurface> = None;
                let mut stack_on = |place: &dyn Fn(&WlSurface), surface: &WlSurface| {
                    if let Some(below) = &below {
                        place(below);
                    }
                    below = Some(surface.clone());
                };
                for column in &self.columns {
                    stack_on(
                        &|b| column.surfaces.place_above(b),
                        column.surfaces.clip_surface(),
                    );
                }
                let beside = self.columns.iter().filter_map(|c| c.status.as_ref());
                let player = [self.preview_pane.as_ref(), self.preview_video.as_ref()];
                for pane in beside.chain(player.into_iter().flatten()) {
                    stack_on(&|b| pane.place_above(b), pane.wl_surface());
                }
                let dividers = self.columns.iter().filter_map(|c| c.divider.as_ref());
                for fill in dividers.chain(self.preview_divider.as_ref()) {
                    stack_on(&|b| fill.place_above(b), fill.wl_surface());
                }
                // The order is the stack band's pending state.
                stack.band_surface().commit();
            }
        }
        if !std::mem::take(&mut self.stack_dirty) {
            return;
        }
        let mut below: Option<WlSurface> = self
            .stack
            .as_ref()
            .map(|stack| stack.clip_surface().clone());
        let overlays = [
            self.palette.as_ref(),
            self.catcher.as_ref(),
            self.quickview.as_ref(),
        ];
        for pane in overlays.into_iter().flatten() {
            if let Some(below) = &below {
                pane.place_above(below);
            }
            below = Some(pane.wl_surface().clone());
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

        // Once per open, and again when the exit starts — the two moments the
        // answer can actually differ.
        let placement = session.closing.is_some();
        if self.quickview_placement != Some(placement) {
            self.quickview_placement = Some(placement);
            self.quickview_awaiting = true;
            pane.ask_output_frame();
        }
        // The *old* rect stays in force until the new answer lands. Nulling it
        // here is what made the panel snap to the window's centre and back.
        if self.quickview_awaiting {
            if let Some(rect) = pane.output_frame() {
                self.quickview_display = Some(rect);
                self.quickview_awaiting = false;
            }
        }
        self.quickview_display
            .map(|display| quickview::resting_in(display, session.expanded))
    }

    /// The command palette, on a surface of its own.
    ///
    /// Painted into the window the card could not be dragged past the window's
    /// edge, and its material could not blur anything but the listing it was
    /// covering. Here the compositor owns both: the card goes where it is put,
    /// and the frost samples the desktop behind the window.
    ///
    /// It takes its own pointer input, like Quick View and for the same
    /// reason — dragged clear of the window, the card is over pixels the
    /// toplevel is never told about. The keyboard is untouched: a subsurface
    /// takes no keyboard focus, so the palette's keys keep arriving at the
    /// toplevel exactly as they always have.
    /// Called on its own rather than from [`Self::sync`], because painting the
    /// card needs the palette borrowed mutably — its text field renders itself
    /// — and the `Frame` the rest of this module works from holds the browser
    /// borrowed for as long as it lives.
    pub fn sync_palette(
        &mut self,
        parent: &WlSurface,
        theme: &otto_kit::theme::Theme,
        (width, height): (f32, f32),
        palette: Option<&PaletteFrame>,
        paint_field: impl FnOnce(&skia_safe::Canvas),
    ) -> bool {
        let Some(palette) = palette else {
            // Asked afresh on the next open: the window may have been moved
            // to another display in between.
            self.palette_asked = false;
            // The catcher is dropped rather than pooled: it is the size of the
            // display, and a buffer that size is not worth holding for the
            // next Ctrl+P.
            let had_catcher = self.catcher.take().is_some();
            return self
                .palette
                .as_mut()
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false)
                | had_catcher;
        };
        // Exactly the card, not the card plus slack: the compositor rounds and
        // frosts the *surface's* rect, and a surface even a point larger is
        // rounded along a different curve from the one the card paints — the
        // frost and the hairline part company at every corner.
        let rect = palette.card;

        if self.palette.is_none() {
            self.palette = PlacedSurface::new(parent, rect).ok();
            self.stack_dirty = true;
            if let Some(pane) = self.palette.as_ref() {
                Self::style_palette(pane);
                // The card takes no pointer input. Its pointer is answered by
                // the catcher below, never by the card itself: pointer
                // positions arrive relative to the surface under the pointer,
                // and a surface that is being dragged is a moving ruler — each
                // motion event re-applies a correction that is already in
                // flight, and the card runs away.
            }
        }
        let mut painted = self.sync_palette_catcher(parent, width, height);
        if let Some(pane) = self.palette.as_mut() {
            // The card is pooled between opens, so a corner or frosting
            // setting changed while it was closed is picked up here.
            if pane.is_hidden() {
                Self::style_palette(pane);
            }
            painted |= pane.set_hidden(false);
            pane.set_rect(rect);

            // Painted only when the card itself changed: the rows and their
            // scroll are the list pane's, below, and a drag is the surface's
            // position. The field paints itself, so the frame carries a key
            // for it.
            //
            // The card is drawn where it *rests*, in window points, and the
            // drag is carried entirely by the surface's position. Drawing the
            // dragged rect into a surface that had already been moved would
            // apply the offset twice.
            let shift = (-palette.resting.left, -palette.resting.top);
            let data = palette.data();
            let paint = pane.paint(palette.card_key(), |canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
                canvas.save();
                canvas.translate(shift);
                view::draw_palette(canvas, theme, width, &data);
                // The field's text, over the box the card left for it — the
                // same two-step the rename and location fields take. Still in
                // window points, inside the same translate, so the caller
                // works in one coordinate space rather than two.
                paint_field(canvas);
                canvas.restore();
            });
            painted |= self.painted(paint);
        }
        painted |= self.sync_palette_list(palette, theme, width);
        // On every path, not only the painting one: the catcher may have just
        // been created, and it has to land above the card before the first
        // press or that press goes to the card's empty region instead.
        self.restack(parent);
        painted
    }

    /// The palette's rows: a vertical scroll pane inside the card, over the
    /// list's viewport. A scroll moves its band; the rows are painted again
    /// only when what they show changes — the highlight among them, since a
    /// highlighted row's text changes colour with it.
    fn sync_palette_list(&mut self, palette: &PaletteFrame, theme: &Theme, width: f32) -> bool {
        let Some(card) = self.palette.as_ref() else {
            return false;
        };
        let data = palette.data();
        // In window points where the card rests, which is what the rows are
        // measured in; the pane itself is placed in the card's coordinates.
        let viewport = view::palette_list_rect(width, &data.rows, data.message.is_some());
        let local = viewport.with_offset((-palette.resting.left, -palette.resting.top));
        if self.palette_list.is_none() {
            match ScrollSurfaces::new(card.wl_surface(), local, Color::TRANSPARENT) {
                Ok(mut list) => {
                    list.set_input_passthrough();
                    self.palette_list = Some(list);
                }
                Err(_) => return false,
            }
        }
        let Some(list) = self.palette_list.as_mut() else {
            return false;
        };
        if data.rows.is_empty() || local.height() <= 0.0 {
            return list.set_hidden(true);
        }
        let mut painted = list.set_hidden(false);
        list.set_viewport(local);
        let key = palette.list_key(width);
        if self.palette_list_key != key {
            self.palette_list_key = key;
            list.invalidate();
        }
        painted |= list.sync_state(&palette.scroll, palette.velocity, theme, |canvas, _band| {
            // The band's canvas starts at the list's first row.
            canvas.translate((-viewport.left, -viewport.top));
            view::draw_palette_rows(canvas, theme, width, &data);
        }) || list.waiting();
        painted
    }

    /// The palette's material, handed to the compositor once.
    ///
    /// The frost is the point of the surface as much as the dragging is. The
    /// compositor has the pixels behind the window and blurs *and tints* them,
    /// which is what lifts the card off a listing of the same colour; it also
    /// casts the shadow outside the card's bounds, where a shadow is actually
    /// visible. Neither is possible in the window's own buffer.
    fn style_palette(pane: &PlacedSurface) {
        let Some(style) = pane.style() else {
            return;
        };
        // Physical pixels for the shadow; the radius alone is in points — the
        // compositor scales it itself, and pre-scaling it here rounded the clip
        // at twice the card's radius on a 2x display.
        let scale = AppContext::fractional_scale();
        // The radius the card paints itself with, so the blur and the shadow
        // follow the corners instead of squaring them off — and the hairline
        // meets the clip rather than sitting inside a rounder one.
        style.set_corner_radius(crate::view::palette_radius() as f64);
        style.set_shadow(0.28, 24.0 * scale, 0.0, 8.0 * scale, 0.0, 0.0, 0.0);
        style.set_blend_mode(if otto_kit::frosting::enabled() {
            otto_kit::protocols::otto_surface_style_v1::BlendMode::BackgroundBlur
        } else {
            otto_kit::protocols::otto_surface_style_v1::BlendMode::Normal
        });
    }

    /// The surface the palette's pointer arrives on, and where that surface
    /// sits in window points — the catcher, not the card. The pointer callback
    /// adds the rect's origin to what the compositor reports and is back in
    /// window points, against a rect that does not change under it mid-drag.
    pub fn palette_target(&self) -> Option<(ObjectId, Rect)> {
        use wayland_client::Proxy;
        let pane = self.catcher.as_ref().filter(|pane| !pane.is_hidden())?;
        Some((pane.wl_surface().id(), pane.rect()))
    }

    /// Keep the catcher covering the display — or the window, until the
    /// compositor has said where the display is.
    ///
    /// It is never moved during a drag: its rect only changes when the answer
    /// about the display arrives, which is once, at the start of a session,
    /// before anyone has had time to take hold of anything.
    fn sync_palette_catcher(&mut self, parent: &WlSurface, width: f32, height: f32) -> bool {
        let rect = self
            .palette_display
            .unwrap_or_else(|| Rect::from_wh(width, height));
        if self.catcher.is_none() {
            self.catcher = PlacedSurface::new(parent, rect).ok();
            self.stack_dirty = true;
            if let Some(pane) = self.catcher.as_mut() {
                pane.set_takes_input(true);
            }
        }
        let Some(pane) = self.catcher.as_mut() else {
            return false;
        };
        let mut painted = pane.set_hidden(false);
        pane.set_rect(rect);
        // Painted once per size: there is nothing on it, but a surface with no
        // buffer is not mapped and takes no input.
        let paint = pane.paint(0, |canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });
        painted |= self.painted(paint);
        painted
    }

    /// Whether a paint happened, noting one the throttle held back so the
    /// caller keeps the loop turning until it lands.
    fn painted(&mut self, paint: Paint) -> bool {
        match paint {
            Paint::Painted => true,
            Paint::Held => {
                self.pending = true;
                false
            }
            Paint::Unchanged => false,
        }
    }

    /// The display the palette is on, in window points, or `None` until the
    /// compositor has answered.
    ///
    /// Asked once per opening. The answer is relative to the window, so it
    /// only means anything for as long as the window stays put — which for the
    /// length of one palette session it does.
    pub fn palette_display(&mut self) -> Option<Rect> {
        let pane = self.palette.as_ref()?;
        if !self.palette_asked {
            self.palette_asked = true;
            pane.ask_output_frame();
        }
        if let Some(rect) = pane.output_frame() {
            self.palette_display = Some(rect);
        }
        self.palette_display
    }

    /// The Quick View panel.
    ///
    /// Drawn into the window it would be buried: the column surfaces sit over
    /// the toplevel, so a panel painted underneath them is a panel nobody can
    /// see. It gets a surface of its own, stacked above every column and above
    /// the horizontal bar — the topmost thing this window puts on screen.
    ///
    /// Unlike the columns it answers for its own pointer: centred on the
    /// display it hangs outside the toplevel, where no event reaches the
    /// window. See [`Self::quickview_target`].
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
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false);
        };
        // Centred on the display when the compositor has told us where the
        // display is, and centred on the window until it has. Everything below
        // stays in window coordinates either way, which is what lets the
        // entrance keep growing out of the file's icon: the anchor and the
        // resting place are in the same space.
        // Where it rests, moved by however far it has been dragged by its
        // title strip. Folded in here rather than inside the session's own
        // geometry so that everything derived from the resting rect — the
        // surface's placement, the drawing, and the rect the pointer is
        // hit-tested against — is moved by the same amount.
        let resting = self
            .resting_for(session)
            .unwrap_or_else(|| {
                quickview::resting_in(Rect::from_wh(f.width, f.window_h()), session.expanded)
            })
            .with_offset(session.offset);
        self.quickview_resting = Some(resting);
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
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false);
        }

        if self.quickview.is_none() {
            self.quickview = PlacedSurface::new(parent, rect).ok();
            self.stack_dirty = true;
            if let Some(pane) = self.quickview.as_mut() {
                Self::style_quickview(pane);
                // A panel centred on the display hangs outside the toplevel,
                // and the pointer never reports those coordinates to this
                // client — so the close button would be dead exactly when the
                // panel is where it is supposed to be. The surface answers for
                // itself, and stops while it is hidden.
                pane.set_takes_input(true);
            }
        }
        let Some(pane) = self.quickview.as_mut() else {
            return false;
        };
        let mut painted = pane.set_hidden(false);
        pane.set_rect(rect);
        // Everything the panel's pixels depend on has to be in here or the
        // repaint is skipped: the card's rect, which file it is showing, how
        // far its content is scrolled — and now how far its picture is zoomed
        // and dragged, which changes what is drawn without moving the card an
        // inch.
        let key = quickview_key(panel, generation, session);
        let origin = (rect.left, rect.top);
        let paint = pane.paint(key, |canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
            canvas.save();
            canvas.translate((-origin.0, -origin.1));
            view::draw_quickview(canvas, f, session, resting);
            canvas.restore();
        });
        painted |= self.painted(paint);
        painted
    }
    /// The docked preview column, in the stack beside the last column: its
    /// paper, its picture or text and its caption, painted when what it shows
    /// changes and never for a pan. Hidden while the stack is panned away
    /// from it.
    fn sync_preview_pane(
        &mut self,
        stack: &WlSurface,
        f: &Frame,
        viewport: Rect,
        (shown_from, shown_to): (f32, f32),
    ) -> bool {
        let rect = Rect::from_xywh(
            f.panes.len() as f32 * f.miller_w,
            0.0,
            view::PREVIEW_W,
            viewport.height(),
        );
        let shown = rect.right > shown_from && rect.left < shown_to;
        let Some(data) = f.preview.as_ref().filter(|_| shown) else {
            let mut changed = self
                .preview_pane
                .as_mut()
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false);
            if let Some(divider) = self.preview_divider.as_mut() {
                changed |= divider.set_hidden(true);
            }
            return changed;
        };

        if self.preview_pane.is_none() {
            self.preview_pane = PlacedSurface::new(stack, rect).ok();
            self.stack_children_dirty = true;
        }
        let mut painted = sync_divider(
            &mut self.preview_divider,
            stack,
            rect,
            f.theme,
            &mut self.stack_children_dirty,
        );
        let Some(pane) = self.preview_pane.as_mut() else {
            return painted;
        };
        painted |= pane.set_hidden(false);
        pane.set_rect(rect);

        let key = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            data.name.hash(&mut hasher);
            data.first_row.hash(&mut hasher);
            data.decoded.is_some().hash(&mut hasher);
            // A video painted here re-records on every frame of it; on its own
            // surface it is none of this one's business.
            if !data.video_on_surface {
                data.video
                    .map(crate::quickview::Video::key)
                    .hash(&mut hasher);
            }
            view::is_dark().hash(&mut hasher);
            hash_rect(Rect::from_wh(rect.width(), rect.height())).hash(&mut hasher);
            hasher.finish()
        };
        let ground = view::content_ground();
        let (width, height) = (rect.width(), rect.height());
        let paint = pane.paint(key, |canvas| {
            canvas.clear(ground);
            view::preview_content(data, f.theme.clone())(canvas, width, height);
        });
        painted |= self.painted(paint);
        painted
    }

    /// The docked preview column's video, on its own subsurface.
    ///
    /// Sized to the video's shape and placed over the column's stage, so the
    /// picture updates without the toplevel — or the scene's cached preview
    /// picture — being touched. A child of the stack's band, placed in the
    /// stack's coordinates, so it pans and is clipped with the columns. Input
    /// stays with the toplevel, exactly like the columns: the browser's
    /// pointer routing already hit-tests the box in window coordinates (see
    /// `Browser::preview_video_pointer`).
    fn sync_preview_video(
        &mut self,
        stack: &WlSurface,
        f: &Frame,
        viewport: Rect,
        quickview_up: bool,
    ) -> bool {
        // Shown only for a video in the column view, and never behind the
        // Quick View panel — which is its own, larger player.
        let video = (f.mode == ViewMode::Columns && !quickview_up)
            .then(|| {
                f.preview
                    .as_ref()
                    .and_then(|p| p.video.map(|v| (v, p.info.len())))
            })
            .flatten();
        let Some((video, info_lines)) = video else {
            return self
                .preview_video
                .as_mut()
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false);
        };

        let snapshot = video.snapshot();
        // Held hidden until the worker has said what shape the video is. The
        // box is sized from the aspect, so showing before it is known would
        // put a full-height surface up and then resize it to the real box a
        // frame later — a visible snap. The dark column ground stands in for
        // the few milliseconds until Ready arrives, and the surface then
        // appears once, at the right size.
        let Some(aspect) = snapshot.aspect() else {
            return self
                .preview_video
                .as_mut()
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false);
        };

        // Unpanned, then moved from the window's coordinates into the stack's.
        let full = view::preview_pane_rect(f.panes.len(), f.height, 0.0, f.miller_w);
        let stage = view::preview_stage_rect(full, info_lines);
        let rect = view::preview_video_box(stage, Some(aspect))
            .with_offset((-viewport.left, -viewport.top));
        if rect.width() <= 1.0 {
            return self
                .preview_video
                .as_mut()
                .map(|pane| pane.set_hidden(true))
                .unwrap_or(false);
        }

        if self.preview_video.is_none() {
            self.preview_video = PlacedSurface::new(stack, rect).ok();
            self.stack_children_dirty = true;
        }
        let Some(pane) = self.preview_video.as_mut() else {
            return false;
        };
        let mut painted = pane.set_hidden(false);
        pane.set_rect(rect);

        let key = hash_rect(rect) ^ video.key().rotate_left(19);
        let theme = f.theme.clone();
        let origin = (rect.left, rect.top);
        let paint = pane.paint(key, |canvas| {
            let poster = snapshot
                .poster
                .as_ref()
                .and_then(otto_kit::preview::Pixels::to_image);
            canvas.clear(skia_safe::Color::TRANSPARENT);
            canvas.save();
            canvas.translate((-origin.0, -origin.1));
            otto_media_kit::view::draw_frame(
                canvas,
                rect,
                snapshot.frame.as_ref(),
                poster.as_ref(),
                &snapshot.state,
                otto_media_kit::view::Interaction {
                    scrubbing: snapshot.scrubbing,
                    transport_opacity: 1.0,
                },
                &theme,
            );
            canvas.restore();
        });
        painted |= self.painted(paint);
        painted
    }

    /// Whether a paint is still owed, because the throttle turned one away.
    /// The caller has to keep the frame loop turning until this clears.
    pub fn pending(&self) -> bool {
        self.pending
    }

    fn hide_all(&mut self) -> bool {
        // The columns and the player are the stack's children, and go with it.
        self.stack
            .as_mut()
            .is_some_and(|stack| stack.set_hidden(true))
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
            pane.rect().width() - quickview::SURFACE_MARGIN * 2.0,
            pane.rect().height() - quickview::SURFACE_MARGIN * 2.0,
        );
        Some((pane.wl_surface().id(), panel))
    }

    /// The display the panel is centred on, in window points, once the
    /// compositor has answered. What a drag of the title strip is bounded by:
    /// a panel dragged off the screen is one nobody can find the way back to.
    pub fn quickview_display(&self) -> Option<Rect> {
        self.quickview_display
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
    fn style_quickview(pane: &PlacedSurface) {
        let Some(style) = pane.style() else {
            return;
        };
        // Physical pixels for the shadow; the radius is in points, because the
        // compositor scales it itself.
        let scale = AppContext::fractional_scale();
        // Matches the radius the card paints itself with, so the blur and the
        // shadow follow the corners instead of squaring them off.
        style.set_corner_radius(12.0);
        style.set_shadow(0.30, 28.0 * scale, 0.0, 10.0 * scale, 0.0, 0.0, 0.0);
        style.set_blend_mode(if otto_kit::frosting::enabled() {
            otto_kit::protocols::otto_surface_style_v1::BlendMode::BackgroundBlur
        } else {
            otto_kit::protocols::otto_surface_style_v1::BlendMode::Normal
        });

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
}

/// The hairline down the trailing edge of `column`, in the stack's
/// coordinates: a point wide, centred on the edge — a kit [`Fill`], so it
/// costs a request to move and nothing to paint.
fn sync_divider(
    divider: &mut Option<Fill>,
    stack: &WlSurface,
    column: Rect,
    theme: &Theme,
    created: &mut bool,
) -> bool {
    if divider.is_none() {
        *divider = Fill::new(stack).ok();
        *created = true;
    }
    let Some(fill) = divider.as_mut() else {
        return false;
    };
    let line = Rect::from_xywh(column.right - 0.5, column.top, 1.0, column.height());
    fill.set_style(theme.fill_tertiary, 0.0) | fill.set_rect(line) | fill.set_hidden(false)
}

/// The line a column shows in place of rows — loading, empty, or why it could
/// not be read — centred on the column, on a small surface of its own.
fn sync_status(
    slot: &mut Option<PlacedSurface>,
    stack: &WlSurface,
    column: Rect,
    pane: &PaneData<'_>,
    theme: &Theme,
    pending: &mut bool,
    created: &mut bool,
) -> bool {
    let line = if let Some(error) = pane.error {
        Some((error.to_string(), theme.text_secondary))
    } else if pane.loading {
        Some((otto_kit::t_owned!("files-loading"), theme.text_tertiary))
    } else if pane.entries.is_empty() {
        Some((otto_kit::t_owned!("files-empty"), theme.text_tertiary))
    } else {
        None
    };
    let Some((text, color)) = line else {
        return slot
            .as_mut()
            .map(|pane| pane.set_hidden(true))
            .unwrap_or(false);
    };

    let rect = Rect::from_xywh(
        column.left,
        column.center_y() - STATUS_H / 2.0,
        column.width(),
        STATUS_H,
    );
    if slot.is_none() {
        *slot = PlacedSurface::new(stack, rect).ok();
        *created = true;
    }
    let Some(surface) = slot.as_mut() else {
        return false;
    };
    let mut painted = surface.set_hidden(false);
    surface.set_rect(rect);

    let key = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        (color.a(), color.r(), color.g(), color.b()).hash(&mut hasher);
        rect.width().to_bits().hash(&mut hasher);
        hasher.finish()
    };
    let (width, height) = (rect.width(), rect.height());
    match surface.paint(key, |canvas| {
        canvas.clear(Color::TRANSPARENT);
        Label::new(&text)
            .with_style(styles::BODY)
            .with_color(color)
            .centered_at(width / 2.0, height / 2.0)
            .render(canvas);
    }) {
        Paint::Painted => painted = true,
        Paint::Held => *pending = true,
        Paint::Unchanged => {}
    }
    painted
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
    // A thumbnail landing changes the rows without changing anything else
    // here; the store's epoch moves exactly when one does.
    f.thumbs.map(|store| store.epoch()).hash(&mut hasher);
    (depth == f.active).hash(&mut hasher);
    f.renaming.map(|(d, i)| (d == depth, i)).hash(&mut hasher);
    view::is_dark().hash(&mut hasher);
    // A background window's theme mutes the accent, so the selection this
    // column draws changes colour when the window loses focus.
    f.focused.hash(&mut hasher);
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
    // The card's *size*, not where it sits. Its drawing is translated to the
    // surface's own origin, so two panels of the same size are the same
    // pixels wherever they are — and a panel being dragged by its title
    // strip would otherwise repaint in full on every frame of the drag.
    hash_rect(Rect::from_wh(panel.width(), panel.height()))
        ^ generation.rotate_left(17)
        ^ (session.first_row as u64) << 1
        ^ hash_zoom(session.zoom).rotate_left(33)
        ^ hash_bars(session).rotate_left(7)
        // A decode landing does not change the generation — that was bumped
        // when it was asked for — so without this the content arriving is
        // invisible to the key, and the panel never repaints out of its
        // waiting state.
        ^ if session.loading { LOADING_KEY } else { 0 }
        // Words landing on a picture, and a selection moving over them,
        // change what is drawn without moving anything else the key sees.
        ^ session.words_epoch.rotate_left(23)
        ^ hash_selection(session).rotate_left(41)
        // A video changes what is drawn on every frame and every tick of its
        // clock, with nothing else about the panel moving.
        ^ session.video_key()
        // The working badge breathes while the recogniser reads the picture,
        // and nothing else about the panel moves for as long as it takes.
        ^ hash_recognising(session).rotate_left(11)
}

/// The working badge's breath, as a key contribution: the phase quantised to
/// the frames it is actually drawn in, so the panel repaints while the
/// recogniser runs and stops the moment it does. `0` when nothing is running,
/// which is also what a panel with no recogniser on it contributes.
fn hash_recognising(session: &quickview::Session) -> u64 {
    const STEPS_PER_SECOND: f32 = 25.0;
    match session.recognising_phase() {
        Some(phase) => (phase * STEPS_PER_SECOND) as u64 + 1,
        None => 0,
    }
}

/// Which words are selected on the panel's picture, as a key contribution.
fn hash_selection(session: &quickview::Session) -> u64 {
    match session.selection {
        Some(selection) => {
            let range = selection.range();
            ((*range.start() as u64 + 1) << 32) | (*range.end() as u64 + 1)
        }
        None => 0,
    }
}

/// The content key's contribution for a panel that is still waiting for its
/// decode. Any value with a good spread of bits does; this is the golden-ratio
/// constant the hashers in this file use for the same purpose.
const LOADING_KEY: u64 = 0x9E37_79B9_7F4A_7C15;

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

/// The zoom, as a repaint key contribution. Bit patterns rather than values,
/// like [`hash_rect`]: a float has no `Hash`, and rounding one to compare it
/// would let a slow pinch stall.
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

/// A rect as a repaint key: its geometry is its identity.
fn hash_rect(rect: Rect) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for value in [rect.left, rect.top, rect.right, rect.bottom] {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The working badge breathes, and nothing else about the panel moves
    /// while it does. Without the phase in the key the cached picture replays
    /// and the badge is a still glyph for the whole wait.
    #[test]
    fn a_running_recogniser_moves_the_panel_key() {
        let resting = quickview::panel_rect(1100.0, 700.0);
        let anchor = Rect::new_empty();
        let opened_at = std::time::Instant::now();
        let generation = 7;

        let mut session = quickview::Session::new(
            otto_kit::preview::Preview::Text {
                lines: vec!["hello".into()],
                truncated: false,
                language: String::new(),
            },
            "shot.png".into(),
            anchor,
            opened_at,
        );
        let idle = quickview_key(resting, generation, &session);

        // Started two frames ago, and started now: two different keys, so the
        // panel is redrawn between them.
        session.start_recognising(std::time::Instant::now());
        let running = quickview_key(resting, generation, &session);
        assert_ne!(idle, running, "a running recogniser does not move the key");

        session
            .start_recognising(std::time::Instant::now() - std::time::Duration::from_millis(200));
        assert_ne!(
            running,
            quickview_key(resting, generation, &session),
            "the badge's breath does not move the key"
        );

        // And it stops moving the moment the recogniser does.
        session.stop_recognising();
        assert_eq!(idle, quickview_key(resting, generation, &session));
    }

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
