//! otto-files' panels as a `lay-rs` scene.
//!
//! The browser used to be one canvas: every frame replayed the sidebar, the
//! header, every visible row of every Miller column and the preview, because
//! immediate mode has no way to say "this part did not change". Hovering a
//! traffic light re-recorded the file listing; a one-pixel scroll re-recorded
//! the sidebar. It also meant [`Entry::icon_chain`] — a MIME lookup and a
//! `Vec<String>` — ran per row per frame.
//!
//! Here the panels are *layers* instead. Their backgrounds are a style the
//! engine composites (`set_background_color`), not a rect this client paints,
//! and their content is a picture the engine caches until something that
//! actually feeds it changes. What "actually feeds it" means is [`PaneKey`]:
//! rebuild the closure when the key moves, replay the cached picture when it
//! does not.
//!
//! The window's own background is a style too, but one the *compositor* holds
//! — see `FilesApp::on_app_ready`. It has to be, because a client cannot blur
//! what is behind itself.
//!
//! What is still immediate-mode, drawn over this scene by [`crate::view::draw`]:
//! the sidebar's places, the header's title and buttons, and the list and grid
//! views. Those are bounded and cheap; the Miller stack was neither.

use layers::prelude::*;
use layers::types::{Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::icons;
use otto_kit::prelude::*;
use otto_kit::theme::Theme;
use otto_kit::typography::styles;
use skia_safe::{Canvas, Color, Image, Paint, Point, Rect};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::model::Entry;
use crate::view::{self, Frame, PaneData, RunEnds, ViewMode};

/// Colours that reach the engine as a style rather than a paint.
fn paint_color(color: Color) -> PaintColor {
    PaintColor::Solid {
        color: LayerColor::new_rgba255(color.r(), color.g(), color.b(), color.a()),
    }
}

/// The scene's structural panels, plus one layer per Miller column.
pub struct Scene {
    engine: std::sync::Arc<Engine>,
    /// The surface's own root node, handed over by otto-kit. Everything here
    /// hangs off it, so it is drawn into this window's canvas and no other.
    root: Layer,
    sidebar: Layer,
    header: Layer,
    /// The picker's action row. Hidden in the browser, which has none.
    footer: Layer,
    /// The paper the file area sits on. Clips, so a column panned past the
    /// window edge is cut off by the engine rather than by a `clip_rect` this
    /// client has to remember to balance.
    content: Layer,
    /// Pooled: a column that goes away is hidden, not destroyed, because the
    /// next navigation almost always wants it straight back.
    panes: Vec<PaneLayer>,
    preview: Layer,
    preview_key: Option<u64>,

    /// What the panels were last laid out against, so a frame that changed
    /// nothing geometric does not touch the engine at all.
    layout: Option<LayoutKey>,
    dark: Option<bool>,
}

/// One Miller column: the pane itself, and the strip of rows inside it that
/// the scroll offset moves.
struct PaneLayer {
    pane: Layer,
    /// Separated from `pane` so that scrolling is a *position* change. Within
    /// one row's pitch that is all a scroll is, and the engine replays the
    /// cached picture at the new offset without this client recording a thing.
    rows: Layer,
    key: Option<PaneKey>,
}

#[derive(PartialEq)]
struct LayoutKey {
    width: u32,
    height: u32,
    mode: ViewMode,
    pan: i32,
    miller_w: u32,
    panes: usize,
    preview: bool,
    footer: u32,
}

/// Everything one column's rows are drawn from, reduced to something cheap to
/// compare. The entries themselves are hashed rather than cloned: a listing
/// that has not changed hashes the same, and hashing thirty visible rows costs
/// far less than the re-record it avoids.
#[derive(PartialEq)]
struct PaneKey {
    /// Which rows are on screen. Scrolling past a row boundary changes this
    /// and costs one column a re-record; scrolling within a row does not.
    range: (usize, usize),
    entries: u64,
    /// The thumbnail store's epoch, which moves when a picture lands. See
    /// where this is filled in for why it belongs in the key.
    thumbs: u64,
    /// Selection, cursor, cut marks and an in-place rename, over the visible
    /// rows only.
    marks: u64,
    active: bool,
    status: Status,
    size: (u32, u32),
    dark: bool,
}

#[derive(PartialEq, Clone)]
enum Status {
    Rows,
    Loading,
    Empty,
    Error(String),
}

impl Scene {
    /// `root` is the window surface's layer node — see
    /// `BaseWaylandSurface::new`, which builds one per surface and positions
    /// it absolutely so each surface's scene draws at its own origin.
    pub fn new(root: Layer) -> Self {
        let engine = root.engine.clone();

        let new_layer = |key: &str| {
            let layer = engine.new_layer();
            layer.set_key(key);
            layer.set_layout_style(taffy::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            layer
        };

        let sidebar = new_layer("files-sidebar");
        let header = new_layer("files-header");
        let footer = new_layer("files-footer");
        let content = new_layer("files-content");
        let preview = new_layer("files-preview");

        // Header last: it overlaps nothing, but it is the panel drawn over the
        // top of the content area's ground and the order records that.
        let _ = root.add_sublayer(&sidebar);
        let _ = root.add_sublayer(&content);
        let _ = root.add_sublayer(&header);
        let _ = root.add_sublayer(&footer);
        let _ = content.add_sublayer(&preview);

        // `clip_children`, not just `clip_content`: what has to be cut off at
        // the content area's edge is the *columns*, which are children of it.
        // A column panned half past the sidebar is then clipped by the engine
        // rather than by a `clip_rect` this client has to balance by hand.
        content.set_clip_content(true, None);
        content.set_clip_children(true, None);
        preview.set_picture_cached(true);

        Self {
            engine,
            root,
            sidebar,
            header,
            footer,
            content,
            panes: Vec::new(),
            preview,
            preview_key: None,
            layout: None,
            dark: None,
        }
    }

    /// Draw the scene into the window's canvas.
    ///
    /// The same thing `BaseWaylandSurface::render_layer_node` does, done here
    /// so the caller does not have to reach back through the surface for a
    /// node this already holds.
    pub fn render(&self, canvas: &Canvas) {
        draw_scene(canvas, self.engine.scene(), self.root.id());
    }

    /// Bring the scene up to date with `f`, ahead of the frame being drawn.
    ///
    /// Every step here is gated on its own key, so the common frame — a hover
    /// somewhere, a repaint after a frame callback — reaches the engine with
    /// no changes at all and the whole window is replayed from cached
    /// pictures.
    pub fn update(&mut self, f: &Frame) {
        self.sync_materials(f.theme);
        self.sync_layout(f);
        self.sync_panes(f);
        // One tick, so the changes above are folded into the scene before the
        // host renders it. Nothing here animates, so the delta is zero.
        self.engine.update(0.0);
    }

    /// Panel materials. These are the backgrounds the panes "should not draw":
    /// they are set once per colour-scheme change and composited by the
    /// engine, not painted per frame.
    fn sync_materials(&mut self, theme: &Theme) {
        let dark = view::is_dark();
        if self.dark == Some(dark) {
            return;
        }
        self.dark = Some(dark);

        self.sidebar
            .set_background_color(paint_color(theme.material_sidebar), None);
        self.header
            .set_background_color(paint_color(view::header_material()), None);
        // The action row is the same material as the header, and for the same
        // reason: it is chrome laid over the window's blur, not a hole in it.
        // Without a ground it reads as bare blur with buttons floating on it.
        self.footer
            .set_background_color(paint_color(view::header_material()), None);
        self.content
            .set_background_color(paint_color(view::content_ground()), None);
        self.preview
            .set_background_color(paint_color(view::content_ground()), None);
    }

    fn sync_layout(&mut self, f: &Frame) {
        let key = LayoutKey {
            width: f.width.to_bits(),
            height: f.height.to_bits(),
            mode: f.mode,
            pan: f.pan.to_bits() as i32,
            miller_w: f.miller_w.to_bits(),
            panes: f.panes.len(),
            preview: f.preview.is_some(),
            footer: f.footer.to_bits(),
        };
        if self.layout.as_ref() == Some(&key) {
            return;
        }
        self.layout = Some(key);

        let sidebar_w = view::SIDEBAR_W;
        let header_h = view::HEADER_H;

        // The full *window* height, not the file area's: the sidebar is one
        // column of material running from the titlebar to the bottom edge,
        // and stopping it at the content's bottom leaves the picker's
        // bottom-left corner unpainted beside the action row.
        place(&self.sidebar, 0.0, 0.0, sidebar_w, f.height + f.footer);
        place(
            &self.header,
            sidebar_w,
            0.0,
            (f.width - sidebar_w).max(0.0),
            header_h,
        );
        place(
            &self.content,
            sidebar_w,
            header_h,
            (f.width - sidebar_w).max(0.0),
            (f.height - header_h).max(0.0),
        );
        // `f.height` is the file area's bottom, so the row starts exactly
        // where the content ends. Beside the sidebar rather than over it, the
        // way the header is — the sidebar is one column of material running
        // the full height of the window.
        if f.footer > 0.0 {
            self.footer.set_hidden(false);
            place(
                &self.footer,
                sidebar_w,
                f.height,
                (f.width - sidebar_w).max(0.0),
                f.footer,
            );
        } else {
            self.footer.set_hidden(true);
        }
    }

    /// Create, place and hide column layers to match the current stack.
    ///
    /// Only Miller view has columns; the list and grid draw into the content
    /// area directly, so their panes are simply all hidden.
    fn sync_panes(&mut self, f: &Frame) {
        // With the columns in their own subsurfaces they are painted there and
        // composited over this scene; drawing them here too would double them.
        let miller = f.mode == ViewMode::Columns && !crate::pane_surfaces::enabled();
        let wanted = if miller { f.panes.len() } else { 0 };

        while self.panes.len() < wanted {
            let pane = self.engine.new_layer();
            pane.set_key("files-pane");
            pane.set_layout_style(taffy::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            // Likewise the rows strip, which is a child the scroll offset
            // moves: without this a scrolled column would paint over the one
            // below the header.
            pane.set_clip_content(true, None);
            pane.set_clip_children(true, None);

            let rows = self.engine.new_layer();
            rows.set_key("files-pane-rows");
            rows.set_layout_style(taffy::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            rows.set_picture_cached(true);
            // The strip is the one layer here that is stable in content and
            // moves constantly, which is exactly what an offscreen cache is
            // for: a frame of scrolling becomes a blit of the recorded band
            // instead of a replay of every row's display list.
            //
            // **Off by default, because lay-rs rasterises it at the wrong
            // resolution.** `create_surface_for_node` sizes the offscreen
            // from `surface_size_for_render_layer`, which is the layer's
            // bounds in *points*, with no device scale applied — and the
            // image is then drawn back onto a render canvas that already
            // carries the HiDPI transform. On a 2x display every cached row
            // is therefore a 2x upscale, and the column text is visibly
            // soft next to the sidebar and the list view, which draw
            // straight to the canvas. It is invisible at 1x, which is why it
            // survived.
            //
            // `picture_cached` above is unaffected — a display list is
            // replayed at the canvas's own resolution, not resampled — so
            // what is lost here is blit-instead-of-replay, not the far
            // larger win of not re-recording the rows at all.
            //
            // `OTTO_FILES_ROWCACHE=1` turns it back on, for testing a lay-rs
            // that has been fixed.
            rows.set_image_cached(std::env::var_os("OTTO_FILES_ROWCACHE").is_some());

            let _ = pane.add_sublayer(&rows);
            // Before the preview, which is the trailing member of the stack.
            let _ = self.content.add_sublayer(&pane);
            self.panes.push(PaneLayer {
                pane,
                rows,
                key: None,
            });
        }

        for (depth, slot) in self.panes.iter_mut().enumerate() {
            if depth >= wanted {
                slot.pane.set_hidden(true);
                continue;
            }
            slot.pane.set_hidden(false);
            slot.sync(depth, f);
        }

        self.sync_preview(f);
    }

    fn sync_preview(&mut self, f: &Frame) {
        let Some(data) = f.preview.as_ref() else {
            self.preview.set_hidden(true);
            self.preview_key = None;
            return;
        };
        if f.mode != ViewMode::Columns {
            self.preview.set_hidden(true);
            return;
        }
        self.preview.set_hidden(false);

        // The preview's rect comes from the same stack geometry the columns
        // use, translated into the content area's own coordinates.
        let full = view::preview_pane_rect(f.panes.len(), f.height, f.pan, f.miller_w);
        place_in_content(&self.preview, full, f.height);

        let key = hash_of(&(
            data.name,
            data.first_row,
            data.decoded.is_some(),
            view::is_dark(),
            full.width().to_bits(),
            full.height().to_bits(),
        ));
        if self.preview_key == Some(key) {
            return;
        }
        self.preview_key = Some(key);

        let content = view::preview_content(data, f.theme.clone());
        self.preview
            .set_draw_content(move |canvas: &Canvas, w: f32, h: f32| {
                content(canvas, w, h);
                Rect::from_wh(w, h)
            });
    }
}

impl PaneLayer {
    fn sync(&mut self, depth: usize, f: &Frame) {
        let pane = &f.panes[depth];
        let full = view::miller_pane_rect(depth, f.height, f.pan, f.miller_w);
        place_in_content(&self.pane, full, f.height);

        // The active column is a shade off the ground — as a style, so which
        // column has the keyboard costs a colour change and not a repaint.
        let active = depth == f.active;
        self.pane.set_background_color(
            paint_color(if active {
                f.theme.fill_quaternary
            } else {
                Color::TRANSPARENT
            }),
            None,
        );

        let status = if let Some(error) = pane.error {
            Status::Error(error.to_string())
        } else if pane.loading {
            Status::Loading
        } else if pane.entries.is_empty() {
            Status::Empty
        } else {
            Status::Rows
        };

        // Record a few rows past each edge of the viewport. The strip's own
        // key is what triggers a re-record, and the key holds `range` — so
        // without any margin the band is left, and re-recorded, on every row
        // boundary the scroll crosses. A margin turns that into once every
        // `OVERSCAN_ROWS`, at the cost of carrying that many extra rows in
        // the cached image.
        const OVERSCAN_ROWS: usize = 8;
        let visible = view::miller_visible_range(f, depth);
        let range = (
            visible.0.saturating_sub(OVERSCAN_ROWS),
            (visible.1 + OVERSCAN_ROWS).min(pane.entries.len()),
        );
        let inset = view::MILLER_ROW_INSET;

        // The strip covers the *visible band*, not the whole listing.
        //
        // Sizing it to the listing is the obvious thing and it is wrong: a
        // directory of five hundred entries makes a twelve-thousand-pixel
        // layer, and moving that layer once per scroll frame damages its whole
        // bounds however little of it the pane shows. The picture stays cheap
        // — Skia replays only what was recorded — but the compositor is handed
        // a full-screen damage rect every frame, which is what spins the fans.
        //
        // So the strip holds only the rows in `range` and is positioned so
        // that row `range.0` lands where it belongs. Scrolling within a row's
        // pitch moves the strip and nothing more; crossing a row boundary
        // changes `range`, which moves the key below and re-records the band.
        //
        // A column with no rows to show — loading, empty, or failed — has no
        // band at all, so the strip takes the pane's own box instead and its
        // one line of text is centred in that.
        let rows_shown = status == Status::Rows;
        let (strip_h, strip_y) = if rows_shown {
            (
                range.1.saturating_sub(range.0) as f32 * view::ROW_H,
                inset + range.0 as f32 * view::ROW_H - pane.scroll,
            )
        } else {
            (full.height(), 0.0)
        };
        self.rows
            .set_size(LayerSize::points(full.width(), strip_h), None);
        self.rows.set_position(LayerPoint::new(0.0, strip_y), None);

        let key = PaneKey {
            range,
            entries: hash_entries(&pane.entries, range),
            // A thumbnail landing changes what this pane draws without
            // changing anything else the key is made of, so the store's epoch
            // rides along: it moves exactly when a picture arrives, and a pane
            // that would show it rebuilds while the rest replay.
            thumbs: f.thumbs.map(|store| store.epoch()).unwrap_or(0),
            marks: hash_marks(pane, range, f, depth),
            active,
            status: status.clone(),
            size: (full.width().to_bits(), strip_h.to_bits()),
            dark: view::is_dark(),
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);

        let theme = f.theme.clone();
        match status {
            Status::Rows => {
                let rows = build_rows(pane, range, f, depth);
                self.rows
                    .set_draw_content(move |canvas: &Canvas, w: f32, h: f32| {
                        for row in &rows {
                            row.draw(canvas, &theme, w);
                        }
                        Rect::from_wh(w, h)
                    });
            }
            Status::Loading | Status::Empty | Status::Error(_) => {
                let (text, color) = match &status {
                    Status::Error(error) => (error.clone(), theme.text_secondary),
                    Status::Loading => ("Loading…".to_string(), theme.text_tertiary),
                    _ => ("Empty".to_string(), theme.text_tertiary),
                };
                // The strip is the pane's own box in this state, so centring on
                // it is centring on the column.
                self.rows
                    .set_draw_content(move |canvas: &Canvas, w: f32, h: f32| {
                        Label::new(&text)
                            .with_style(styles::BODY)
                            .with_color(color)
                            .centered_at(w / 2.0, h / 2.0)
                            .render(canvas);
                        Rect::from_wh(w, h)
                    });
            }
        }
    }
}

/// Paint one Miller column's rows into a canvas whose origin is the column's
/// own top-left corner, with the scroll offset already applied.
///
/// The same drawing [`PaneLayer::sync`] records into the rows strip, but aimed
/// at whatever canvas the caller has — used by [`crate::pane_surfaces`] to
/// paint a column into its own subsurface instead of into the window.
pub(crate) fn paint_column(canvas: &Canvas, f: &Frame, depth: usize, width: f32) {
    let pane = &f.panes[depth];
    canvas.clear(paint_to_color(view::content_ground()));
    if pane.error.is_some() || pane.loading || pane.entries.is_empty() {
        return;
    }
    let range = view::miller_visible_range(f, depth);
    let rows = build_rows(pane, range, f, depth);
    canvas.save();
    canvas.translate((
        0.0,
        view::MILLER_ROW_INSET + range.0 as f32 * view::ROW_H - pane.scroll,
    ));
    for row in &rows {
        row.draw(canvas, f.theme, width);
    }
    canvas.restore();
}

fn paint_to_color(color: Color) -> Color {
    color
}

/// One row, with everything it draws already resolved — the icon decoded, the
/// name measured and ellipsized, the colours chosen. This is the work that
/// used to happen per row per frame.
struct Row {
    /// Top edge in the strip's own coordinates.
    top: f32,
    name: String,
    icon: Option<Image>,
    /// The file's own picture, where one is ready. Drawn instead of `icon`.
    ///
    /// Owned rather than borrowed because the strip's draw closure outlives
    /// the frame that built it: the picture has to travel into the closure,
    /// the way the icon already does. A Skia image is a handle over shared
    /// pixels, so this is a refcount bump and not a copy of the bitmap.
    thumb: Option<Image>,
    is_dir: bool,
    selected: bool,
    ends: RunEnds,
    cursor: bool,
    cut: bool,
    /// Suppressed while the host's text field is over this row.
    renaming: bool,
    text_color: Color,
    detail_color: Color,
    selection_color: Color,
}

impl Row {
    fn draw(&self, canvas: &Canvas, theme: &Theme, width: f32) {
        let rect = Rect::from_ltrb(0.0, self.top, width, self.top + view::ROW_H);

        if self.selected {
            view::draw_selection_run(canvas, rect, self.selection_color, 6.0, self.ends);
        } else if self.cursor {
            view::draw_cursor_ring(canvas, theme, rect, 6.0);
        }

        let icon_box = Rect::from_xywh(
            14.0,
            rect.center_y() - view::ICON_SIZE / 2.0,
            view::ICON_SIZE,
            view::ICON_SIZE,
        );
        if let Some(image) = &self.thumb {
            // The same painter the list and grid use, so a file looks the same
            // in all three views rather than only in the two that draw
            // themselves immediately.
            view::draw_thumbnail(canvas, image, icon_box, self.cut);
        } else if let Some(image) = &self.icon {
            let mut paint = Paint::default();
            if self.cut {
                paint.set_alpha(110);
            }
            canvas.draw_image_rect(image, None, icon_box, &paint);
        }

        if !self.renaming {
            Label::new(&self.name)
                .with_style(styles::BODY_MEDIUM)
                .with_color(self.text_color)
                .centered_on(14.0 + view::ICON_SIZE + 8.0, rect.center_y())
                .render(canvas);
        }

        if self.is_dir {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(self.detail_color);
            paint.set_stroke_width(1.4);
            paint.set_style(skia_safe::paint::Style::Stroke);
            paint.set_stroke_cap(skia_safe::paint::Cap::Round);
            let x = width - 18.0;
            let cy = rect.center_y();
            let mut builder = skia_safe::PathBuilder::new();
            builder.move_to(Point::new(x, cy - 3.5));
            builder.line_to(Point::new(x + 3.5, cy));
            builder.line_to(Point::new(x, cy + 3.5));
            canvas.draw_path(&builder.detach(), &paint);
        }
    }
}

fn build_rows(pane: &PaneData<'_>, range: (usize, usize), f: &Frame, depth: usize) -> Vec<Row> {
    let width = f.miller_w;
    let font = styles::BODY_MEDIUM.font();
    let active = depth == f.active;

    (range.0..range.1)
        .map(|index| {
            let entry = pane.entries[index];
            let selected = pane.is_selected(index);
            let cut = f.cut.contains(&entry.path);
            let highlighted = selected && active;
            let (text_color, detail_color) = view::row_colors(f.theme, highlighted);

            // A thumbnail, where the store has one; the icon is resolved
            // anyway, because it is what this row falls back to and it is a
            // cache hit either way.
            let thumb = f
                .thumbs
                .and_then(|store| store.image(&entry.path, entry.modified))
                .cloned();
            let chain = entry.icon_chain();
            let refs: Vec<&str> = chain.iter().map(String::as_str).collect();
            let icon =
                icons::cached_icon_chain_at(&refs, view::ICON_SIZE as i32, icons::FULL_COLOUR_SIZE);

            let name_x = 14.0 + view::ICON_SIZE + 8.0;
            let trailing = if entry.is_dir { 24.0 } else { 8.0 };
            let name = view::ellipsize(&font, &entry.name, width - trailing - name_x);

            Row {
                // Relative to the band, which is what the strip layer holds:
                // the strip's own position carries `range.0` and the scroll.
                top: (index - range.0) as f32 * view::ROW_H,
                name,
                icon,
                thumb,
                is_dir: entry.is_dir,
                selected,
                ends: RunEnds::of_pane(pane, index),
                cursor: active && pane.cursor == Some(index) && !selected,
                cut,
                renaming: f.renaming == Some((depth, index)),
                text_color: if cut {
                    view::dim_color(text_color)
                } else {
                    text_color
                },
                detail_color,
                selection_color: if active {
                    f.theme.material_selection_focused
                } else {
                    f.theme.fill_quaternary
                },
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Identity of the visible rows. Names and kinds are what is drawn, so they
/// are what has to be watched; a listing reordered or refreshed under the same
/// names draws the same picture.
fn hash_entries(entries: &[&Entry], range: (usize, usize)) -> u64 {
    let mut hasher = DefaultHasher::new();
    for entry in &entries[range.0.min(entries.len())..range.1.min(entries.len())] {
        entry.name.hash(&mut hasher);
        entry.is_dir.hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_marks(pane: &PaneData<'_>, range: (usize, usize), f: &Frame, depth: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    // One past the range at each end: a selection run's rounding depends on
    // whether its neighbour is selected, so a row just off screen changes what
    // the first visible one draws.
    let lo = range.0.saturating_sub(1);
    let hi = (range.1 + 1).min(pane.entries.len());
    for index in lo..hi {
        pane.is_selected(index).hash(&mut hasher);
        f.cut.contains(&pane.entries[index].path).hash(&mut hasher);
    }
    pane.cursor.hash(&mut hasher);
    f.renaming.map(|(d, i)| (d == depth, i)).hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

fn place(layer: &Layer, x: f32, y: f32, width: f32, height: f32) {
    layer.set_position(LayerPoint::new(x, y), None);
    layer.set_size(LayerSize::points(width, height), None);
}

/// Place a column, whose geometry [`crate::view`] computes in *window*
/// coordinates, inside the content layer.
fn place_in_content(layer: &Layer, full: Rect, window_h: f32) {
    place(
        layer,
        full.left - view::SIDEBAR_W,
        0.0,
        full.width(),
        (window_h - view::HEADER_H).max(0.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::time::SystemTime;

    use crate::model::Entry;
    use crate::thumbnails::{Found, Store};
    use otto_kit::filetype::Kind;

    fn red_image(w: i32, h: i32) -> Image {
        let info = skia_safe::ImageInfo::new(
            (w, h),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| [255u8, 0, 0, 255]).collect();
        skia_safe::images::raster_from_data(
            &info,
            skia_safe::Data::new_copy(&pixels),
            w as usize * 4,
        )
        .expect("raster image")
    }

    fn photo() -> Entry {
        Entry {
            name: "photo.png".into(),
            path: PathBuf::from("/tmp/photo.png"),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            kind: Kind::Image,
            size: Some(1),
            modified: Some(SystemTime::UNIX_EPOCH),
        }
    }

    /// The Miller columns build their rows here rather than going through
    /// `view::draw_entry_icon`, so a thumbnail reaching the list and the grid
    /// says nothing about whether it reaches the *default* view. It did not,
    /// once: the store was consulted by the two immediate-mode paths and this
    /// third one resolved an icon and drew that. This test is that regression.
    #[test]
    fn a_miller_row_draws_the_thumbnail_over_the_icon() {
        let entry = photo();
        let mut store = Store::new();
        store.wanted(
            [crate::thumbnails::Request {
                path: entry.path.clone(),
                modified: entry.modified,
                may_generate: true,
            }],
            crate::thumbcache::Size::Normal,
        );
        store.finish(
            entry.path.clone(),
            entry.modified,
            Found::Thumbnail(red_image(64, 64)),
        );

        let owned = vec![entry];
        let entries: Vec<&Entry> = owned.iter().collect();
        let pane = PaneData {
            entries,
            selected: vec![false],
            cursor: None,
            scroll: 0.0,
            bar: None,
            loading: false,
            error: None,
        };
        let theme = Theme::light();
        let frame = Frame {
            width: 1100.0,
            height: 700.0,
            theme: &theme,
            title: "Home",
            subtitle: String::new(),
            places: &[],
            selected_place: None,
            mode: ViewMode::Columns,
            panes: vec![pane],
            active: 0,
            pan: 0.0,
            pan_bar: None,
            miller_w: view::MILLER_W,
            sort: crate::model::SortKey::Name,
            ascending: true,
            list_columns: view::ListColumnWidths::default(),
            opening: None,
            renaming: None,
            cut: Vec::new(),
            controls: otto_kit::components::titlebar::WindowControlsState::new(),
            can_go_back: false,
            can_go_forward: false,
            nav_pressed: None,
            preview: None,
            action_row: None,
            footer: 0.0,
            quickview_close_hovered: false,
            drop_target: None,
            thumbs: Some(&store),
        };

        let rows = build_rows(&frame.panes[0], (0, 1), &frame, 0);
        assert!(
            rows[0].thumb.is_some(),
            "a Miller row must carry the thumbnail the store holds"
        );

        // And it must actually be painted: draw the row and look for red where
        // the icon box is.
        let mut surface = skia_safe::surfaces::raster_n32_premul((240, 40)).unwrap();
        surface.canvas().clear(skia_safe::Color::WHITE);
        rows[0].draw(surface.canvas(), &theme, 240.0);

        let info = skia_safe::ImageInfo::new(
            (1, 1),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let mut px = [0u8; 4];
        let x = (14.0 + view::ICON_SIZE / 2.0) as i32;
        let y = (view::ROW_H / 2.0) as i32;
        assert!(surface.image_snapshot().read_pixels(
            &info,
            &mut px,
            4,
            (x, y),
            skia_safe::image::CachingHint::Allow
        ));
        assert!(
            px[0] > 200 && px[1] < 60 && px[2] < 60,
            "expected the thumbnail in the row's icon box, got {px:?}"
        );
    }
}
