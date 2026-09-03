//! Pure drawing half of the open picker: mode switcher plus the three modes.
//!
//! Still no `AppContext`/`AppRunner`/wayland-client dependency — [`draw`]
//! takes a bare canvas and a `rect`, the same as
//! [`well`](super::well). [`super::popup`] is the client half that hosts
//! this inside an actual popup surface and turns pointer events into calls
//! back into the geometry helpers below, so the popup and this module can
//! never disagree about where anything is.
//!
//! Layout is fixed-size regardless of which [`Mode`] is selected —
//! [`content_height`] returns the tallest of the three so switching mode
//! never has to resize the popup surface underneath the pointer.
//!
//! The saturation/value square is the one thing here worth double-checking
//! visually: it has to fade from white (s=0) at the left to the pure hue at
//! the right, and from full brightness at the top to black at the bottom,
//! for whatever hue is currently selected. Composited as two gradients
//! layered over a hue-coloured fill — see [`draw_hsv`].

use skia_safe::{
    gradient_shader, BlendMode, Canvas, ClipOp, Color, Paint, PaintStyle, Point, RRect, Rect,
    TileMode,
};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::styles;

use super::hsv::{hsv_to_rgb, rgb_to_hsv};

/// Width the visual design is tuned at. Fixed, unlike [`well`](super::well)
/// — a picker panel has fixed-size interior controls (the HSV square,
/// switcher segments), so there is no caller-supplied content to size
/// around.
pub const WIDTH: f32 = 232.0;

const PADDING: f32 = 12.0;
const SWITCHER_HEIGHT: f32 = 26.0;
const SECTION_GAP: f32 = 12.0;

/// Width of everything below the panel's padding — the switcher, the
/// swatch grid and the HSV square plus hue strip all span exactly this, so
/// every mode lines up on both edges.
const CONTENT_WIDTH: f32 = WIDTH - PADDING * 2.0;

const SWATCH_SIZE: f32 = 28.0;
const SWATCH_COLS: usize = 6;
/// Derived, not tuned: whatever is left over after the swatches themselves,
/// split between the gaps. Hardcoding it let the grid run past the right
/// padding — see `swatch_grid_fits_the_content_width`.
const SWATCH_GAP: f32 =
    (CONTENT_WIDTH - SWATCH_SIZE * SWATCH_COLS as f32) / (SWATCH_COLS as f32 - 1.0);

const HUE_STRIP_WIDTH: f32 = 18.0;
const HUE_STRIP_GAP: f32 = 10.0;
/// The hue indicator is drawn 2px proud of the strip on each side, so the
/// strip stops that far short of the content edge to keep the whole control
/// inside the padding.
const HUE_INDICATOR_OVERHANG: f32 = 2.0;
const SQUARE_SIZE: f32 = CONTENT_WIDTH - HUE_STRIP_GAP - HUE_STRIP_WIDTH - HUE_INDICATOR_OVERHANG;
const PREVIEW_ROW_HEIGHT: f32 = 24.0;

const FIELD_ROW_HEIGHT: f32 = 24.0;
const FIELD_ROW_GAP: f32 = 8.0;

/// The three ways to pick a colour, switched with the segmented control at
/// the top of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Swatches,
    Hsv,
    Hex,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Swatches, Mode::Hsv, Mode::Hex];

    fn label(self) -> &'static str {
        match self {
            Mode::Swatches => "Swatches",
            Mode::Hsv => "HSV",
            Mode::Hex => "Hex",
        }
    }

    fn index(self) -> usize {
        match self {
            Mode::Swatches => 0,
            Mode::Hsv => 1,
            Mode::Hex => 2,
        }
    }
}

/// A caller-supplied preset for [`Mode::Swatches`] — the mode Otto's
/// `accent_color` setting actually drives, since the compositor accepts a
/// fixed named set rather than an arbitrary hex value.
#[derive(Debug, Clone)]
pub struct Swatch {
    pub name: String,
    pub color: Color,
}

impl Swatch {
    pub fn new(name: impl Into<String>, color: Color) -> Self {
        Self {
            name: name.into(),
            color,
        }
    }
}

/// The hex/RGB mode's four fields, for hit-testing which one a click landed
/// on. No inline text editing yet — see the module docs on
/// [`super::popup`] for why, and what a click on one of these does instead
/// (selects it, for a future numeric-entry pass to build on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexField {
    Hex,
    R,
    G,
    B,
}

/// Height of the content area below the switcher for `mode`, given
/// `swatch_count` presets (only relevant to [`Mode::Swatches`]).
pub fn content_height(mode: Mode, swatch_count: usize) -> f32 {
    match mode {
        Mode::Swatches => {
            let rows = swatch_count.div_ceil(SWATCH_COLS).max(1);
            rows as f32 * SWATCH_SIZE + (rows.saturating_sub(1)) as f32 * SWATCH_GAP
        }
        Mode::Hsv => SQUARE_SIZE + SECTION_GAP + PREVIEW_ROW_HEIGHT,
        Mode::Hex => 4.0 * FIELD_ROW_HEIGHT + 3.0 * FIELD_ROW_GAP,
    }
}

/// The tallest content area across all three modes, for `swatch_count`
/// presets. The popup sizes itself to `PADDING*2 + SWITCHER_HEIGHT +
/// SECTION_GAP + max_content_height(..)` so switching mode is a pure
/// redraw.
pub fn max_content_height(swatch_count: usize) -> f32 {
    Mode::ALL
        .iter()
        .map(|&m| content_height(m, swatch_count))
        .fold(0.0_f32, f32::max)
}

/// Full panel size for `swatch_count` presets — pass to the popup's XDG
/// positioner.
pub fn panel_size(swatch_count: usize) -> (f32, f32) {
    (
        WIDTH,
        PADDING * 2.0 + SWITCHER_HEIGHT + SECTION_GAP + max_content_height(swatch_count),
    )
}

fn switcher_rect(rect: Rect) -> Rect {
    Rect::from_xywh(
        rect.left + PADDING,
        rect.top + PADDING,
        rect.width() - PADDING * 2.0,
        SWITCHER_HEIGHT,
    )
}

fn content_rect(rect: Rect) -> Rect {
    let switcher = switcher_rect(rect);
    Rect::from_xywh(
        switcher.left,
        switcher.bottom + SECTION_GAP,
        switcher.width(),
        rect.bottom - (switcher.bottom + SECTION_GAP) - PADDING,
    )
}

/// Which switcher segment, if any, `(x, y)` is over.
pub fn mode_at(rect: Rect, x: f32, y: f32) -> Option<Mode> {
    let s = switcher_rect(rect);
    if x < s.left || x > s.right || y < s.top || y > s.bottom {
        return None;
    }
    let seg_w = s.width() / Mode::ALL.len() as f32;
    let idx = (((x - s.left) / seg_w) as usize).min(Mode::ALL.len() - 1);
    Some(Mode::ALL[idx])
}

/// Rect of swatch `index` within the swatches grid.
pub fn swatch_rect(rect: Rect, index: usize) -> Rect {
    let c = content_rect(rect);
    let col = (index % SWATCH_COLS) as f32;
    let row = (index / SWATCH_COLS) as f32;
    Rect::from_xywh(
        c.left + col * (SWATCH_SIZE + SWATCH_GAP),
        c.top + row * (SWATCH_SIZE + SWATCH_GAP),
        SWATCH_SIZE,
        SWATCH_SIZE,
    )
}

/// Which swatch, if any, `(x, y)` is over — `count` must match the slice
/// [`draw`] was given.
pub fn swatch_at(rect: Rect, count: usize, x: f32, y: f32) -> Option<usize> {
    (0..count).find(|&i| {
        let r = swatch_rect(rect, i);
        x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
    })
}

/// The saturation/value square's rect, in [`Mode::Hsv`].
pub fn hsv_square_rect(rect: Rect) -> Rect {
    let c = content_rect(rect);
    Rect::from_xywh(c.left, c.top, SQUARE_SIZE, SQUARE_SIZE)
}

/// The hue strip's rect, in [`Mode::Hsv`].
pub fn hsv_hue_rect(rect: Rect) -> Rect {
    let c = content_rect(rect);
    Rect::from_xywh(
        c.left + SQUARE_SIZE + HUE_STRIP_GAP,
        c.top,
        HUE_STRIP_WIDTH,
        SQUARE_SIZE,
    )
}

/// Map a point inside (or clamped to) the SV square to `(saturation, value)`
/// in `0.0..=1.0`. Shared by dragging and click-to-jump so both land on the
/// same value for the same pointer position.
pub fn sv_at(square: Rect, x: f32, y: f32) -> (f32, f32) {
    let s = ((x - square.left) / square.width()).clamp(0.0, 1.0);
    let v = (1.0 - (y - square.top) / square.height()).clamp(0.0, 1.0);
    (s, v)
}

/// Map a point inside (or clamped to) the hue strip to a hue in
/// `0.0..360.0`.
pub fn hue_at(strip: Rect, y: f32) -> f32 {
    (((y - strip.top) / strip.height()).clamp(0.0, 1.0)) * 360.0
}

fn hex_row_rect(rect: Rect) -> Rect {
    let c = content_rect(rect);
    Rect::from_xywh(c.left, c.top, c.width(), FIELD_ROW_HEIGHT)
}

fn rgb_row_rect(rect: Rect, i: usize) -> Rect {
    let c = content_rect(rect);
    let top = c.top + (FIELD_ROW_HEIGHT + FIELD_ROW_GAP) * (i as f32 + 1.0);
    Rect::from_xywh(c.left, top, c.width(), FIELD_ROW_HEIGHT)
}

/// Which of the hex/RGB rows, if any, `(x, y)` is over, in [`Mode::Hex`].
pub fn hex_field_at(rect: Rect, x: f32, y: f32) -> Option<HexField> {
    let hit = |r: Rect| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
    if hit(hex_row_rect(rect)) {
        return Some(HexField::Hex);
    }
    for (i, field) in [HexField::R, HexField::G, HexField::B]
        .into_iter()
        .enumerate()
    {
        if hit(rgb_row_rect(rect, i)) {
            return Some(field);
        }
    }
    None
}

/// Corner radius of the panel's background. The popup surface is rounded to
/// the same radius compositor-side (see `ColorPickerPopup::apply_surface_effects`),
/// so the two must agree or one clips the other.
pub const CORNER_RADIUS: f32 = 10.0;

/// Draw the whole panel into `rect` (sized via [`panel_size`]): switcher,
/// then whichever mode is selected.
///
/// `color` is the single source of truth for every mode — HSV and hex/RGB
/// are both derived from it, never tracked separately, so there is nothing
/// for them to drift out of sync with.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    mode: Mode,
    color: Color,
    swatches: &[Swatch],
    selected_swatch: Option<usize>,
    selected_field: Option<HexField>,
    theme: &Theme,
) {
    let body = RRect::new_rect_xy(rect, CORNER_RADIUS, CORNER_RADIUS);
    canvas.draw_rrect(body, &fill(theme.material_popup));
    // The same hairline a menu and a window carry: the panel floats over
    // arbitrary content and needs its shape closed against it.
    canvas.draw_rrect(body, &stroke(theme.hairline(), Theme::HAIRLINE_WIDTH));

    draw_switcher(canvas, rect, mode, theme);

    match mode {
        Mode::Swatches => draw_swatches(canvas, rect, swatches, selected_swatch, theme),
        Mode::Hsv => draw_hsv(canvas, rect, color, theme),
        Mode::Hex => draw_hex(canvas, rect, color, selected_field, theme),
    }
}

fn draw_switcher(canvas: &Canvas, rect: Rect, mode: Mode, theme: &Theme) {
    let s = switcher_rect(rect);
    let track = RRect::new_rect_xy(s, SWITCHER_HEIGHT / 2.0, SWITCHER_HEIGHT / 2.0);
    canvas.draw_rrect(track, &fill(theme.fill_tertiary));

    let seg_w = s.width() / Mode::ALL.len() as f32;
    let selected = Rect::from_xywh(
        s.left + seg_w * mode.index() as f32,
        s.top,
        seg_w,
        s.height(),
    );
    canvas.draw_rrect(
        RRect::new_rect_xy(selected, SWITCHER_HEIGHT / 2.0, SWITCHER_HEIGHT / 2.0),
        &fill(theme.material_highlight),
    );

    for m in Mode::ALL {
        let seg = Rect::from_xywh(s.left + seg_w * m.index() as f32, s.top, seg_w, s.height());
        let color = if m == mode {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        // `centered_on` only centres vertically (see its doc comment); the
        // horizontal centre has to come from the label's own measured
        // width, the same way `well::draw`'s hex label does not need to
        // but a segmented control's caption does.
        let text_width = styles::CAPTION_1.font().measure_str(m.label(), None).0;
        Label::new(m.label())
            .with_style(styles::CAPTION_1)
            .with_color(color)
            .centered_on(seg.center_x() - text_width / 2.0, seg.center_y())
            .render(canvas);
    }
}

fn draw_swatches(
    canvas: &Canvas,
    rect: Rect,
    swatches: &[Swatch],
    selected: Option<usize>,
    theme: &Theme,
) {
    for (i, swatch) in swatches.iter().enumerate() {
        let r = swatch_rect(rect, i);
        let rr = RRect::new_rect_xy(r, 6.0, 6.0);
        canvas.draw_rrect(rr, &fill(swatch.color));

        let is_selected = selected == Some(i);
        let (border_color, border_width) = if is_selected {
            (theme.accent, 2.0)
        } else {
            (theme.fill_primary, 1.0)
        };
        canvas.draw_rrect(rr, &stroke(border_color, border_width));

        if is_selected {
            draw_check(canvas, r.center(), contrasting_mark(swatch.color));
        }
    }
}

fn draw_hsv(canvas: &Canvas, rect: Rect, color: Color, theme: &Theme) {
    let (h, s, v) = rgb_to_hsv(color);
    let square = hsv_square_rect(rect);
    let hue_color = hsv_to_rgb(h, 1.0, 1.0);

    // Base fill is the pure hue; a left-to-right white-to-transparent
    // gradient handles saturation, and a top-to-bottom transparent-to-black
    // gradient handles value. Layering two gradients over a flat fill keeps
    // this a couple of `draw_rect` calls instead of a per-pixel shader.
    canvas.draw_rect(square, &fill(hue_color));

    if let Some(shader) = gradient_shader::linear(
        (
            Point::new(square.left, square.top),
            Point::new(square.right, square.top),
        ),
        &[Color::WHITE, Color::from_argb(0, 0xFF, 0xFF, 0xFF)][..],
        None,
        TileMode::Clamp,
        None,
        None,
    ) {
        let mut paint = Paint::default();
        paint.set_shader(shader);
        canvas.draw_rect(square, &paint);
    }

    if let Some(shader) = gradient_shader::linear(
        (
            Point::new(square.left, square.top),
            Point::new(square.left, square.bottom),
        ),
        &[Color::from_argb(0, 0, 0, 0), Color::BLACK][..],
        None,
        TileMode::Clamp,
        None,
        None,
    ) {
        let mut paint = Paint::default();
        paint.set_shader(shader);
        canvas.draw_rect(square, &paint);
    }

    canvas.draw_rect(square, &stroke(theme.fill_primary, 1.0));

    // Selection cursor: a ring in the current saturation/value position.
    // White-on-dark and dark-on-light both need to stay visible, so it's
    // drawn with both a white and a black ring rather than one colour.
    let cursor = Point::new(
        square.left + s * square.width(),
        square.top + (1.0 - v) * square.height(),
    );
    canvas.draw_circle(cursor, 6.0, &stroke(Color::WHITE, 2.0));
    canvas.draw_circle(cursor, 6.0, &stroke(Color::from_argb(0x80, 0, 0, 0), 1.0));

    // Hue strip: the full spectrum top (0deg) to bottom (360deg).
    let strip = hsv_hue_rect(rect);
    let stops: Vec<Color> = (0..=6)
        .map(|i| hsv_to_rgb(i as f32 * 60.0, 1.0, 1.0))
        .collect();
    if let Some(shader) = gradient_shader::linear(
        (
            Point::new(strip.left, strip.top),
            Point::new(strip.left, strip.bottom),
        ),
        &stops[..],
        None,
        TileMode::Clamp,
        None,
        None,
    ) {
        let mut paint = Paint::default();
        paint.set_shader(shader);
        canvas.draw_rrect(RRect::new_rect_xy(strip, 4.0, 4.0), &paint);
    }
    canvas.draw_rrect(
        RRect::new_rect_xy(strip, 4.0, 4.0),
        &stroke(theme.fill_primary, 1.0),
    );

    let indicator_y = strip.top + (h / 360.0) * strip.height();
    let indicator = Rect::from_xywh(
        strip.left - HUE_INDICATOR_OVERHANG,
        indicator_y - 2.0,
        strip.width() + HUE_INDICATOR_OVERHANG * 2.0,
        4.0,
    );
    canvas.draw_rrect(
        RRect::new_rect_xy(indicator, 2.0, 2.0),
        &stroke(Color::WHITE, 2.0),
    );
    canvas.draw_rrect(
        RRect::new_rect_xy(indicator, 2.0, 2.0),
        &stroke(Color::from_argb(0x80, 0, 0, 0), 1.0),
    );

    // Preview row: swatch plus hex, the same shape as the closed well, so
    // the picked colour is legible without switching to Hex mode.
    let preview_y = square.bottom + SECTION_GAP + PREVIEW_ROW_HEIGHT / 2.0;
    let preview = Rect::from_xywh(square.left, preview_y - 11.0, 22.0, 22.0);
    canvas.draw_rrect(RRect::new_rect_xy(preview, 5.0, 5.0), &fill(color));
    canvas.draw_rrect(
        RRect::new_rect_xy(preview, 5.0, 5.0),
        &stroke(theme.fill_primary, 1.0),
    );
    Label::new(super::well::hex_string(color))
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_secondary)
        .centered_on(preview.right + 10.0, preview_y)
        .render(canvas);
}

fn draw_hex(canvas: &Canvas, rect: Rect, color: Color, selected: Option<HexField>, theme: &Theme) {
    draw_field_row(
        canvas,
        hex_row_rect(rect),
        "Hex",
        &super::well::hex_string(color),
        selected == Some(HexField::Hex),
        theme,
    );

    let rows = [
        (HexField::R, "R", color.r()),
        (HexField::G, "G", color.g()),
        (HexField::B, "B", color.b()),
    ];
    for (i, (field, label, component)) in rows.into_iter().enumerate() {
        draw_field_row(
            canvas,
            rgb_row_rect(rect, i),
            label,
            &component.to_string(),
            selected == Some(field),
            theme,
        );
    }
}

fn draw_field_row(
    canvas: &Canvas,
    row: Rect,
    label: &str,
    value: &str,
    selected: bool,
    theme: &Theme,
) {
    Label::new(label)
        .with_style(styles::FOOTNOTE)
        .with_color(theme.text_tertiary)
        .centered_on(row.left, row.center_y())
        .render(canvas);

    let field = Rect::from_xywh(row.left + 24.0, row.top, row.width() - 24.0, row.height());
    let rr = RRect::new_rect_xy(field, 6.0, 6.0);
    canvas.draw_rrect(rr, &fill(theme.fill_quaternary));
    let (border_color, border_width) = if selected {
        (theme.accent, 1.5)
    } else {
        (theme.fill_secondary, 1.0)
    };
    canvas.draw_rrect(rr, &stroke(border_color, border_width));

    canvas.save();
    canvas.clip_rrect(rr, ClipOp::Intersect, true);
    Label::new(value)
        .with_style(styles::SUBHEADLINE)
        .with_color(theme.text_primary)
        .centered_on(field.left + 9.0, field.center_y())
        .render(canvas);
    canvas.restore();
}

fn draw_check(canvas: &Canvas, center: Point, color: Color) {
    let mut paint = stroke(color, 2.0);
    paint.set_stroke_cap(skia_safe::paint::Cap::Round);
    paint.set_stroke_join(skia_safe::paint::Join::Round);
    let mut builder = skia_safe::PathBuilder::new();
    builder.move_to(Point::new(center.x - 5.0, center.y));
    builder.line_to(Point::new(center.x - 1.5, center.y + 3.5));
    builder.line_to(Point::new(center.x + 5.0, center.y - 4.0));
    canvas.draw_path(&builder.detach(), &paint);
}

/// Black or white, whichever reads against `bg` — used for the swatch
/// selection checkmark, which has to sit on top of an arbitrary preset
/// colour.
fn contrasting_mark(bg: Color) -> Color {
    let luminance = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luminance > 140.0 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint.set_blend_mode(BlendMode::SrcOver);
    paint
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(count: usize) -> Rect {
        let (w, h) = panel_size(count);
        Rect::from_xywh(0.0, 0.0, w, h)
    }

    fn presets() -> Vec<Swatch> {
        vec![
            Swatch::new("Blue", Color::from_rgb(0x0A, 0x84, 0xFF)),
            Swatch::new("Purple", Color::from_rgb(0xBF, 0x5A, 0xF2)),
            Swatch::new("Pink", Color::from_rgb(0xFF, 0x2D, 0x55)),
            Swatch::new("Red", Color::from_rgb(0xFF, 0x3B, 0x30)),
            Swatch::new("Orange", Color::from_rgb(0xFF, 0x95, 0x00)),
            Swatch::new("Yellow", Color::from_rgb(0xFF, 0xCC, 0x00)),
            Swatch::new("Green", Color::from_rgb(0x34, 0xC7, 0x59)),
        ]
    }

    #[test]
    fn mode_at_covers_all_three_segments() {
        let r = rect(presets().len());
        let s = switcher_rect(r);
        assert_eq!(mode_at(r, s.left + 1.0, s.center_y()), Some(Mode::Swatches));
        assert_eq!(mode_at(r, s.center_x(), s.center_y()), Some(Mode::Hsv));
        assert_eq!(mode_at(r, s.right - 1.0, s.center_y()), Some(Mode::Hex));
        assert_eq!(mode_at(r, s.left - 5.0, s.center_y()), None);
    }

    #[test]
    fn swatch_grid_wraps_at_the_column_count() {
        let r = rect(presets().len());
        let first_row_last = swatch_rect(r, SWATCH_COLS - 1);
        let second_row_first = swatch_rect(r, SWATCH_COLS);
        assert!(second_row_first.top > first_row_last.top);
        assert_eq!(second_row_first.left, swatch_rect(r, 0).left);
    }

    #[test]
    fn swatch_grid_fits_the_content_width() {
        let r = rect(presets().len());
        let c = content_rect(r);
        let last_col = swatch_rect(r, SWATCH_COLS - 1);
        assert_eq!(swatch_rect(r, 0).left, c.left);
        assert!(
            last_col.right <= c.right + f32::EPSILON,
            "swatch grid runs {}px past the content edge",
            last_col.right - c.right
        );
    }

    #[test]
    fn hsv_controls_fit_the_content_width() {
        let r = rect(presets().len());
        let c = content_rect(r);
        assert_eq!(hsv_square_rect(r).left, c.left);
        // The hue indicator, not the strip, is the rightmost thing drawn.
        assert!(hsv_hue_rect(r).right + HUE_INDICATOR_OVERHANG <= c.right + f32::EPSILON);
    }

    #[test]
    fn swatch_at_matches_swatch_rect() {
        let r = rect(presets().len());
        let target = swatch_rect(r, 2);
        assert_eq!(
            swatch_at(r, presets().len(), target.center_x(), target.center_y()),
            Some(2)
        );
        assert_eq!(swatch_at(r, presets().len(), r.right + 50.0, r.top), None);
    }

    #[test]
    fn sv_at_the_square_corners() {
        let r = rect(presets().len());
        let square = hsv_square_rect(r);
        assert_eq!(sv_at(square, square.left, square.top), (0.0, 1.0));
        assert_eq!(sv_at(square, square.right, square.top), (1.0, 1.0));
        assert_eq!(sv_at(square, square.left, square.bottom), (0.0, 0.0));
        assert_eq!(sv_at(square, square.right, square.bottom), (1.0, 0.0));
    }

    #[test]
    fn sv_at_clamps_outside_the_square() {
        let r = rect(presets().len());
        let square = hsv_square_rect(r);
        assert_eq!(
            sv_at(square, square.left - 50.0, square.top - 50.0),
            (0.0, 1.0)
        );
        assert_eq!(
            sv_at(square, square.right + 50.0, square.bottom + 50.0),
            (1.0, 0.0)
        );
    }

    #[test]
    fn hue_at_spans_the_full_wheel() {
        let r = rect(presets().len());
        let strip = hsv_hue_rect(r);
        assert_eq!(hue_at(strip, strip.top), 0.0);
        assert_eq!(hue_at(strip, strip.bottom), 360.0);
        assert!((hue_at(strip, strip.top + strip.height() / 2.0) - 180.0).abs() < 0.5);
    }

    #[test]
    fn hex_field_at_finds_each_row() {
        let r = rect(presets().len());
        let hex = hex_row_rect(r);
        assert_eq!(
            hex_field_at(r, hex.center_x(), hex.center_y()),
            Some(HexField::Hex)
        );
        for (i, field) in [HexField::R, HexField::G, HexField::B]
            .into_iter()
            .enumerate()
        {
            let row = rgb_row_rect(r, i);
            assert_eq!(hex_field_at(r, row.center_x(), row.center_y()), Some(field));
        }
        assert_eq!(hex_field_at(r, r.left, r.bottom + 50.0), None);
    }

    #[test]
    fn content_height_takes_the_swatch_count_into_account() {
        assert!(content_height(Mode::Swatches, 3) < content_height(Mode::Swatches, 20));
    }

    #[test]
    fn draw_does_not_panic_across_every_mode_and_theme() {
        let presets = presets();
        let (w, h) = panel_size(presets.len());
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((w as i32, h as i32)).expect("surface");
        let canvas = surface.canvas();
        let r = Rect::from_xywh(0.0, 0.0, w, h);
        let color = Color::from_rgb(0x0A, 0x84, 0xFF);
        for theme in [Theme::light(), Theme::dark()] {
            for mode in Mode::ALL {
                draw(
                    canvas,
                    r,
                    mode,
                    color,
                    &presets,
                    Some(0),
                    Some(HexField::Hex),
                    &theme,
                );
            }
        }
    }
}
