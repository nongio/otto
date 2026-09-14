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
//! actually feeds it changes.
//!
//! The window's own background is a style too, but one the *compositor* holds
//! — see `FilesApp::on_app_ready`. It has to be, because a client cannot blur
//! what is behind itself.
//!
//! What is still immediate-mode, drawn over this scene by [`crate::view::draw`]:
//! the sidebar's places, the header's title and buttons, and the list and grid
//! views. Those are bounded and cheap. The Miller stack is neither, and is not
//! drawn in the window at all — see [`crate::pane_surfaces`].

use layers::prelude::*;
use layers::types::{Color as LayerColor, Point as LayerPoint, Size as LayerSize};
use otto_kit::components::scroll::RowLayout;
use otto_kit::icons;
use otto_kit::prelude::*;
use otto_kit::theme::Theme;
use otto_kit::typography::styles;
use skia_safe::{Canvas, Color, Image, Paint, Point, Rect};

use crate::view::{self, Frame, PaneData, RunEnds, ViewMode};

/// Colours that reach the engine as a style rather than a paint.
fn paint_color(color: Color) -> PaintColor {
    PaintColor::Solid {
        color: LayerColor::new_rgba255(color.r(), color.g(), color.b(), color.a()),
    }
}

/// The scene's structural panels.
pub struct Scene {
    engine: std::sync::Arc<Engine>,
    /// The surface's own root node, handed over by otto-kit. Everything here
    /// hangs off it, so it is drawn into this window's canvas and no other.
    root: Layer,
    sidebar: Layer,
    header: Layer,
    /// The picker's action row. Hidden in the browser, which has none.
    footer: Layer,
    /// The breadcrumb strip along the bottom. Hidden in the picker, whose
    /// bottom edge belongs to the action row.
    path_bar: Layer,
    /// The paper the file area sits on.
    content: Layer,

    /// What the panels were last laid out against, so a frame that changed
    /// nothing geometric does not touch the engine at all.
    layout: Option<LayoutKey>,
    /// Colour scheme and blur the panel materials were last built for. The
    /// blur is in the key because it decides whether they are translucent —
    /// see [`Scene::sync_materials`].
    materials: Option<(bool, bool)>,
    /// Turns the compositor's backdrop blur on and off. The scene owns that
    /// timing because it owns the fade the toggle has to hide under — see
    /// [`Scene::sync_materials`]. `None` before the window has handed it over,
    /// and where there is no blur to switch at all.
    frost: std::sync::Arc<FrostState>,
    /// When the engine was last ticked, so a fade advances by real time.
    last_tick: std::time::Instant,
}

/// What the scene needs from its host while the panel materials fade.
///
/// Shared rather than called back into, because both of these belong to the
/// `Window` — and a `Window` is not `Send`, so it cannot ride the engine's own
/// animation callbacks, which are.
#[derive(Default)]
pub struct FrostState {
    /// The compositor's backdrop blur, to be switched once it is safe:
    /// entering the frost that is immediately, while the panels are still
    /// filled in; leaving it, not until they have finished filling in again.
    /// Taken by the host, which owns the window.
    pending: std::sync::Mutex<Option<bool>>,
    /// Whether a fade is still running. The engine only advances when it is
    /// driven, so a fade that stopped being drawn would stop halfway: the host
    /// keeps ticking and repainting while this is set.
    fading: std::sync::atomic::AtomicBool,
}

impl FrostState {
    /// The blur switch the scene is waiting on, if any. Cleared by the read —
    /// it is an instruction, not a state.
    pub fn take_pending(&self) -> Option<bool> {
        self.pending.lock().ok().and_then(|mut p| p.take())
    }

    pub fn is_fading(&self) -> bool {
        self.fading.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn request(&self, frosted: bool) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(frosted);
        }
    }
}

/// How long the panel materials take to fade between translucent and filled
/// in. Matches otto-kit's own `MATERIAL_FADE`: a window that composites its
/// materials and one that hands them to the window should not fade at
/// different speeds.
const MATERIAL_FADE: f32 = 0.3;

#[derive(PartialEq)]
struct LayoutKey {
    width: u32,
    height: u32,
    mode: ViewMode,
    footer: u32,
    band: u32,
    path_bar: u32,
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
        let path_bar = new_layer("files-path-bar");
        let content = new_layer("files-content");

        // Header last: it overlaps nothing, but it is the panel drawn over the
        // top of the content area's ground and the order records that.
        let _ = root.add_sublayer(&sidebar);
        let _ = root.add_sublayer(&content);
        let _ = root.add_sublayer(&header);
        let _ = root.add_sublayer(&footer);
        let _ = root.add_sublayer(&path_bar);

        Self {
            engine,
            root,
            sidebar,
            header,
            footer,
            path_bar,
            content,
            layout: None,
            materials: None,
            frost: Default::default(),
            last_tick: std::time::Instant::now(),
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
        self.sync_materials(f);
        self.sync_layout(f);
        // One tick, so the changes above are folded into the scene before the
        // host renders it. The delta is zero unless the panel materials are
        // mid-fade — that is the only thing here that animates, and everything
        // else would rather not have time pass under it.
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;
        self.engine
            .update(if self.frost.is_fading() { elapsed } else { 0.0 });
    }

    /// Panel materials. These are the backgrounds the panes "should not draw":
    /// they are set once per colour-scheme change — or when the window's blur
    /// comes and goes with the focus — and composited by the engine, not
    /// painted per frame.
    fn sync_materials(&mut self, f: &Frame) {
        let dark = view::is_dark();
        // Translucent only while there is a blur to be translucent over. See
        // [`view::opaque`]. Where the compositor has no blur to offer at all,
        // the panels take a solid shade of their own rather than the white of
        // the content they frame.
        let fill = |color: skia_safe::Color| {
            paint_color(if f.blurred {
                color
            } else if otto_kit::backdrop::blur_available() {
                view::opaque(color)
            } else {
                view::solid_panel_material()
            })
        };
        let previous = self.materials;
        if previous == Some((dark, f.blurred)) {
            return;
        }
        self.materials = Some((dark, f.blurred));

        // The blur only ever changes under a cover. Entering the frost the
        // window has already turned it on — a configure lands before this
        // frame, while the panels are still filled in. Leaving it, it stays on
        // until they have finished filling in again, which is what the
        // transaction below waits for. So what the eye follows is the material
        // thinning or thickening, never the frost arriving or leaving.
        //
        // Only a change of *blur* fades. A change of colour scheme is a
        // different set of colours rather than the same one at a different
        // opacity, and crossfading it would run every panel through a wrong
        // intermediate.
        let fades = previous
            .is_some_and(|(was_dark, was_blurred)| was_dark == dark && was_blurred != f.blurred);
        let transition = fades.then(|| Transition::ease_out_quad(MATERIAL_FADE));

        // The sidebar takes the same material as everything else on this
        // window's chrome — see [`view::panel_material`] — rather than the
        // toolkit's sidebar colour, which is a shade apart and showed as a
        // seam down the edge where the two met.
        let sidebar = self
            .sidebar
            .set_background_color(fill(view::panel_material()), transition.clone());
        self.header
            .set_background_color(fill(view::panel_material()), transition.clone());
        // The action row is the same material as the header, and for the same
        // reason: it is chrome laid over the window's blur, not a hole in it.
        // Without a ground it reads as bare blur with buttons floating on it.
        self.footer
            .set_background_color(fill(view::panel_material()), transition.clone());
        // The path bar is chrome laid over the blur too, and the same material
        // as the header keeps the window's top and bottom edges reading as one
        // frame around the listing.
        self.path_bar
            .set_background_color(fill(view::panel_material()), transition.clone());
        // The content ground is opaque either way — there is no blur behind
        // the file area to be translucent over — so it never fades.
        self.content
            .set_background_color(paint_color(view::content_ground()), None);

        if !fades {
            // Nothing to wait for: the panels are already where they belong.
            if !f.blurred {
                self.frost.request(false);
            }
            return;
        }

        // Any of the panels' transactions would do as the clock; the sidebar is
        // the widest of them and the one the frost is most visible through.
        self.frost
            .fading
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let frost = self.frost.clone();
        let leaving = !f.blurred;
        sidebar.on_finish(
            move |_: &Layer, _| {
                frost
                    .fading
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                // Leaving the frost: only now, with the panels opaque again,
                // is the blur safe to drop.
                if leaving {
                    frost.request(false);
                }
            },
            true,
        );
    }

    /// The blur switch the scene is waiting on, and whether a fade is still
    /// running.
    ///
    /// The scene fades the panel materials, so it is the only thing that knows
    /// when they are opaque enough to hide the switch — but the switch belongs
    /// to the window. See [`FrostState`].
    pub fn frost_state(&self) -> std::sync::Arc<FrostState> {
        self.frost.clone()
    }

    fn sync_layout(&mut self, f: &Frame) {
        let key = LayoutKey {
            width: f.width.to_bits(),
            height: f.height.to_bits(),
            mode: f.mode,
            footer: f.footer.to_bits(),
            // The filter strip grows the header panel and shortens the
            // content one, so opening it has to relayout.
            band: view::search_band_h().to_bits(),
            path_bar: f.path_bar_h.to_bits(),
        };
        if self.layout.as_ref() == Some(&key) {
            return;
        }
        self.layout = Some(key);

        let sidebar_w = view::sidebar_w();
        let header_h = view::header_h();

        // The full *window* height, not the file area's: the sidebar is one
        // column of material running from the titlebar to the bottom edge, and
        // stopping it at the content's bottom leaves the desktop showing
        // through beside the path bar and the picker's action row.
        place(&self.sidebar, 0.0, 0.0, sidebar_w, f.window_h());
        place(
            &self.header,
            sidebar_w,
            0.0,
            (f.width - sidebar_w).max(0.0),
            header_h,
        );
        // The path bar is drawn *inside* this panel rather than on one of
        // its own: it is the bottom edge of the file area, sharing the paper
        // the rows sit on, and a panel of its own would put a seam between
        // them that says they are different surfaces. Always its full height —
        // the bar's content comes and goes with the selection, the band does
        // not, or every row would move whenever it did.
        let path_bar_h = view::PATH_BAR_H;
        place(
            &self.content,
            sidebar_w,
            header_h,
            (f.width - sidebar_w).max(0.0),
            (f.height + path_bar_h - header_h).max(0.0),
        );
        // Anchored to the window's bottom edge, which is below the path bar
        // as well as below the content: `f.height` is the file area's bottom
        // and no longer the last thing above the row. Beside the sidebar
        // rather than over it, the way the header is — the sidebar is one
        // column of material running the full height of the window.
        if f.footer > 0.0 {
            self.footer.set_hidden(false);
            place(
                &self.footer,
                sidebar_w,
                f.height + f.path_bar_h,
                (f.width - sidebar_w).max(0.0),
                f.footer,
            );
        } else {
            self.footer.set_hidden(true);
        }

        // Between the file area and the action row: `f.height` is the file
        // area's bottom, and the picker's row is anchored to the window's.
        if f.path_bar_h > 0.0 {
            self.path_bar.set_hidden(false);
            place(
                &self.path_bar,
                sidebar_w,
                f.height,
                (f.width - sidebar_w).max(0.0),
                f.path_bar_h,
            );
        } else {
            self.path_bar.set_hidden(true);
        }
    }
}

/// Paint the rows of one Miller column that fall inside `band`, a slice of the
/// column's content in content coordinates — `0` is the top of the column's
/// content, before any scroll.
///
/// For a column's own surfaces, where the compositor moves the painted band to
/// scroll it: nothing here knows the scroll offset, so a glide never repaints.
///
/// Rows only, on a transparent ground: the file area's paper is the window's,
/// underneath, and the column's tint and status line are its own surfaces —
/// see [`crate::pane_surfaces`].
pub(crate) fn paint_column_band(canvas: &Canvas, f: &Frame, depth: usize, width: f32, band: Rect) {
    let pane = &f.panes[depth];
    if pane.error.is_some() || pane.loading || pane.entries.is_empty() {
        return;
    }
    let layout =
        RowLayout::new(view::ROW_H, pane.entries.len()).with_insets(view::MILLER_ROW_INSET, 0.0);
    let visible = layout.visible(band);
    if visible.is_empty() {
        return;
    }
    let rows = build_rows(pane, (visible.start, visible.end), f, depth);
    canvas.save();
    canvas.translate((0.0, layout.rect(visible.start, width).top));
    for row in &rows {
        row.draw(canvas, f.theme, width);
    }
    canvas.restore();
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
// Placement
// ---------------------------------------------------------------------------

fn place(layer: &Layer, x: f32, y: f32, width: f32, height: f32) {
    layer.set_position(LayerPoint::new(x, y), None);
    layer.set_size(LayerSize::points(width, height), None);
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
            origin: None,
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

        let owned = [entry];
        let entries: Vec<&Entry> = owned.iter().collect();
        let pane = PaneData {
            entries,
            selection: None,
            cursor: None,
            scroll: 0.0,
            bar: None,
            velocity: 0.0,
            loading: false,
            error: None,
        };
        let theme = Theme::light();
        let frame = Frame {
            search: None,
            index_available: true,
            search_focused: false,
            search_placeholder: "",
            search_scope: crate::model::SearchScope::Folder,
            grid_sections: view::GridSections::FLAT,
            show_folders: false,
            mode_locked: false,
            trash: None,
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
            focused: true,
            blurred: true,
            can_go_back: false,
            can_go_forward: false,
            nav_pressed: None,
            preview: None,
            action_row: None,
            footer: 0.0,
            quickview_close_hovered: false,
            quickview_expand_hovered: false,
            quickview_expanded: false,
            drop_target: None,
            marquee: None,
            path_bar: Vec::new(),
            path_bar_note: None,
            path_bar_h: view::PATH_BAR_H,
            path_crumb_hover: None,
            path_entry: false,
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
