//! The Graph section's Matrix and Lanes views.
//!
//! Both are drawn from the patchbay's scene, so its rows, wires and saved
//! state are theirs too: a matrix cell is a pair of patchbay rows, and
//! selecting one selects that wire, so the toolbar and the details panel
//! act on it as they do on the canvas.

// Rust guideline compliant 2026-02-21

use super::*;
use otto_audio::graph_view::{canvas_ground, card_ground, Cell, GraphMode, MatrixLine};

/// The matrix's row headers: a node's name and what it is.
const ROW_HEADER_W: f32 = 210.0;
/// The matrix's column headers, two lines over each column.
const COLUMN_HEADER_H: f32 = 58.0;
/// One cell of the matrix.
const CELL_W: f32 = 124.0;
const CELL_H: f32 = 44.0;
/// A cell's mark: filled when saved, a ring when not.
const MARK_R: f32 = 8.0;
/// The spacing of the hatching on a node's own cell.
const HATCH: f32 = 6.0;

/// One lane, and the room between lanes and around them.
const LANE_H: f32 = 62.0;
const LANE_GAP: f32 = 10.0;
const LANES_MARGIN: f32 = 16.0;
/// The line over the lanes that says what they are.
const LANES_NOTE_H: f32 = 36.0;
/// A step's pill: its height and the range of its width.
const PILL_H: f32 = 40.0;
const PILL_MIN_W: f32 = 64.0;
const PILL_MAX_W: f32 = 210.0;
/// The arrow between two pills.
const ARROW_W: f32 = 30.0;
/// Room at a lane's right end for whether it is saved.
const LANE_STATUS_W: f32 = 100.0;

/// The scroll positions of the Matrix and Lanes views.
pub(super) struct ViewScrolls {
    /// The matrix's rows, up and down.
    rows: ScrollView,
    /// The matrix's columns, left and right.
    columns: ScrollView,
    lanes: ScrollView,
}

impl Default for ViewScrolls {
    fn default() -> Self {
        let none = Rect::from_xywh(0.0, 0.0, 0.0, 0.0);
        Self {
            rows: ScrollView::new(none),
            columns: ScrollView::horizontal(none),
            lanes: ScrollView::new(none),
        }
    }
}

impl ViewScrolls {
    /// Whether the view `mode` is still coasting or bouncing.
    pub(super) fn is_animating(&self, mode: GraphMode) -> bool {
        match mode {
            GraphMode::Patchbay => false,
            GraphMode::Matrix => self.rows.is_animating() || self.columns.is_animating(),
            GraphMode::Lanes => self.lanes.is_animating(),
        }
    }
}

/// One step of a lane as drawn.
struct Step {
    node: u32,
    title: String,
    kind: String,
    /// Between a source and a destination: an effect or a virtual device.
    between: bool,
}

impl Ui {
    /// The matrix in `viewport`: outputs down the side, inputs across.
    pub(super) fn draw_matrix(
        &mut self,
        canvas: &Canvas,
        scene: &Scene,
        viewport: Rect,
        theme: &Theme,
    ) {
        let m = graph_view::matrix(scene, &self.graph);
        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));
        canvas.draw_rect(viewport, &fill(card_ground(theme)));
        if m.is_empty() {
            canvas.restore();
            if self.graph.is_synced() {
                Label::new(t!("audio-graph-empty"))
                    .with_style(styles::BODY)
                    .with_color(theme.text_secondary)
                    .centered_at(viewport.center_x(), viewport.center_y())
                    .render(canvas);
            }
            return;
        }
        let cells = Rect::from_ltrb(
            viewport.left + ROW_HEADER_W,
            viewport.top + COLUMN_HEADER_H,
            viewport.right.max(viewport.left + ROW_HEADER_W + 1.0),
            viewport.bottom.max(viewport.top + COLUMN_HEADER_H + 1.0),
        );
        let scrolls = &mut self.views;
        scrolls.rows.set_viewport(cells);
        scrolls
            .rows
            .set_content_length(m.rows.len() as f32 * CELL_H + 8.0);
        scrolls.rows.tick();
        scrolls.columns.set_viewport(cells);
        scrolls
            .columns
            .set_content_length(m.columns.len() as f32 * CELL_W + 8.0);
        scrolls.columns.tick();
        let ox = cells.left - scrolls.columns.offset();
        let oy = cells.top - scrolls.rows.offset();
        let selected_node = self.view.selected_node;
        let selected_wire = self.view.selected_wire;
        let tint =
            |color: Color, alpha: u8| Color::from_argb(alpha, color.r(), color.g(), color.b());
        let accent = theme.accent;

        // The cells.
        canvas.save();
        canvas.clip_rect(cells, None, Some(true));
        for (r, row) in m.rows.iter().enumerate() {
            let top = oy + r as f32 * CELL_H;
            if top + CELL_H < cells.top || top > cells.bottom {
                continue;
            }
            for (c, col) in m.columns.iter().enumerate() {
                let left = ox + c as f32 * CELL_W;
                if left + CELL_W < cells.left || left > cells.right {
                    continue;
                }
                let rect = Rect::from_xywh(left, top, CELL_W, CELL_H);
                let wire_selected = selected_wire == Some((row.key, col.key));
                if wire_selected {
                    canvas.draw_rect(rect, &fill(tint(accent, 0x22)));
                } else if selected_node.is_some_and(|n| n == row.key.node || n == col.key.node) {
                    canvas.draw_rect(rect, &fill(theme.fill_quaternary));
                }
                let center = Point::new(rect.center_x(), rect.center_y());
                let color = media_color(theme, row.key.media);
                let action = match m.cell(r, c) {
                    Cell::Unavailable => {
                        if row.key.node == col.key.node {
                            draw_hatch(canvas, rect, theme);
                        }
                        None
                    }
                    Cell::Open => {
                        canvas.draw_circle(
                            center,
                            MARK_R - 1.0,
                            &stroke(theme.fill_secondary, 1.5),
                        );
                        Some(Action::LinkCell(row.key, col.key))
                    }
                    Cell::Linked { saved, links } => {
                        if wire_selected {
                            canvas.draw_circle(center, MARK_R + 5.0, &fill(tint(color, 0x40)));
                        }
                        if saved {
                            canvas.draw_circle(center, MARK_R + 1.0, &fill(color));
                        } else {
                            canvas.draw_circle(center, MARK_R, &fill(card_ground(theme)));
                            canvas.draw_circle(center, MARK_R - 0.25, &stroke(color, 2.5));
                        }
                        // As on a folded wire: the count, when it isn't the
                        // usual two of a stereo pair.
                        let folded = row.key.port.is_none() || col.key.port.is_none();
                        if folded && links != 2 {
                            Label::new(links.to_string())
                                .with_style(styles::CAPTION_2_EMPHASIZED)
                                .with_color(if saved { Color::WHITE } else { color })
                                .centered_at(center.x, center.y)
                                .render(canvas);
                        }
                        Some(Action::SelectWire(row.key, col.key))
                    }
                };
                if let (Some(action), Some(hit)) = (action, clipped(rect, cells)) {
                    self.targets.push((hit, action));
                }
            }
        }
        let grid = stroke(theme.hairline, 1.0);
        for r in 0..=m.rows.len() {
            let y = oy + r as f32 * CELL_H;
            canvas.draw_line(Point::new(cells.left, y), Point::new(cells.right, y), &grid);
        }
        for c in 0..=m.columns.len() {
            let x = ox + c as f32 * CELL_W;
            canvas.draw_line(Point::new(x, cells.top), Point::new(x, cells.bottom), &grid);
        }
        canvas.restore();

        // Row headers, following the rows up and down.
        let side = Rect::from_ltrb(viewport.left, cells.top, cells.left, cells.bottom);
        canvas.save();
        canvas.clip_rect(side, None, Some(true));
        for (r, row) in m.rows.iter().enumerate() {
            let rect = Rect::from_xywh(side.left, oy + r as f32 * CELL_H, ROW_HEADER_W, CELL_H);
            if rect.bottom < side.top || rect.top > side.bottom {
                continue;
            }
            let selected = selected_node == Some(row.key.node);
            self.draw_matrix_header(canvas, rect, row, selected, theme);
            canvas.draw_line(
                Point::new(rect.left, rect.top),
                Point::new(rect.right, rect.top),
                &grid,
            );
        }
        canvas.restore();

        // Column headers, following the columns left and right.
        let head = Rect::from_ltrb(cells.left, viewport.top, cells.right, cells.top);
        canvas.save();
        canvas.clip_rect(head, None, Some(true));
        for (c, col) in m.columns.iter().enumerate() {
            let rect = Rect::from_xywh(ox + c as f32 * CELL_W, head.top, CELL_W, COLUMN_HEADER_H);
            if rect.right < head.left || rect.left > head.right {
                continue;
            }
            let selected = selected_node == Some(col.key.node);
            self.draw_matrix_header(canvas, rect, col, selected, theme);
            canvas.draw_line(
                Point::new(rect.left, rect.top + 8.0),
                Point::new(rect.left, rect.bottom),
                &grid,
            );
        }
        canvas.restore();

        // The corner, and the rules that frame the cells.
        Label::new(t!("audio-matrix-corner"))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_tertiary)
            .centered_on(viewport.left + 16.0, viewport.top + COLUMN_HEADER_H - 16.0)
            .render(canvas);
        canvas.draw_line(
            Point::new(viewport.left, cells.top),
            Point::new(viewport.right, cells.top),
            &grid,
        );
        canvas.draw_line(
            Point::new(cells.left, viewport.top),
            Point::new(cells.left, viewport.bottom),
            &grid,
        );
        self.views.rows.render(canvas, theme, |_, _| {});
        self.views.columns.render(canvas, theme, |_, _| {});
        canvas.restore();
    }

    /// A row's or column's header: the media's dot, the node's name and
    /// what it is. Pressing it selects the node.
    fn draw_matrix_header(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        line: &MatrixLine,
        selected: bool,
        theme: &Theme,
    ) {
        if selected {
            let a = theme.accent;
            canvas.draw_rect(rect, &fill(Color::from_argb(0x22, a.r(), a.g(), a.b())));
        }
        let left = rect.left + 14.0;
        let bottom = rect.bottom - CELL_H / 2.0;
        let width = rect.right - 8.0 - (left + 12.0);
        canvas.draw_circle(
            Point::new(left + 3.0, bottom - 7.0),
            3.5,
            &fill(media_color(theme, line.key.media)),
        );
        let title_font = styles::SUBHEADLINE_EMPHASIZED.font();
        Label::new(ellipsize(&title_font, &line.title, width))
            .with_style(styles::SUBHEADLINE_EMPHASIZED)
            .with_color(theme.text_primary)
            .centered_on(left + 12.0, bottom - 7.0)
            .render(canvas);
        let kind_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&kind_font, &line.kind, width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left + 12.0, bottom + 9.0)
            .render(canvas);
        self.targets.push((rect, Action::SelectNode(line.key.node)));
    }

    /// The lanes in `viewport`, one path from source to destination each.
    pub(super) fn draw_lanes(
        &mut self,
        canvas: &Canvas,
        scene: &Scene,
        viewport: Rect,
        theme: &Theme,
    ) {
        let lanes = graph_view::lanes(scene, &self.graph);
        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));
        canvas.draw_rect(viewport, &fill(canvas_ground(theme)));
        if lanes.is_empty() {
            canvas.restore();
            if self.graph.is_synced() {
                Label::new(t!("audio-lanes-empty"))
                    .with_style(styles::BODY)
                    .with_color(theme.text_secondary)
                    .centered_at(viewport.center_x(), viewport.center_y())
                    .render(canvas);
            }
            return;
        }
        let length = LANES_NOTE_H + lanes.len() as f32 * (LANE_H + LANE_GAP) + LANES_MARGIN;
        self.views.lanes.set_viewport(viewport);
        self.views.lanes.set_content_length(length);
        self.views.lanes.tick();
        let top = viewport.top - self.views.lanes.offset();
        let left = viewport.left + LANES_MARGIN;
        let width = viewport.width() - 2.0 * LANES_MARGIN;

        let note_font = styles::SUBHEADLINE.font();
        Label::new(ellipsize(&note_font, t!("audio-lanes-note"), width))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(left, top + LANES_NOTE_H / 2.0 + 4.0)
            .render(canvas);

        for (index, lane) in lanes.iter().enumerate() {
            let y = top + LANES_NOTE_H + index as f32 * (LANE_H + LANE_GAP);
            let rect = Rect::from_xywh(left, y, width, LANE_H);
            if rect.bottom < viewport.top || rect.top > viewport.bottom {
                continue;
            }
            let steps: Vec<Step> = lane
                .steps
                .iter()
                .map(|node| {
                    let card = scene.card(*node);
                    Step {
                        node: *node,
                        title: card.map_or_else(String::new, |c| c.title.clone()),
                        kind: card.map_or_else(String::new, |c| c.subtitle.clone()),
                        between: self
                            .graph
                            .node(*node)
                            .is_some_and(|n| graph_view::column(n) == 1),
                    }
                })
                .collect();
            self.draw_lane(
                canvas,
                rect,
                &steps,
                &lane.saved,
                lane.media,
                viewport,
                theme,
            );
        }
        self.views.lanes.render(canvas, theme, |_, _| {});
        canvas.restore();
    }

    /// One lane: its steps as pills with arrows between, and whether it is
    /// saved at its right end.
    #[expect(
        clippy::too_many_arguments,
        reason = "a lane is placed, filled and coloured"
    )]
    fn draw_lane(
        &mut self,
        canvas: &Canvas,
        rect: Rect,
        steps: &[Step],
        saved: &[bool],
        media: MediaType,
        viewport: Rect,
        theme: &Theme,
    ) {
        let rrect = RRect::new_rect_xy(rect, 12.0, 12.0);
        canvas.draw_rrect(rrect, &fill(card_ground(theme)));
        canvas.draw_rrect(rrect, &stroke(theme.hairline, 1.0));
        let color = media_color(theme, media);
        let cy = rect.center_y();

        // Each pill as wide as its text, all narrowed alike to fit.
        let title_font = styles::SUBHEADLINE_EMPHASIZED.font();
        let kind_font = styles::SUBHEADLINE.font();
        let natural: Vec<f32> = steps
            .iter()
            .map(|s| {
                let text = title_font
                    .measure_str(&s.title, None)
                    .0
                    .max(kind_font.measure_str(&s.kind, None).0);
                (text + 24.0).clamp(PILL_MIN_W, PILL_MAX_W)
            })
            .collect();
        let room =
            rect.width() - 28.0 - LANE_STATUS_W - ARROW_W * steps.len().saturating_sub(1) as f32;
        let total: f32 = natural.iter().sum();
        let scale = if total > room { room / total } else { 1.0 };

        canvas.save();
        canvas.clip_rrect(rrect, None, Some(true));
        let mut x = rect.left + 14.0;
        for (index, (step, w)) in steps.iter().zip(&natural).enumerate() {
            let w = (w * scale).max(PILL_MIN_W);
            if index > 0 {
                let hop_saved = saved.get(index - 1).copied().unwrap_or(false);
                draw_arrow(canvas, x - ARROW_W + 4.0, x - 4.0, cy, color, hop_saved);
            }
            let pill = Rect::from_xywh(x, cy - PILL_H / 2.0, w, PILL_H);
            let pr = RRect::new_rect_xy(pill, 8.0, 8.0);
            if step.between {
                canvas.draw_rrect(
                    pr,
                    &fill(Color::from_argb(0x1e, color.r(), color.g(), color.b())),
                );
                canvas.draw_rrect(
                    pr,
                    &stroke(Color::from_argb(0x70, color.r(), color.g(), color.b()), 1.0),
                );
            } else {
                canvas.draw_rrect(pr, &fill(theme.fill_quaternary));
                canvas.draw_rrect(pr, &stroke(theme.hairline, 1.0));
            }
            if self.view.selected_node == Some(step.node) {
                canvas.draw_rrect(pr, &stroke(theme.accent, 2.0));
            }
            let text_w = w - 20.0;
            Label::new(ellipsize(&title_font, &step.title, text_w))
                .with_style(styles::SUBHEADLINE_EMPHASIZED)
                .with_color(theme.text_primary)
                .centered_on(pill.left + 10.0, cy - 7.0)
                .render(canvas);
            Label::new(ellipsize(&kind_font, &step.kind, text_w))
                .with_style(styles::SUBHEADLINE)
                .with_color(theme.text_secondary)
                .centered_on(pill.left + 10.0, cy + 9.0)
                .render(canvas);
            if let Some(hit) = clipped(pill, viewport) {
                self.targets.push((hit, Action::SelectNode(step.node)));
            }
            x = pill.right + ARROW_W;
        }
        canvas.restore();

        // Saved or not, as the toolbar's legend draws it.
        let all_saved = saved.iter().all(|s| *s);
        let label = if all_saved {
            t!("audio-legend-saved")
        } else {
            t!("audio-legend-unsaved")
        };
        let w = kind_font.measure_str(label, None).0;
        let text_x = rect.right - 14.0 - w;
        let dot = Point::new(text_x - 10.0, cy);
        if all_saved {
            canvas.draw_circle(dot, 4.0, &fill(color));
        } else {
            canvas.draw_circle(dot, 3.5, &stroke(color, 1.8));
        }
        Label::new(label)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_on(text_x, cy)
            .render(canvas);
    }

    /// Open the Graph section on `mode`, without remembering it: for the
    /// offscreen preview.
    pub fn show_graph_mode(&mut self, mode: GraphMode) {
        self.view.mode = mode;
    }

    /// A press on the Matrix or Lanes view that hit no control: clear the
    /// selection.
    pub(super) fn press_view(&mut self) -> bool {
        self.view.selected_node = None;
        self.view.selected_wire = None;
        true
    }

    /// Scroll the Matrix or Lanes view.
    pub(super) fn scroll_view(&mut self, dx: f32, dy: f32, discrete: bool, stop: bool) -> bool {
        let views = &mut self.views;
        match self.view.mode {
            GraphMode::Matrix => {
                let down = wheel(&mut views.rows, dy, discrete, stop);
                let across = wheel(&mut views.columns, dx, discrete, stop);
                down || across
            }
            // Lanes scroll only up and down.
            GraphMode::Lanes | GraphMode::Patchbay => wheel(&mut views.lanes, dy, discrete, stop),
        }
    }

    /// Run one of the views' own actions.
    pub(super) fn run_view(&mut self, action: Action) {
        match action {
            Action::GraphMode(mode) => {
                self.view.set_mode(mode);
                self.drag = None;
            }
            Action::LinkCell(from, to) => {
                let scene = self.view.scene(&self.graph, &self.routing);
                for (output, input) in self.view.pairs_to_link(&scene, &self.graph, from, to) {
                    self.connection.link(output, input, LinkLifetime::Lingering);
                }
                // Selected as it arrives, so Save is one press away.
                self.view.selected_wire = Some((from, to));
                self.view.selected_node = None;
            }
            Action::SelectWire(from, to) => {
                self.view.selected_wire = Some((from, to));
                self.view.selected_node = None;
            }
            Action::SelectNode(node) => {
                if self.view.selected_node != Some(node) {
                    self.props_scroll.scroll_to(0.0);
                }
                self.view.selected_node = Some(node);
                self.view.selected_wire = None;
            }
            _ => {}
        }
    }
}

/// Feed one axis of a scroll to `scroll`: a notched wheel moves one step per
/// click, a touchpad stream drives momentum, and lifting the fingers throws
/// it. Returns whether to redraw.
fn wheel(scroll: &mut ScrollView, delta: f32, discrete: bool, stop: bool) -> bool {
    if stop {
        scroll.on_wheel_end();
        true
    } else if delta == 0.0 {
        false
    } else if discrete {
        scroll.on_wheel_discrete(delta)
    } else {
        scroll.on_wheel(delta)
    }
}

/// The part of `rect` inside `within`, if any.
fn clipped(rect: Rect, within: Rect) -> Option<Rect> {
    let r = Rect::from_ltrb(
        rect.left.max(within.left),
        rect.top.max(within.top),
        rect.right.min(within.right),
        rect.bottom.min(within.bottom),
    );
    (r.width() > 0.0 && r.height() > 0.0).then_some(r)
}

/// Diagonal hatching over a node's own cell, which cannot be linked.
fn draw_hatch(canvas: &Canvas, rect: Rect, theme: &Theme) {
    canvas.save();
    canvas.clip_rect(rect, None, Some(true));
    let paint = stroke(theme.fill_tertiary, 1.0);
    let mut x = rect.left - rect.height();
    while x < rect.right {
        canvas.draw_line(
            Point::new(x, rect.bottom),
            Point::new(x + rect.height(), rect.top),
            &paint,
        );
        x += HATCH;
    }
    canvas.restore();
}

/// An arrow from `x0` to `x1` at `y`; dashed when the hop is not saved.
fn draw_arrow(canvas: &Canvas, x0: f32, x1: f32, y: f32, color: Color, saved: bool) {
    let mut paint = stroke(color, 1.8);
    if !saved {
        paint.set_path_effect(otto_kit::skia::PathEffect::dash(&[4.0, 3.0], 0.0));
    }
    canvas.draw_line(Point::new(x0, y), Point::new(x1 - 4.0, y), &paint);
    let mut head = PathBuilder::new();
    head.move_to((x1 - 6.0, y - 4.0));
    head.line_to((x1, y));
    head.line_to((x1 - 6.0, y + 4.0));
    head.close();
    canvas.draw_path(&head.detach(), &fill(color));
}
