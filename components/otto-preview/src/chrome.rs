//! The window's own chrome: the titlebar with its traffic lights, and the
//! toolbar under it.
//!
//! Geometry and drawing live side by side so a press is always tested against
//! the rects that were painted. Everything is in logical points with the
//! window's top-left at the origin.

// Rust guideline compliant 2026-02-21

use otto_kit::common::Renderable;
use otto_kit::components::titlebar::{DecorationVariant, WindowControl, WindowDecoration};
use otto_kit::components::toolbar::Toolbar;
use otto_kit::icons;
use otto_kit::prelude::*;
use otto_kit::skia::{BlendMode, Contains, PaintStyle, PathBuilder, Point, RRect};
use otto_kit::typography::ellipsize;

use crate::viewer::Viewer;

/// The toolbar strip under the titlebar.
pub const TOOLBAR_H: f32 = 40.0;
/// A toolbar button, square around its icon.
const BUTTON: f32 = 28.0;
/// Between two buttons of one group.
const GAP: f32 = 2.0;
/// From the window's leading edge to the first button.
const EDGE: f32 = 10.0;
/// The page counter between the two page buttons.
const PAGE_LABEL_W: f32 = 76.0;
/// The zoom level between the zoom-out and zoom-in buttons.
const ZOOM_LABEL_W: f32 = 48.0;
/// The least room between the leading buttons and the page controls.
const GROUP_GAP: f32 = 12.0;
/// A symbolic icon's size inside a button.
const GLYPH: f32 = 16.0;

/// A toolbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    ZoomOut,
    ZoomFit,
    ZoomIn,
    PreviousPage,
    NextPage,
    /// Shows or hides the pages sidebar.
    Sidebar,
    /// Shows or hides the chat beside the document.
    Chat,
    /// Turns drawing marks on the document on or off.
    Mark,
    /// Hides or shows the marks, the person's and the agent's.
    ShowMarks,
    /// Steps the file back a version.
    Undo,
    /// Steps the file forward again.
    Redo,
}

/// Where the toolbar's buttons sit for one window width.
#[derive(Debug, Clone)]
pub struct ToolbarLayout {
    pub buttons: Vec<(Tool, Rect)>,
    /// The page counter's box, when the preview has pages.
    pub page_label: Option<Rect>,
    /// The zoom level's box, between zoom out and zoom in.
    pub zoom_label: Rect,
}

impl ToolbarLayout {
    /// The button under a window-local point.
    pub fn tool_at(&self, x: f32, y: f32) -> Option<Tool> {
        self.buttons
            .iter()
            .find(|(_, rect)| rect.contains(Point::new(x, y)))
            .map(|(tool, _)| *tool)
    }
}

/// The titlebar's height for the decoration the window wears.
pub fn titlebar_h(variant: DecorationVariant) -> f32 {
    WindowDecoration::height_for(variant)
}

/// Everything above the content: the titlebar and the toolbar.
pub fn chrome_h(variant: DecorationVariant) -> f32 {
    titlebar_h(variant) + TOOLBAR_H
}

/// The box the preview is drawn in: the whole window under the chrome.
pub fn content_rect(width: f32, height: f32, variant: DecorationVariant) -> Rect {
    let top = chrome_h(variant);
    Rect::from_ltrb(0.0, top, width.max(1.0), height.max(top + 1.0))
}

/// The titlebar the window draws, and hit-tests against.
pub fn decoration(viewer: &Viewer) -> WindowDecoration {
    let (width, _) = viewer.size;
    let variant = viewer.variant;
    let mut decoration = WindowDecoration::new(String::new(), width)
        .with_variant(variant)
        .with_active(viewer.active)
        .with_dark(viewer.dark())
        // Opaque: nothing is blurred behind this window.
        .with_blurred(false);
    decoration.controls_hovered = viewer.controls.hovered();
    decoration.pressed = viewer.controls.pressed();
    // Clear of the lights at both ends, since the title is centred.
    let room = (width - 2.0 * 90.0).max(40.0);
    let font = WindowDecoration::title_style_for(variant).font();
    decoration.title = ellipsize(&font, &viewer.name, room);
    decoration
}

/// The traffic light under a window-local point.
pub fn control_at(viewer: &Viewer, x: f32, y: f32) -> Option<WindowControl> {
    decoration(viewer).control_at(x, y)
}

/// Lay the toolbar out for a window `width` points wide.
///
/// The sidebar button leads when the preview is a document of pages, then
/// zoom out, the zoom level, zoom in and fit. The page controls sit in the
/// middle when there are pages to turn, pushed along when a narrow window
/// leaves the middle to the leading buttons.
pub fn toolbar_layout(
    width: f32,
    variant: DecorationVariant,
    paged: bool,
    sidebar: bool,
    versions: bool,
) -> ToolbarLayout {
    let top = titlebar_h(variant);
    let y = top + (TOOLBAR_H - BUTTON) / 2.0;
    let square = |x: f32| Rect::from_xywh(x, y, BUTTON, BUTTON);
    let mut buttons = Vec::with_capacity(7);
    // The chat at the trailing edge, and the pen that points things out to
    // it beside it.
    buttons.push((Tool::Chat, square(width - EDGE - BUTTON)));
    let pen = width - EDGE - 2.0 * BUTTON - GAP * 4.0;
    buttons.push((Tool::Mark, square(pen)));
    let eye = pen - BUTTON - GAP;
    buttons.push((Tool::ShowMarks, square(eye)));
    // Back and forward through the file's versions, once there are any.
    if versions {
        let redo = eye - BUTTON - GAP * 4.0;
        buttons.push((Tool::Redo, square(redo)));
        buttons.push((Tool::Undo, square(redo - BUTTON - GAP)));
    }

    let mut x = EDGE;
    if sidebar {
        buttons.push((Tool::Sidebar, square(x)));
        // Its own group, apart from the zoom.
        x += BUTTON + GAP * 4.0;
    }
    buttons.push((Tool::ZoomOut, square(x)));
    x += BUTTON;
    let zoom_label = Rect::from_xywh(x, y, ZOOM_LABEL_W, BUTTON);
    x += ZOOM_LABEL_W;
    buttons.push((Tool::ZoomIn, square(x)));
    x += BUTTON + GAP * 4.0;
    buttons.push((Tool::ZoomFit, square(x)));
    x += BUTTON;

    let page_label = paged.then(|| {
        let group = BUTTON * 2.0 + PAGE_LABEL_W;
        let left = ((width - group) / 2.0).max(x + GROUP_GAP);
        buttons.push((Tool::PreviousPage, square(left)));
        buttons.push((Tool::NextPage, square(left + BUTTON + PAGE_LABEL_W)));
        Rect::from_xywh(left + BUTTON, y, PAGE_LABEL_W, BUTTON)
    });

    ToolbarLayout {
        buttons,
        page_label,
        zoom_label,
    }
}

/// Paint the titlebar and the toolbar over the top of the window.
pub fn draw(canvas: &Canvas, viewer: &Viewer, theme: &Theme) {
    let (width, _) = viewer.size;
    let decoration = decoration(viewer);

    crate::sidebar::draw(canvas, viewer, theme);

    // The toolbar first, so the titlebar's bottom hairline lands over it.
    let top = titlebar_h(viewer.variant);
    Toolbar::new()
        .at(0.0, top)
        .with_width(width)
        .with_height(TOOLBAR_H)
        .with_padding(0.0)
        .with_background(decoration.material_tint(false))
        .with_border_bottom(theme.fill_tertiary)
        .render(canvas);
    decoration.draw(canvas);

    let paged = viewer.page_status();
    let layout = viewer.toolbar();
    for (tool, rect) in &layout.buttons {
        let enabled = viewer.tool_enabled(*tool);
        let hovered = enabled && viewer.hovered_tool == Some(*tool);
        let pressed = hovered && viewer.pressed_tool == Some(*tool);
        // The sidebar button is a toggle: it stays down while the sidebar
        // shows.
        let pressed = pressed
            || (*tool == Tool::Sidebar && viewer.sidebar_open())
            || (*tool == Tool::Chat && viewer.chat_open)
            || (*tool == Tool::Mark && viewer.marking)
            || (*tool == Tool::ShowMarks && viewer.marks_hidden);
        draw_icon_button(canvas, theme, *rect, *tool, enabled, hovered, pressed);
    }

    if let Some(percent) = viewer.zoom_percent() {
        let rect = layout.zoom_label;
        Label::new(format!("{percent}%"))
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .centered_at(rect.center_x(), rect.center_y())
            .render(canvas);
    }

    if let (Some(rect), Some((page, pages))) = (layout.page_label, paged) {
        Label::new(otto_kit::t_owned!(
            "peek-page-of",
            page = page.to_string(),
            pages = pages.to_string()
        ))
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_at(rect.center_x(), rect.center_y())
        .render(canvas);
    }
}

/// The ground under a button the pointer is over or holding.
fn button_ground(hovered: bool, pressed: bool, theme: &Theme) -> Option<Color> {
    match (hovered, pressed) {
        (_, true) => Some(theme.fill_tertiary),
        (true, false) => Some(theme.fill_quaternary),
        _ => None,
    }
}

fn draw_icon_button(
    canvas: &Canvas,
    theme: &Theme,
    rect: Rect,
    tool: Tool,
    enabled: bool,
    hovered: bool,
    pressed: bool,
) {
    if let Some(ground) = button_ground(hovered, pressed, theme) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(ground);
        canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0, 6.0), &paint);
    }
    let color = match (enabled, hovered) {
        (false, _) => theme.text_tertiary,
        (true, true) => theme.text_primary,
        (true, false) => theme.text_secondary,
    };
    let dst = Rect::from_xywh(
        rect.center_x() - GLYPH / 2.0,
        rect.center_y() - GLYPH / 2.0,
        GLYPH,
        GLYPH,
    );
    if tool == Tool::Sidebar {
        // Drawn rather than looked up: themes disagree on what the sidebar
        // icon is, and some put an arrow in it.
        draw_sidebar_glyph(canvas, dst, color);
        return;
    }
    if tool == Tool::Chat {
        // Drawn too: few themes have a chat bubble, and fewer agree on one.
        draw_chat_glyph(canvas, dst, color);
        return;
    }
    if tool == Tool::Mark {
        draw_pen_glyph(canvas, dst, color);
        return;
    }
    if tool == Tool::ShowMarks {
        // Crossed out while the marks are hidden, which is when it is down.
        draw_eye_glyph(canvas, dst, color, pressed);
        return;
    }
    match icons::cached_icon_chain(icon_names(tool), GLYPH as i32) {
        Some(image) => {
            // Symbolic art recoloured to the text tone, as the rest of the
            // chrome does with its glyphs.
            let mut tint = Paint::default();
            tint.set_color_filter(otto_kit::skia::color_filters::blend(
                color,
                BlendMode::SrcIn,
            ));
            canvas.draw_image_rect(&image, None, dst, &tint);
        }
        None => draw_fallback_glyph(canvas, dst, tool, color),
    }
}

/// The themed symbolic icons for a tool, most specific first.
fn icon_names(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::ZoomOut => &["zoom-out-symbolic"],
        Tool::ZoomFit => &["zoom-fit-best-symbolic", "zoom-original-symbolic"],
        Tool::ZoomIn => &["zoom-in-symbolic"],
        Tool::PreviousPage => &["go-up-symbolic", "pan-up-symbolic"],
        Tool::NextPage => &["go-down-symbolic", "pan-down-symbolic"],
        Tool::Undo => &["edit-undo-symbolic"],
        Tool::Redo => &["edit-redo-symbolic"],
        // Never looked up; see `draw_sidebar_glyph` and `draw_chat_glyph`.
        Tool::Sidebar | Tool::Chat | Tool::Mark | Tool::ShowMarks => &[],
    }
}

/// A plain drawn glyph for a theme with no symbolic art for the tool.
fn draw_fallback_glyph(canvas: &Canvas, dst: Rect, tool: Tool, color: Color) {
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_stroke_cap(otto_kit::skia::PaintCap::Round);
    stroke.set_color(color);
    let r = dst.with_inset((2.0, 2.0));
    let (cx, cy) = (r.center_x(), r.center_y());
    let mut path = PathBuilder::new();
    match tool {
        Tool::ZoomOut | Tool::ZoomIn => {
            path.move_to((r.left, cy));
            path.line_to((r.right, cy));
            if tool == Tool::ZoomIn {
                path.move_to((cx, r.top));
                path.line_to((cx, r.bottom));
            }
        }
        Tool::ZoomFit => {
            // Four corner brackets: the picture's frame.
            let arm = r.width() * 0.3;
            for (x, y, dx, dy) in [
                (r.left, r.top, 1.0, 1.0),
                (r.right, r.top, -1.0, 1.0),
                (r.left, r.bottom, 1.0, -1.0),
                (r.right, r.bottom, -1.0, -1.0),
            ] {
                path.move_to((x + dx * arm, y));
                path.line_to((x, y));
                path.line_to((x, y + dy * arm));
            }
        }
        Tool::PreviousPage | Tool::NextPage => {
            let dy = if tool == Tool::PreviousPage {
                -1.0
            } else {
                1.0
            };
            let half = r.width() * 0.35;
            path.move_to((cx - half, cy - dy * half / 2.0));
            path.line_to((cx, cy + dy * half / 2.0));
            path.line_to((cx + half, cy - dy * half / 2.0));
        }
        Tool::Sidebar => draw_sidebar_glyph(canvas, dst, color),
        Tool::Chat => draw_chat_glyph(canvas, dst, color),
        Tool::Mark => draw_pen_glyph(canvas, dst, color),
        Tool::ShowMarks => draw_eye_glyph(canvas, dst, color, false),
        Tool::Undo | Tool::Redo => {
            // A hooked arrow, pointing back for undo.
            let dx = if tool == Tool::Undo { 1.0 } else { -1.0 };
            let (start, end) = if dx > 0.0 {
                (r.left, r.right)
            } else {
                (r.right, r.left)
            };
            path.move_to((start + dx * 3.0, cy - 4.0));
            path.line_to((start, cy));
            path.line_to((start + dx * 3.0, cy + 4.0));
            path.move_to((start, cy));
            path.line_to((end - dx * 3.0, cy));
            path.quad_to((end, cy), (end, cy + 3.0));
        }
    }
    canvas.draw_path(&path.detach(), &stroke);
}

/// The sidebar button's icon: a rounded window with a shaded pane down its
/// leading edge, the shape every desktop draws for "sidebar".
fn draw_sidebar_glyph(canvas: &Canvas, dst: Rect, color: Color) {
    let frame = Rect::from_ltrb(
        dst.left + 0.75,
        dst.top + 2.25,
        dst.right - 0.75,
        dst.bottom - 2.25,
    );
    let radius = 2.5;
    let divider = frame.left + frame.width() * 0.36;

    let mut pane = Paint::default();
    pane.set_anti_alias(true);
    pane.set_color(color.with_a((color.a() as f32 * 0.35) as u8));
    canvas.save();
    canvas.clip_rrect(RRect::new_rect_xy(frame, radius, radius), None, true);
    canvas.draw_rect(
        Rect::from_ltrb(frame.left, frame.top, divider, frame.bottom),
        &pane,
    );
    canvas.restore();

    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_color(color);
    canvas.draw_rrect(RRect::new_rect_xy(frame, radius, radius), &stroke);
    canvas.draw_line((divider, frame.top), (divider, frame.bottom), &stroke);
}

/// A pen, nib down to the lower leading corner, drawing a short line.
fn draw_pen_glyph(canvas: &Canvas, dst: Rect, color: Color) {
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_stroke_join(otto_kit::skia::PaintJoin::Round);
    stroke.set_stroke_cap(otto_kit::skia::PaintCap::Round);
    stroke.set_color(color);
    let (l, t, r, b) = (
        dst.left + 1.5,
        dst.top + 1.5,
        dst.right - 1.5,
        dst.bottom - 1.5,
    );
    let mut pen = PathBuilder::new();
    // The barrel, a slanted box from the top trailing corner to the nib.
    pen.move_to((r - 3.0, t));
    pen.line_to((r, t + 3.0));
    pen.line_to((l + 4.0, b - 1.0));
    pen.line_to((l, b));
    pen.line_to((l + 1.0, b - 4.0));
    pen.close();
    canvas.draw_path(&pen.detach(), &stroke);
}

/// An eye, for the marks shown over the document; struck through when they
/// are hidden.
fn draw_eye_glyph(canvas: &Canvas, dst: Rect, color: Color, struck: bool) {
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_stroke_join(otto_kit::skia::PaintJoin::Round);
    stroke.set_stroke_cap(otto_kit::skia::PaintCap::Round);
    stroke.set_color(color);
    let (l, r, cy) = (dst.left + 1.0, dst.right - 1.0, dst.center_y());
    let cx = dst.center_x();
    let lid = dst.height() * 0.42;
    let mut eye = PathBuilder::new();
    eye.move_to((l, cy));
    eye.quad_to((cx, cy - lid), (r, cy));
    eye.quad_to((cx, cy + lid), (l, cy));
    eye.close();
    canvas.draw_path(&eye.detach(), &stroke);
    canvas.draw_circle((cx, cy), 2.25, &stroke);
    if struck {
        canvas.draw_line(
            (dst.left + 2.5, dst.bottom - 2.0),
            (dst.right - 2.5, dst.top + 2.0),
            &stroke,
        );
    }
}

/// A speech bubble: a rounded box with a tail at its lower leading corner.
fn draw_chat_glyph(canvas: &Canvas, dst: Rect, color: Color) {
    let bubble = Rect::from_ltrb(
        dst.left + 1.0,
        dst.top + 2.0,
        dst.right - 1.0,
        dst.bottom - 4.5,
    );
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_stroke_join(otto_kit::skia::PaintJoin::Round);
    stroke.set_color(color);
    canvas.draw_rrect(RRect::new_rect_xy(bubble, 4.0, 4.0), &stroke);
    let mut tail = PathBuilder::new();
    tail.move_to((bubble.left + 3.5, bubble.bottom));
    tail.line_to((bubble.left + 3.0, dst.bottom - 1.0));
    tail.line_to((bubble.left + 7.5, bubble.bottom));
    canvas.draw_path(&tail.detach(), &stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(layout: &ToolbarLayout, wanted: Tool) -> Rect {
        layout
            .buttons
            .iter()
            .find(|(tool, _)| *tool == wanted)
            .map(|(_, rect)| *rect)
            .unwrap()
    }

    #[test]
    fn the_page_controls_clear_the_zoom_group_in_the_narrowest_window() {
        let layout = toolbar_layout(
            crate::app::MIN_W,
            DecorationVariant::default(),
            true,
            true,
            true,
        );
        let leading = button(&layout, Tool::ZoomFit).right;
        let previous = button(&layout, Tool::PreviousPage).left;
        assert!(previous > leading, "{previous} <= {leading}");
    }

    #[test]
    fn the_zoom_level_sits_between_zoom_out_and_zoom_in() {
        let layout = toolbar_layout(900.0, DecorationVariant::default(), false, false, false);
        assert!(button(&layout, Tool::ZoomOut).right <= layout.zoom_label.left);
        assert!(layout.zoom_label.right <= button(&layout, Tool::ZoomIn).left);
    }
}
