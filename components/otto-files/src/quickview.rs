//! Quick View, embedded — the Space-bar preview.
//!
//! Quick View used to be a separate application reached over `org.otto.QuickView1`.
//! It is now a library this window embeds, because the preview wants to be a
//! subsurface of the file view and a subsurface's parent must be a `wl_surface`
//! owned by the same client: "parented to the browser" and "separate process"
//! are mutually exclusive. Being parented also dissolves the anchor problem —
//! the row's rect is already in this surface's coordinates — and hands
//! stacking, focus and dismissal to this window instead of leaving them to be
//! managed by hand.
//!
//! What is still a separate process is the *decoder*. Untrusted bytes are
//! parsed by a sandboxed worker that is this binary re-executed, which is why
//! `main` must call [`otto_quickview::run_worker_if_requested`] before anything
//! else. This module never interprets file bytes; it receives a validated
//! [`Preview`] and draws it.

use std::path::Path;
use std::time::Instant;

use otto_kit::preview::{Preview, Zoom};
use otto_quickview::decode::Request;
use otto_quickview::opening;
use skia_safe::Rect;

/// The title strip along the top of the panel: the file's name, and the
/// close button. The preview's content starts below it, so neither ever
/// draws over the other.
pub const TITLEBAR_H: f32 = 30.0;

/// The panel's share of the window, and the least it may shrink to. A preview
/// that filled the window would stop reading as something laid over the file
/// list; one that scaled without a floor would be useless in a small window.
pub const PANEL_FRACTION: f32 = 0.72;
pub const PANEL_MIN: (f32, f32) = (420.0, 320.0);

/// An open preview.
pub struct Session {
    pub preview: Preview,
    /// The file's own name, shown in the panel's title strip. The preview
    /// itself does not carry one — a decoded image knows nothing about where
    /// it came from — so the host puts it here when it opens the session.
    pub name: String,
    /// Scroll offset into a listing or a text preview. The host owns it, as it
    /// owns every other piece of interaction state.
    pub first_row: usize,
    /// How far an image preview is zoomed in, and how far it has been dragged
    /// about while it is. Lives here rather than in the toolkit for the same
    /// reason `first_row` does: the drawing half is canvas-pure and holds no
    /// interaction state. Reset by construction — a session is built afresh
    /// for every file, so changing file or closing the panel puts the picture
    /// back to fit without anything having to remember to.
    pub zoom: Zoom,
    /// The row this grew out of, in surface-local coordinates. Empty means
    /// "open in place" — a row scrolled out of view, or a panned-away Miller
    /// column.
    pub anchor: Rect,
    /// When the entrance started.
    pub opened_at: Instant,
    /// When the exit started, once it has. A closing session is no longer the
    /// window's open preview — it is only still on screen, going home.
    pub closing: Option<Instant>,
}

impl Session {
    /// How far into the entrance the panel is, 0 → 1.
    pub fn entrance_t(&self) -> f32 {
        let elapsed = self.opened_at.elapsed().as_secs_f32();
        (elapsed / opening::geometry_in().as_secs_f32()).clamp(0.0, 1.0)
    }

    /// How far into the exit the panel is, 0 → 1. Always 0 while open.
    pub fn exit_t(&self) -> f32 {
        let Some(started) = self.closing else {
            return 0.0;
        };
        (started.elapsed().as_secs_f32() / opening::geometry_out().as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Whether this session still has frames to run — arriving, or leaving.
    pub fn animating(&self) -> bool {
        match self.closing {
            Some(_) => self.exit_t() < 1.0,
            None => self.entrance_t() < 1.0,
        }
    }

    /// Where the panel is now: partway in, at rest, or partway back to the
    /// item it came from.
    pub fn panel(&self, resting: Rect) -> Rect {
        match self.closing {
            Some(_) => exit_at(self.anchor, resting, self.exit_t()),
            None => entrance_at(self.anchor, resting, self.entrance_t()),
        }
    }

    /// Scroll a listing or a text preview by `rows`, stopping at both ends.
    ///
    /// Images and cards do not scroll: they are laid out to fit, so there is
    /// nothing under the fold to reach. A zoomed image *does* have something
    /// under the fold, but that is a pan rather than a scroll — see
    /// [`Session::pan_by`], which the host reaches for first.
    pub fn scroll_by(&mut self, rows: i32, panel: Rect) {
        let total = match &self.preview {
            Preview::Text { lines, .. } => lines.len(),
            Preview::Rows { rows, .. } => rows.len(),
            _ => return,
        };
        let visible =
            otto_kit::preview::layout(panel, &self.preview, self.first_row, self.zoom).visible_rows;
        let max = total.saturating_sub(visible);
        let next = self.first_row as i64 + rows as i64;
        self.first_row = next.clamp(0, max as i64) as usize;
    }

    /// Whether a two-finger gesture over `panel` should move the picture
    /// rather than scroll the content.
    ///
    /// False for everything but an image, and false for an image at fit: one
    /// that fills no more than its box has nothing to pan to, so the gesture
    /// must go on meaning exactly what it meant before there was a zoom.
    /// Asked against the panel's content rect because a zoom clamped for one
    /// box is not clamped for another — resizing the window can leave a
    /// stored zoom with no slack left.
    pub fn pannable(&self, panel: Rect) -> bool {
        !otto_kit::preview::clamp_zoom(panel, &self.preview, self.zoom).is_fit()
    }

    /// Drag a zoomed image by `dx`, `dy` in the panel's own pixels, stopping
    /// where its edge reaches the edge of the content box.
    ///
    /// Returns whether anything moved, so a host that repaints on demand does
    /// not repaint for a gesture that was already against the stop.
    pub fn pan_by(&mut self, dx: f32, dy: f32, panel: Rect) -> bool {
        let asked = Zoom {
            scale: self.zoom.scale,
            offset: (self.zoom.offset.0 + dx, self.zoom.offset.1 + dy),
        };
        let next = otto_kit::preview::clamp_zoom(panel, &self.preview, asked);
        let moved = next != self.zoom;
        self.zoom = next;
        moved
    }

    /// Zoom an image to `scale` about `focus`, a point in the same
    /// coordinates as `panel`.
    ///
    /// Returns whether anything moved. The clamping — the range, the snap
    /// back to fit and the pan limits — all happens in the toolkit, so the
    /// panel and the file picker cannot end up with different ideas of how
    /// far a picture zooms.
    pub fn zoom_to(&mut self, scale: f32, focus: (f32, f32), panel: Rect) -> bool {
        let next = otto_kit::preview::zoom_about(panel, &self.preview, self.zoom, scale, focus);
        let moved = next != self.zoom;
        self.zoom = next;
        moved
    }
}

/// Where the panel rests, centred in a window of `width` × `height`.
pub fn panel_rect(width: f32, height: f32) -> Rect {
    let w = (width * PANEL_FRACTION).max(PANEL_MIN.0).min(width - 32.0);
    let h = (height * PANEL_FRACTION)
        .max(PANEL_MIN.1)
        .min(height - 32.0);
    Rect::from_xywh(
        (width - w) / 2.0,
        (height - h) / 2.0,
        w.max(1.0),
        h.max(1.0),
    )
}

/// Room around the panel in its own surface. The card has no shadow to spill
/// past its edge any more — only the antialiasing on its border — so this is
/// a single point, and the surface is the card.
pub const SURFACE_MARGIN: f32 = 1.0;

/// The panel's rect partway through the entrance.
///
/// Runs the same curve `opening` describes, in this process rather than through
/// a compositor transaction — the panel is drawn into this window's own
/// surface, so there is no separate surface for the compositor to transform.
pub fn entrance_at(anchor: Rect, resting: Rect, t: f32) -> Rect {
    let rect = opening::sample(to_opening(anchor), to_opening(resting), t);
    Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
}

/// The reverse: the panel on its way back to the item it grew out of.
pub fn exit_at(anchor: Rect, resting: Rect, t: f32) -> Rect {
    let rect = opening::sample_out(to_opening(anchor), to_opening(resting), t);
    Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
}

fn to_opening(rect: Rect) -> opening::Rect {
    if rect.is_empty() {
        return opening::Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    opening::Rect::new(rect.left, rect.top, rect.width(), rect.height())
}

/// Decode one file. **Blocks** until the sandboxed worker answers or its
/// deadline expires, so it must never be called on the UI thread.
///
/// `panel` is the resting rect in logical pixels and `scale` the output's
/// scale; the worker is asked for roughly twice that, so a scaled decode still
/// has detail to show when the panel is looked at closely.
pub fn decode(path: &Path, panel: Rect, scale: f32) -> Preview {
    let request = Request {
        width: ((panel.width() * scale * 2.0) as u32).clamp(64, 4096),
        height: ((panel.height() * scale * 2.0) as u32).clamp(64, 4096),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Request::default()
    };
    otto_quickview::decode_path(path, &request)
}

#[cfg(test)]
mod tests {
    use super::*;

    use otto_kit::preview::Pixels;

    /// A session on a picture wide and tall enough that it is scaled *down*
    /// to fit any panel these tests use, so the fit rect fills one axis of
    /// the content box exactly and the zoom arithmetic has something to bite
    /// on.
    fn image_session(width: u32, height: u32) -> Session {
        Session {
            preview: Preview::Pixels {
                pixels: Pixels {
                    width,
                    height,
                    intrinsic_width: width,
                    intrinsic_height: height,
                    data: vec![0; (width * height * 4) as usize],
                },
                pages: 1,
                page: 1,
            },
            name: "photo.png".into(),
            first_row: 0,
            zoom: Zoom::FIT,
            anchor: Rect::new_empty(),
            opened_at: Instant::now(),
            closing: None,
        }
    }

    fn text_session() -> Session {
        Session {
            preview: Preview::Text {
                lines: (0..200).map(|n| format!("line {n}")).collect(),
                truncated: false,
                language: String::new(),
            },
            name: "notes.txt".into(),
            first_row: 0,
            zoom: Zoom::FIT,
            anchor: Rect::new_empty(),
            opened_at: Instant::now(),
            closing: None,
        }
    }

    /// The content box of a panel resting in a window of a comfortable size.
    fn content() -> Rect {
        crate::view::quickview_content_rect(panel_rect(1100.0, 700.0))
    }

    #[test]
    fn a_pinch_zooms_between_fit_and_the_maximum() {
        let content = content();
        let mut session = image_session(2000, 1500);
        let centre = (content.center_x(), content.center_y());

        session.zoom_to(200.0, centre, content);
        assert_eq!(session.zoom.scale, Zoom::MAX);

        // Pinching back out lands on fit exactly, rather than a hair above it.
        session.zoom_to(1.005, centre, content);
        assert!(session.zoom.is_fit(), "{:?}", session.zoom);
    }

    /// A picture no larger than its box has nothing to pan to, so a
    /// two-finger scroll over one must go on meaning what it always meant.
    #[test]
    fn a_fitted_image_does_not_pan() {
        let content = content();
        let mut session = image_session(2000, 1500);
        assert!(!session.pannable(content));
        assert!(!session.pan_by(-120.0, -90.0, content));
        assert_eq!(session.zoom, Zoom::FIT);
    }

    /// Zoomed in, panning stops with the picture still covering the box —
    /// it can never be dragged off the panel and left showing nothing.
    #[test]
    fn a_zoomed_image_pans_but_cannot_be_dragged_off_the_panel() {
        let content = content();
        let mut session = image_session(2000, 1500);
        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);
        assert!(session.pannable(content));
        assert!(session.pan_by(-40.0, -30.0, content));

        // Far past any stop, in both directions, and the picture still
        // reaches both edges of the box it is looked at through.
        session.pan_by(-100_000.0, -100_000.0, content);
        let drawn = otto_kit::preview::layout(content, &session.preview, 0, session.zoom).content;
        let inner = otto_kit::preview::layout(content, &session.preview, 0, Zoom::FIT).inner;
        assert!(drawn.right >= inner.right, "{drawn:?} {inner:?}");
        assert!(drawn.bottom >= inner.bottom, "{drawn:?} {inner:?}");
        assert!(drawn.left <= inner.left, "{drawn:?} {inner:?}");
    }

    /// Zoom is an image affordance. A text preview keeps scrolling, and a
    /// pinch over one changes nothing.
    #[test]
    fn text_previews_scroll_and_do_not_zoom() {
        let content = content();
        let mut session = text_session();
        session.zoom_to(4.0, (content.center_x(), content.center_y()), content);
        assert!(session.zoom.is_fit());
        assert!(!session.pannable(content));

        session.scroll_by(5, content);
        assert_eq!(session.first_row, 5);
    }

    #[test]
    fn the_panel_is_centred_and_inside_the_window() {
        let panel = panel_rect(1100.0, 700.0);
        assert!(panel.left > 0.0 && panel.right < 1100.0, "{panel:?}");
        assert!((panel.center_x() - 550.0).abs() < 0.5, "{panel:?}");
        assert!((panel.center_y() - 350.0).abs() < 0.5, "{panel:?}");
    }

    #[test]
    fn a_small_window_still_gets_a_panel_with_area() {
        let panel = panel_rect(320.0, 240.0);
        assert!(panel.width() > 0.0 && panel.height() > 0.0, "{panel:?}");
    }

    /// The entrance starts at the row and ends at the resting rect — the whole
    /// point of carrying the anchor.
    #[test]
    fn the_panel_grows_out_of_the_row() {
        let resting = panel_rect(1100.0, 700.0);
        let row = Rect::from_xywh(240.0, 180.0, 260.0, 24.0);

        let start = entrance_at(row, resting, 0.0);
        assert!(start.width() < resting.width() / 2.0, "{start:?}");
        // Near the row it came from, not the middle of the window.
        assert!((start.center_x() - row.center_x()).abs() < 1.0, "{start:?}");

        let end = entrance_at(row, resting, 1.0);
        assert!((end.width() - resting.width()).abs() < 1.0, "{end:?}");
    }

    /// A row scrolled out of view has no anchor; the panel swells in place
    /// rather than growing out of nowhere.
    #[test]
    fn no_anchor_opens_in_place() {
        let resting = panel_rect(1100.0, 700.0);
        let start = entrance_at(Rect::new_empty(), resting, 0.0);
        assert!(
            (start.center_x() - resting.center_x()).abs() < 1.0,
            "{start:?}"
        );
        assert!(start.width() < resting.width(), "{start:?}");
        assert!(start.width() > resting.width() * 0.9, "{start:?}");
    }
}
