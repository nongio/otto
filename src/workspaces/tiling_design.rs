//! Design mode's chrome: the pane grid, the handles between the panes, the
//! per-cell toolbar and the row of starting-point presets.
//!
//! Generalises [`TilingOverlayView`] from the single preview pane the edge
//! snap shows to one pane per cell, with the same recipe — 30 % white fill,
//! 80 % white 2 pt border, 12 pt radius — so entering design mode reads as
//! "the thing you already saw when you dragged a window to an edge, once per
//! cell". Everything here is presentation and hit-testing; the arithmetic is
//! in [`crate::workspaces::tiling::design`] and the edits are in
//! `src/shell/tiling.rs`.
//!
//! [`TilingOverlayView`]: crate::workspaces::TilingOverlayView

use std::sync::{Arc, RwLock};

use layers::{
    engine::Engine,
    prelude::*,
    types::{Point, Size},
};

use crate::workspaces::tiling::{
    design::{BarHandle, CornerHandle},
    layout::Rect,
    tree::NodeId,
    Axis, Preset,
};

/// Pane fill and border, straight from the snap overlay.
const FILL: (f32, f32, f32, f32) = (1.0, 1.0, 1.0, 0.3);
const BORDER: (f32, f32, f32, f32) = (1.0, 1.0, 1.0, 0.8);
const BORDER_WIDTH: f32 = 2.0;
const RADIUS: f32 = 12.0;

/// The toolbar that appears over a hovered pane, in logical pixels.
const BUTTON: f32 = 28.0;
const BUTTON_GAP: f32 = 6.0;
const TOOLBAR_PAD: f32 = 6.0;

/// One preset button on an empty workspace, in logical pixels.
const PRESET_W: f32 = 96.0;
const PRESET_H: f32 = 72.0;
const PRESET_GAP: f32 = 16.0;

/// What a cell's toolbar button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellAction {
    /// Split the cell left/right, the new half an empty slot.
    SplitHorizontal,
    /// Split the cell top/bottom.
    SplitVertical,
    /// Close the window in the cell, or drop the empty slot.
    Close,
}

impl CellAction {
    const ALL: [CellAction; 3] = [
        CellAction::SplitHorizontal,
        CellAction::SplitVertical,
        CellAction::Close,
    ];
}

/// One cell of the grid, as design mode draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignCell {
    pub node: NodeId,
    /// The pane's rectangle: the cell plus half the inner gap on each side,
    /// so the panes tile the usable area exactly.
    pub rect: Rect,
    /// An empty slot rather than a window: dashed border and a `+`.
    pub empty: bool,
    /// Carries the accent border.
    pub focused: bool,
}

/// Everything design mode has on screen right now, in logical pixels. The
/// pointer path hit-tests against this rather than against the scene.
#[derive(Debug, Default, Clone)]
pub struct DesignGeometry {
    pub active: bool,
    /// The usable area the grid covers. Points outside it are not design
    /// mode's to answer for.
    pub area: Rect,
    pub cells: Vec<DesignCell>,
    pub bars: Vec<BarHandle>,
    pub corners: Vec<CornerHandle>,
    /// Shown instead of the panes when the tree is empty.
    pub presets: Vec<(Preset, Rect)>,
    pub scale: f32,
}

/// What the pointer is over.
#[derive(Debug, Clone, PartialEq)]
pub enum DesignHit {
    Bar(BarHandle),
    Corner(CornerHandle),
    /// A button on the hovered cell's toolbar.
    Toolbar(NodeId, CellAction),
    Preset(Preset),
    /// The body of a pane: a window's cell, or an empty slot.
    Pane(NodeId),
}

/// What is highlighted, so a redraw can show it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hover {
    Bar(usize),
    Corner(usize),
    Pane(usize),
    Preset(usize),
}

fn colour(c: (f32, f32, f32, f32)) -> PaintColor {
    PaintColor::Solid {
        color: Color::new_rgba(c.0, c.1, c.2, c.3),
    }
}

/// The toolbar's rectangle inside a pane, in logical pixels. `None` when the
/// pane is too small to hold it.
pub fn toolbar_rect(cell: Rect) -> Option<Rect> {
    let w = BUTTON * 3.0 + BUTTON_GAP * 2.0 + TOOLBAR_PAD * 2.0;
    let h = BUTTON + TOOLBAR_PAD * 2.0;
    if (cell.w as f32) < w + 8.0 || (cell.h as f32) < h + 8.0 {
        return None;
    }
    Some(Rect::new(
        cell.x + ((cell.w as f32 - w) / 2.0).round() as i32,
        cell.y + ((cell.h as f32 - h) / 2.0).round() as i32,
        w.round() as i32,
        h.round() as i32,
    ))
}

/// The three button rectangles inside a toolbar.
pub fn toolbar_buttons(bar: Rect) -> [(CellAction, Rect); 3] {
    let mut out = [(CellAction::SplitHorizontal, bar); 3];
    for (i, action) in CellAction::ALL.into_iter().enumerate() {
        let x = bar.x as f32 + TOOLBAR_PAD + i as f32 * (BUTTON + BUTTON_GAP);
        out[i] = (
            action,
            Rect::new(
                x.round() as i32,
                bar.y + TOOLBAR_PAD.round() as i32,
                BUTTON.round() as i32,
                BUTTON.round() as i32,
            ),
        );
    }
    out
}

/// Where the preset buttons sit in `area`: one row, centred.
pub fn preset_rects(area: Rect) -> Vec<(Preset, Rect)> {
    let presets = Preset::ALL;
    let total = presets.len() as f32 * PRESET_W + (presets.len() - 1) as f32 * PRESET_GAP;
    let left = area.x as f32 + (area.w as f32 - total) / 2.0;
    let top = area.y as f32 + (area.h as f32 - PRESET_H) / 2.0;
    presets
        .into_iter()
        .enumerate()
        .map(|(i, preset)| {
            (
                preset,
                Rect::new(
                    (left + i as f32 * (PRESET_W + PRESET_GAP)).round() as i32,
                    top.round() as i32,
                    PRESET_W as i32,
                    PRESET_H as i32,
                ),
            )
        })
        .collect()
}

/// The pane grid and its handles.
pub struct TilingDesignView {
    engine: Arc<Engine>,
    /// Full-screen, non-interactive container attached beside the snap
    /// overlay's. Hidden whenever design mode is off.
    pub wrap_layer: Layer,
    panes: RwLock<Vec<Layer>>,
    bars: RwLock<Vec<Layer>>,
    corners: RwLock<Vec<Layer>>,
    presets: RwLock<Vec<Layer>>,
    /// One toolbar, moved to whichever pane the pointer is over.
    toolbar: Layer,
    /// The two shares as percentages, shown on the bar while it is dragged.
    label: Layer,
    geometry: RwLock<DesignGeometry>,
    hover: RwLock<Option<Hover>>,
    /// `(first, second)` percentages, while a bar is being dragged.
    dragging: RwLock<Option<(BarHandle, f32, f32)>>,
}

impl TilingDesignView {
    pub fn new(engine: Arc<Engine>) -> Self {
        let wrap = engine.new_layer();
        wrap.set_key("tiling_design_container");
        wrap.set_size(Size::percent(1.0, 1.0), None);
        wrap.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        // Every hit test design mode needs runs against `geometry`, in logical
        // pixels, from `surface_under` — the scene never has to answer.
        wrap.set_pointer_events(false);
        wrap.set_hidden(true);

        let toolbar = engine.new_layer();
        toolbar.set_key("tiling_design_toolbar");
        toolbar.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        toolbar.set_pointer_events(false);
        toolbar.set_opacity(0.0_f32, None);
        let _ = wrap.add_sublayer(&toolbar);

        let label = engine.new_layer();
        label.set_key("tiling_design_label");
        label.set_layout_style(taffy::style::Style {
            position: taffy::style::Position::Absolute,
            ..Default::default()
        });
        label.set_pointer_events(false);
        label.set_opacity(0.0_f32, None);
        let _ = wrap.add_sublayer(&label);

        Self {
            engine,
            wrap_layer: wrap,
            panes: RwLock::new(Vec::new()),
            bars: RwLock::new(Vec::new()),
            corners: RwLock::new(Vec::new()),
            presets: RwLock::new(Vec::new()),
            toolbar,
            label,
            geometry: RwLock::new(DesignGeometry::default()),
            hover: RwLock::new(None),
            dragging: RwLock::new(None),
        }
    }

    pub fn is_visible(&self) -> bool {
        !self.wrap_layer.hidden()
    }

    /// The geometry the pointer path hit-tests against.
    pub fn geometry(&self) -> DesignGeometry {
        self.geometry.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn is_active(&self) -> bool {
        self.geometry.read().map(|g| g.active).unwrap_or(false)
    }

    // ── Hit testing ──────────────────────────────────────────────────────

    /// What the logical point `(x, y)` is over, if anything.
    ///
    /// Corners beat bars, bars beat the toolbar, and the pane body is the
    /// fallback — a click there is a click "on the window", which leaves
    /// design mode.
    pub fn hit(&self, x: f64, y: f64) -> Option<DesignHit> {
        use crate::workspaces::tiling::design::contains;
        let geo = self.geometry();
        if !geo.active || !contains(geo.area, x, y) {
            return None;
        }
        for (preset, rect) in geo.presets.iter() {
            if contains(*rect, x, y) {
                return Some(DesignHit::Preset(*preset));
            }
        }
        for corner in geo.corners.iter() {
            if contains(corner.rect, x, y) {
                return Some(DesignHit::Corner(corner.clone()));
            }
        }
        for bar in geo.bars.iter() {
            if contains(bar.rect, x, y) {
                return Some(DesignHit::Bar(*bar));
            }
        }
        // The toolbar only exists over the pane the pointer is already on, so
        // it can only be hit from inside that pane.
        if let Some(Hover::Pane(index)) = *self.hover.read().ok()? {
            if let Some(cell) = geo.cells.get(index) {
                if let Some(bar) = toolbar_rect(cell.rect) {
                    for (action, rect) in toolbar_buttons(bar) {
                        if contains(rect, x, y) {
                            return Some(DesignHit::Toolbar(cell.node, action));
                        }
                    }
                }
            }
        }
        geo.cells
            .iter()
            .find(|c| contains(c.rect, x, y))
            .map(|c| DesignHit::Pane(c.node))
    }

    /// The cursor a hit asks for.
    pub fn cursor_for(hit: &DesignHit) -> smithay::input::pointer::CursorIcon {
        use smithay::input::pointer::CursorIcon;
        match hit {
            DesignHit::Bar(bar) if bar.axis == Axis::Row => CursorIcon::EwResize,
            DesignHit::Bar(_) => CursorIcon::NsResize,
            DesignHit::Corner(_) => CursorIcon::NwseResize,
            DesignHit::Toolbar(..) | DesignHit::Preset(_) => CursorIcon::Pointer,
            DesignHit::Pane(_) => CursorIcon::default(),
        }
    }

    /// Note what the pointer is over, so the handles light up and the hovered
    /// pane shows its toolbar. Returns true when something changed.
    pub fn set_hover(&self, hit: Option<&DesignHit>) -> bool {
        use crate::workspaces::tiling::design::contains;
        let geo = self.geometry();
        let hover = hit.and_then(|hit| match hit {
            DesignHit::Bar(bar) => geo
                .bars
                .iter()
                .position(|b| b.container == bar.container && b.index == bar.index)
                .map(Hover::Bar),
            DesignHit::Corner(corner) => geo
                .corners
                .iter()
                .position(|c| c.rect == corner.rect)
                .map(Hover::Corner),
            DesignHit::Preset(preset) => geo
                .presets
                .iter()
                .position(|(p, _)| p == preset)
                .map(Hover::Preset),
            DesignHit::Pane(node) | DesignHit::Toolbar(node, _) => geo
                .cells
                .iter()
                .position(|c| c.node == *node)
                .map(Hover::Pane),
        });
        // A toolbar button keeps the pane hovered, so the toolbar does not
        // vanish out from under the pointer on its way to a button.
        let _ = contains;
        let Ok(mut current) = self.hover.write() else {
            return false;
        };
        if *current == hover {
            return false;
        }
        *current = hover;
        drop(current);
        self.paint_hover(&geo);
        true
    }

    // ── Building the scene ───────────────────────────────────────────────

    /// Rebuild the grid from `geometry`, animating everything that moved with
    /// `transition`. `origin` is the output's own origin in logical pixels,
    /// subtracted so the layers land in output-local scene space.
    pub fn update(
        &self,
        geometry: DesignGeometry,
        origin: (i32, i32),
        transition: Option<Transition>,
    ) {
        if !geometry.active {
            self.hide();
            return;
        }
        let first_show = self.wrap_layer.hidden();
        self.wrap_layer.set_hidden(false);
        let scale = if geometry.scale > 0.0 {
            geometry.scale
        } else {
            1.0
        };
        // Nothing flies in from the corner on the first frame: the panes are
        // placed and faded up, exactly as the snap overlay does.
        let move_transition = if first_show { None } else { transition };
        let to_px = |rect: Rect| {
            (
                (rect.x - origin.0) as f32 * scale,
                (rect.y - origin.1) as f32 * scale,
                rect.w as f32 * scale,
                rect.h as f32 * scale,
            )
        };

        // Panes.
        {
            let mut panes = self.panes.write().unwrap();
            self.resize_pool(&mut panes, geometry.cells.len(), "tiling_design_pane");
            for (layer, cell) in panes.iter().zip(geometry.cells.iter()) {
                let (x, y, w, h) = to_px(cell.rect);
                layer.set_hidden(false);
                layer.set_background_color(colour(FILL), None);
                layer.set_border_width(BORDER_WIDTH * scale, None);
                layer.set_border_corner_radius(BorderRadius::new_single(RADIUS * scale), None);
                layer.set_border_color(
                    if cell.focused {
                        PaintColor::Solid {
                            color: crate::theme::accent_color(),
                        }
                    } else if cell.empty {
                        // The dash is drawn; the lay-rs border would fight it.
                        colour((1.0, 1.0, 1.0, 0.0))
                    } else {
                        colour(BORDER)
                    },
                    None,
                );
                if cell.empty {
                    setup_empty_slot(layer, scale, cell.focused);
                } else {
                    // A window's pane is the plain translucent rectangle: the
                    // fill and border are enough, nothing is drawn into it.
                    layer.set_draw_content(|_: &layers::skia::Canvas, w: f32, h: f32| {
                        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
                    });
                }
                layer.set_position(Point { x, y }, move_transition.clone());
                layer.set_size(Size::points(w, h), move_transition.clone());
                layer.set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.2)));
            }
            for layer in panes.iter().skip(geometry.cells.len()) {
                layer.set_hidden(true);
            }
        }

        // Bars and corners.
        {
            let mut bars = self.bars.write().unwrap();
            self.resize_pool(&mut bars, geometry.bars.len(), "tiling_design_bar");
            for (layer, bar) in bars.iter().zip(geometry.bars.iter()) {
                let (x, y, w, h) = to_px(bar.rect);
                layer.set_hidden(false);
                layer.set_background_color(colour((1.0, 1.0, 1.0, 0.25)), None);
                layer.set_border_corner_radius(BorderRadius::new_single(4.0 * scale), None);
                layer.set_position(Point { x, y }, move_transition.clone());
                layer.set_size(Size::points(w, h), move_transition.clone());
                layer.set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.2)));
            }
            for layer in bars.iter().skip(geometry.bars.len()) {
                layer.set_hidden(true);
            }
        }
        {
            let mut corners = self.corners.write().unwrap();
            self.resize_pool(&mut corners, geometry.corners.len(), "tiling_design_corner");
            for (layer, corner) in corners.iter().zip(geometry.corners.iter()) {
                let (x, y, w, h) = to_px(corner.rect);
                layer.set_hidden(false);
                layer.set_background_color(colour((1.0, 1.0, 1.0, 0.45)), None);
                layer.set_border_corner_radius(BorderRadius::new_single(4.0 * scale), None);
                layer.set_position(Point { x, y }, move_transition.clone());
                layer.set_size(Size::points(w, h), move_transition.clone());
                layer.set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.2)));
            }
            for layer in corners.iter().skip(geometry.corners.len()) {
                layer.set_hidden(true);
            }
        }

        // Presets, on an empty workspace.
        {
            let mut presets = self.presets.write().unwrap();
            self.resize_pool(&mut presets, geometry.presets.len(), "tiling_design_preset");
            for (index, (layer, (preset, rect))) in
                presets.iter().zip(geometry.presets.iter()).enumerate()
            {
                let (x, y, w, h) = to_px(*rect);
                layer.set_hidden(false);
                layer.set_background_color(colour(FILL), None);
                layer.set_border_width(BORDER_WIDTH * scale, None);
                layer.set_border_color(colour(BORDER), None);
                layer.set_border_corner_radius(BorderRadius::new_single(8.0 * scale), None);
                setup_preset_thumbnail(layer, *preset, scale);
                layer.set_position(Point { x, y }, None);
                layer.set_size(Size::points(w, h), None);
                layer.set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.2)));
                let _ = index;
            }
            for layer in presets.iter().skip(geometry.presets.len()) {
                layer.set_hidden(true);
            }
        }

        if let Ok(mut stored) = self.geometry.write() {
            *stored = geometry;
        }
        let geo = self.geometry();
        self.paint_hover(&geo);
        self.paint_label(&geo, origin);
    }

    /// Fade everything out and hide the container. Nothing about the windows
    /// changes: leaving design mode leaves the layout exactly as it stands.
    pub fn hide(&self) {
        if let Ok(mut geo) = self.geometry.write() {
            // Cleared, not just deactivated: nothing may still hit-test
            // against a grid that is on its way out.
            *geo = DesignGeometry::default();
        }
        if let Ok(mut hover) = self.hover.write() {
            *hover = None;
        }
        if let Ok(mut drag) = self.dragging.write() {
            *drag = None;
        }
        if self.wrap_layer.hidden() {
            return;
        }
        let fade = Some(Transition::ease_out_quad(0.15));
        for pool in [&self.panes, &self.bars, &self.corners, &self.presets] {
            if let Ok(layers) = pool.read() {
                for layer in layers.iter() {
                    layer.set_opacity(0.0_f32, fade.clone());
                }
            }
        }
        self.toolbar.set_opacity(0.0_f32, fade.clone());
        let wrap = self.wrap_layer.clone();
        self.label
            .set_opacity(0.0_f32, fade)
            .on_finish(move |_l: &Layer, _| wrap.set_hidden(true), true);
    }

    /// Remember the shares a drag is showing, so the bar's label can say them.
    pub fn set_drag_label(&self, bar: Option<(BarHandle, f32, f32)>) {
        if let Ok(mut drag) = self.dragging.write() {
            *drag = bar;
        }
    }

    // ── Painting the transient bits ──────────────────────────────────────

    fn paint_hover(&self, geo: &DesignGeometry) {
        let hover = self.hover.read().ok().and_then(|h| *h);
        let scale = if geo.scale > 0.0 { geo.scale } else { 1.0 };

        if let Ok(bars) = self.bars.read() {
            for (index, layer) in bars.iter().enumerate() {
                let lit = hover == Some(Hover::Bar(index));
                layer.set_background_color(
                    colour(if lit {
                        (1.0, 1.0, 1.0, 0.6)
                    } else {
                        (1.0, 1.0, 1.0, 0.25)
                    }),
                    Some(Transition::ease_out_quad(0.1)),
                );
            }
        }
        if let Ok(corners) = self.corners.read() {
            for (index, layer) in corners.iter().enumerate() {
                let lit = hover == Some(Hover::Corner(index));
                layer.set_background_color(
                    colour(if lit {
                        (1.0, 1.0, 1.0, 0.8)
                    } else {
                        (1.0, 1.0, 1.0, 0.45)
                    }),
                    Some(Transition::ease_out_quad(0.1)),
                );
            }
        }
        if let Ok(presets) = self.presets.read() {
            for (index, layer) in presets.iter().enumerate() {
                let lit = hover == Some(Hover::Preset(index));
                layer.set_background_color(
                    colour(if lit { (1.0, 1.0, 1.0, 0.5) } else { FILL }),
                    Some(Transition::ease_out_quad(0.1)),
                );
            }
        }

        // The toolbar rides on the hovered pane.
        let toolbar = match hover {
            Some(Hover::Pane(index)) => geo
                .cells
                .get(index)
                .and_then(|cell| toolbar_rect(cell.rect).map(|r| (r, cell.empty))),
            _ => None,
        };
        match toolbar {
            Some((rect, empty)) => {
                let x = (rect.x - geo.area.x) as f32;
                let _ = x;
                self.toolbar.set_position(
                    Point {
                        x: rect.x as f32 * scale,
                        y: rect.y as f32 * scale,
                    },
                    None,
                );
                self.toolbar.set_size(
                    Size::points(rect.w as f32 * scale, rect.h as f32 * scale),
                    None,
                );
                setup_toolbar(&self.toolbar, scale, empty);
                self.toolbar
                    .set_opacity(1.0_f32, Some(Transition::ease_out_quad(0.12)));
            }
            None => {
                self.toolbar
                    .set_opacity(0.0_f32, Some(Transition::ease_out_quad(0.12)));
            }
        }
    }

    fn paint_label(&self, geo: &DesignGeometry, origin: (i32, i32)) {
        let scale = if geo.scale > 0.0 { geo.scale } else { 1.0 };
        let drag = self.dragging.read().ok().and_then(|d| *d);
        let Some((bar, first, second)) = drag else {
            self.label
                .set_opacity(0.0_f32, Some(Transition::ease_out_quad(0.1)));
            return;
        };
        let w = 92.0 * scale;
        let h = 24.0 * scale;
        let cx = (bar.rect.x - origin.0) as f32 * scale + bar.rect.w as f32 * scale / 2.0;
        let cy = (bar.rect.y - origin.1) as f32 * scale + bar.rect.h as f32 * scale / 2.0;
        self.label.set_position(
            Point {
                x: cx - w / 2.0,
                y: cy - h / 2.0,
            },
            None,
        );
        self.label.set_size(Size::points(w, h), None);
        setup_label(&self.label, first, second, scale);
        self.label.set_opacity(1.0_f32, None);
    }

    /// Grow or shrink a pool of layers to `len`, keeping the ones already
    /// there so a relayout animates rather than popping.
    fn resize_pool(&self, pool: &mut Vec<Layer>, len: usize, key: &str) {
        while pool.len() < len {
            let layer = self.engine.new_layer();
            layer.set_key(format!("{key}_{}", pool.len()));
            layer.set_layout_style(taffy::style::Style {
                position: taffy::style::Position::Absolute,
                ..Default::default()
            });
            layer.set_pointer_events(false);
            layer.set_opacity(0.0_f32, None);
            let _ = self.wrap_layer.add_sublayer(&layer);
            pool.push(layer);
        }
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// An empty slot: a dashed border and a `+` in the middle.
fn setup_empty_slot(layer: &Layer, scale: f32, focused: bool) {
    layer.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        let colour = if focused {
            crate::theme::accent_color()
        } else {
            Color::new_rgba(1.0, 1.0, 1.0, 0.8)
        };
        let mut paint = layers::skia::Paint::new(colour.c4f(), None);
        paint.set_anti_alias(true);
        paint.set_style(layers::skia::paint::Style::Stroke);
        paint.set_stroke_width(2.0 * scale);
        let dash = layers::skia::PathEffect::dash(&[8.0 * scale, 6.0 * scale], 0.0);
        paint.set_path_effect(dash);
        let inset = 1.0 * scale;
        let rect = layers::skia::Rect::from_xywh(inset, inset, w - 2.0 * inset, h - 2.0 * inset);
        let rrect = layers::skia::RRect::new_rect_xy(rect, 12.0 * scale, 12.0 * scale);
        canvas.draw_rrect(rrect, &paint);

        // The `+`.
        let mut plus = layers::skia::Paint::new(colour.c4f(), None);
        plus.set_anti_alias(true);
        plus.set_style(layers::skia::paint::Style::Stroke);
        plus.set_stroke_width(3.0 * scale);
        let arm = (14.0 * scale).min(w / 4.0).min(h / 4.0);
        canvas.draw_line((w / 2.0 - arm, h / 2.0), (w / 2.0 + arm, h / 2.0), &plus);
        canvas.draw_line((w / 2.0, h / 2.0 - arm), (w / 2.0, h / 2.0 + arm), &plus);
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    });
}

/// The hover toolbar: a dark pill with split-horizontal, split-vertical and
/// close, drawn the way the titlebar controls are — Skia straight onto the
/// layer, no client and no otto-kit.
fn setup_toolbar(layer: &Layer, scale: f32, empty: bool) {
    layer.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        let mut bg = layers::skia::Paint::new(Color::new_rgba(0.1, 0.1, 0.12, 0.85).c4f(), None);
        bg.set_anti_alias(true);
        let rect = layers::skia::Rect::from_xywh(0.0, 0.0, w, h);
        canvas.draw_rrect(
            layers::skia::RRect::new_rect_xy(rect, h / 2.0, h / 2.0),
            &bg,
        );

        let mut ink = layers::skia::Paint::new(Color::new_rgba(1.0, 1.0, 1.0, 0.9).c4f(), None);
        ink.set_anti_alias(true);
        ink.set_style(layers::skia::paint::Style::Stroke);
        ink.set_stroke_width(1.6 * scale);

        let pad = TOOLBAR_PAD * scale;
        let button = BUTTON * scale;
        let gap = BUTTON_GAP * scale;
        for (i, action) in CellAction::ALL.into_iter().enumerate() {
            let x = pad + i as f32 * (button + gap);
            let y = pad;
            let inset = button * 0.22;
            let bx = x + inset;
            let by = y + inset;
            let bw = button - 2.0 * inset;
            let bh = button - 2.0 * inset;
            match action {
                CellAction::SplitHorizontal => {
                    canvas.draw_rect(layers::skia::Rect::from_xywh(bx, by, bw, bh), &ink);
                    canvas.draw_line((bx + bw / 2.0, by), (bx + bw / 2.0, by + bh), &ink);
                }
                CellAction::SplitVertical => {
                    canvas.draw_rect(layers::skia::Rect::from_xywh(bx, by, bw, bh), &ink);
                    canvas.draw_line((bx, by + bh / 2.0), (bx + bw, by + bh / 2.0), &ink);
                }
                CellAction::Close => {
                    canvas.draw_line((bx, by), (bx + bw, by + bh), &ink);
                    canvas.draw_line((bx + bw, by), (bx, by + bh), &ink);
                }
            }
        }
        let _ = empty;
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    });
}

/// `48% · 52%` on the bar being dragged.
fn setup_label(layer: &Layer, first: f32, second: f32, scale: f32) {
    let text = format!(
        "{}%  ·  {}%",
        (first * 100.0).round() as i32,
        (second * 100.0).round() as i32
    );
    layer.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        let mut bg = layers::skia::Paint::new(Color::new_rgba(0.1, 0.1, 0.12, 0.85).c4f(), None);
        bg.set_anti_alias(true);
        canvas.draw_rrect(
            layers::skia::RRect::new_rect_xy(
                layers::skia::Rect::from_xywh(0.0, 0.0, w, h),
                h / 2.0,
                h / 2.0,
            ),
            &bg,
        );
        let typeface =
            layers::skia::FontMgr::new().match_family_style("", layers::skia::FontStyle::default());
        let Some(typeface) = typeface else {
            return layers::skia::Rect::from_xywh(0.0, 0.0, w, h);
        };
        let font = layers::skia::Font::from_typeface(typeface, 12.0 * scale);
        let paint = layers::skia::Paint::new(Color::new_rgba(1.0, 1.0, 1.0, 0.95).c4f(), None);
        let (width, _) = font.measure_str(&text, Some(&paint));
        canvas.draw_str(
            &text,
            ((w - width) / 2.0, h / 2.0 + 4.0 * scale),
            &font,
            &paint,
        );
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    });
}

/// A little wireframe of the shape a preset builds.
fn setup_preset_thumbnail(layer: &Layer, preset: Preset, scale: f32) {
    layer.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        let mut ink = layers::skia::Paint::new(Color::new_rgba(1.0, 1.0, 1.0, 0.9).c4f(), None);
        ink.set_anti_alias(true);
        ink.set_style(layers::skia::paint::Style::Stroke);
        ink.set_stroke_width(2.0 * scale);
        let pad = 14.0 * scale;
        let (x, y, iw, ih) = (pad, pad, w - 2.0 * pad, h - 2.0 * pad);
        let cell = |cx: f32, cy: f32, cw: f32, ch: f32, canvas: &layers::skia::Canvas| {
            let gap = 2.0 * scale;
            canvas.draw_rrect(
                layers::skia::RRect::new_rect_xy(
                    layers::skia::Rect::from_xywh(
                        x + cx * iw + gap,
                        y + cy * ih + gap,
                        cw * iw - 2.0 * gap,
                        ch * ih - 2.0 * gap,
                    ),
                    3.0 * scale,
                    3.0 * scale,
                ),
                &ink,
            );
        };
        match preset {
            Preset::TwoColumns => {
                cell(0.0, 0.0, 0.5, 1.0, canvas);
                cell(0.5, 0.0, 0.5, 1.0, canvas);
            }
            Preset::ThreeColumns => {
                for i in 0..3 {
                    cell(i as f32 / 3.0, 0.0, 1.0 / 3.0, 1.0, canvas);
                }
            }
            Preset::MainAndStack => {
                cell(0.0, 0.0, 0.5, 1.0, canvas);
                cell(0.5, 0.0, 0.5, 0.5, canvas);
                cell(0.5, 0.5, 0.5, 0.5, canvas);
            }
            Preset::Grid => {
                for row in 0..2 {
                    for col in 0..2 {
                        cell(col as f32 * 0.5, row as f32 * 0.5, 0.5, 0.5, canvas);
                    }
                }
            }
        }
        layers::skia::Rect::from_xywh(0.0, 0.0, w, h)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toolbar_is_centred_in_its_pane() {
        let cell = Rect::new(100, 100, 400, 300);
        let bar = toolbar_rect(cell).expect("a 400x300 pane has room");
        assert_eq!(bar.x + bar.w / 2, cell.x + cell.w / 2);
        assert_eq!(bar.y + bar.h / 2, cell.y + cell.h / 2);
    }

    #[test]
    fn a_tiny_pane_has_no_toolbar() {
        assert!(toolbar_rect(Rect::new(0, 0, 60, 60)).is_none());
    }

    #[test]
    fn the_three_buttons_sit_inside_the_toolbar_in_order() {
        let bar = toolbar_rect(Rect::new(0, 0, 800, 600)).unwrap();
        let buttons = toolbar_buttons(bar);
        assert_eq!(buttons[0].0, CellAction::SplitHorizontal);
        assert_eq!(buttons[2].0, CellAction::Close);
        for (_, rect) in buttons.iter() {
            assert!(rect.x >= bar.x && rect.x + rect.w <= bar.x + bar.w);
            assert!(rect.y >= bar.y && rect.y + rect.h <= bar.y + bar.h);
        }
        assert!(buttons[0].1.x < buttons[1].1.x && buttons[1].1.x < buttons[2].1.x);
    }

    #[test]
    fn the_preset_row_is_centred_and_complete() {
        let area = Rect::new(0, 30, 1200, 700);
        let rects = preset_rects(area);
        assert_eq!(rects.len(), Preset::ALL.len());
        let left = rects.first().unwrap().1;
        let right = rects.last().unwrap().1;
        let centre = (left.x + right.x + right.w) / 2;
        assert!((centre - (area.x + area.w / 2)).abs() <= 1, "{centre}");
    }
}

// ── Input target ─────────────────────────────────────────────────────────

/// The whole grid, as one pointer target.
///
/// Design mode owns every point of the usable area while it is up — the panes
/// cover the windows — so one view answers for all of it and dispatches on
/// what [`TilingDesignView::hit`] says is under the pointer. It is hit-tested
/// ahead of windows in `surface_under`, and *only* while design mode is
/// active, so pointer handling on a tiled workspace is untouched otherwise.
#[derive(Clone, Default)]
pub struct TilingDesignInputView {
    /// The last motion, so a button event knows where it happened: Smithay's
    /// `ButtonEvent` carries no location.
    last: Arc<RwLock<(f64, f64)>>,
    /// When the last press landed, for double-click detection.
    last_press: Arc<RwLock<Option<std::time::Instant>>>,
}

/// How close together two presses have to be to count as a double click.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

const BTN_LEFT: u32 = 0x110;

impl std::fmt::Debug for TilingDesignInputView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TilingDesignInputView")
    }
}

impl PartialEq for TilingDesignInputView {
    fn eq(&self, _other: &Self) -> bool {
        // There is only ever one grid.
        true
    }
}

impl<Backend: crate::state::Backend> crate::interactive_view::ViewInteractions<Backend>
    for TilingDesignInputView
{
    fn id(&self) -> Option<usize> {
        // A constant of its own, so crossing between handles is not read as a
        // change of target — the cursor is set from `on_motion` regardless.
        Some(0x7111_de51)
    }

    fn is_alive(&self) -> bool {
        true
    }

    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        if let Ok(mut last) = self.last.write() {
            *last = (event.location.x, event.location.y);
        }
        data.tiling_design_motion(event.location.x, event.location.y);
    }

    fn on_leave_with_data(
        &self,
        data: &mut crate::Otto<Backend>,
        _serial: smithay::utils::Serial,
        _time: u32,
    ) {
        data.workspaces.tiling_design.set_hover(None);
        data.set_cursor(&smithay::input::pointer::CursorImageStatus::Named(
            smithay::input::pointer::CursorIcon::default(),
        ));
    }

    fn on_button(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        if event.button != BTN_LEFT {
            return;
        }
        let (x, y) = self.last.read().map(|l| *l).unwrap_or((0.0, 0.0));
        let pressed = matches!(event.state, smithay::backend::input::ButtonState::Pressed);
        let mut double = false;
        if pressed {
            if let Ok(mut last) = self.last_press.write() {
                let now = std::time::Instant::now();
                double = last.is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK);
                *last = Some(now);
            }
        }
        data.tiling_design_button(pressed, double, x, y);
    }
}
