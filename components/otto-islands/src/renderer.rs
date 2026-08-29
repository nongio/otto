use crate::activity::{Activity, NotificationAction};
use otto_kit::icons::named_icon_sized;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::typography::TextStyle;
use otto_kit::AppContext;
use skia_safe::{Canvas, Color, Paint, RRect, Rect};

// ---------------------------------------------------------------------------
// Constants (from spec)
// ---------------------------------------------------------------------------

pub const MINI_H: f32 = 28.0;
pub const COMPACT_H: f32 = 36.0;
pub const CARD_W: f32 = 300.0;
/// Height of a card whose body fits on one line and which has no actions.
/// A longer body or an action row grows it — see [`card_height`].
pub const CARD_H: f32 = 68.0;
/// Baseline-to-baseline distance between wrapped body lines.
const BODY_LINE_H: f32 = 14.0;
/// A notification that runs longer than this is ellipsised: past three lines
/// the island stops being a glance and wants opening in the app instead.
const MAX_BODY_LINES: usize = 3;
const CARD_PAD: f32 = 10.0;
const CARD_ICON: f32 = 24.0;
const CARD_CLOSE_ZONE: f32 = 40.0;
/// Height of the inline action button row, and the gap above it.
const ACTION_ROW_H: f32 = 18.0;
const ACTION_ROW_GAP: f32 = 8.0;
/// Corner radius of an open notification — noticeably squarer than the fully
/// rounded pill/circle it grew out of.
pub const CARD_RADIUS: f32 = 12.0;

/// Horizontal offset between stacked islands of the same app at rest — small,
/// so they read as one deck with only a sliver of each one behind showing.
pub const PEEK_STEP: f32 = 8.0;
/// Offset between stacked islands when the group is hovered — fanned out far
/// enough that each one can be aimed at and clicked individually.
pub const FAN_STEP: f32 = 20.0;
/// Islands past this depth in a group pile up at the same offset, so a huge
/// group can't grow the row without bound.
pub const MAX_STACK: usize = 5;

pub const SLOT_BUF_W: i32 = 460;
pub const SLOT_BUF_H: i32 = 140;

/// Physical-pixel scale for geometry pushed to `otto-surface-style-v1`
/// (that protocol operates in physical pixels). Must track the real output
/// scale, not the client's fixed 2x raster buffer scale — see
/// `otto-bar/src/app.rs`'s `animate_right_size` for the same fix.
pub fn buffer_scale() -> f64 {
    AppContext::fractional_scale()
}

// ---------------------------------------------------------------------------
// Drawing: group pill (Compact mode)
// ---------------------------------------------------------------------------

/// Compute the width a Compact pill needs for one notification's own title.
pub fn pill_width(title: &str) -> f32 {
    let pad = 8.0;
    let icon_size = COMPACT_H - pad * 2.0;
    let text_x = pad + icon_size + 6.0;
    let font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 11.0,
    }
    .font();
    let (title_w, _) = font.measure_str(title, None);
    (text_x + title_w + pad).clamp(MINI_H, 300.0)
}

/// Compact: this notification's own icon and title on one line.
pub fn draw_pill(canvas: &Canvas, icon: &str, title: &str, w: f32, h: f32) {
    let pad = 8.0;
    let icon_size = h - pad * 2.0;
    let icon_x = pad;
    let icon_y = (h - icon_size) / 2.0;
    draw_app_icon(canvas, icon, icon_x, icon_y, icon_size);

    let text_x = icon_x + icon_size + 6.0;
    let max_w = w - text_x - pad;

    let font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 11.0,
    }
    .font();
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::WHITE);
    let label = truncate_text(title, &font, max_w);
    canvas.draw_str(&label, (text_x, h / 2.0 + 4.0), &font, &paint);
}

// ---------------------------------------------------------------------------
// Drawing: mini circle
// ---------------------------------------------------------------------------

/// Mini: just this notification's icon in a circle. There is no count badge —
/// how many bubbles are stacked behind is what conveys the count.
pub fn draw_mini(canvas: &Canvas, icon: &str, _w: f32, h: f32) {
    let pad = 6.0;
    let icon_size = h - pad * 2.0;
    draw_app_icon(canvas, icon, pad, (h - icon_size) / 2.0, icon_size);
}

// ---------------------------------------------------------------------------
// Drawing: notification card
// ---------------------------------------------------------------------------

/// Human-readable elapsed-time label shown on a card ("just now", "5m ago").
/// Also part of the card content signature used to skip unchanged redraws.
pub fn elapsed_label(created_at: std::time::Instant) -> String {
    let elapsed = created_at.elapsed().as_secs();
    if elapsed < 60 {
        otto_kit::t_owned!("islands-elapsed-just-now")
    } else if elapsed < 3600 {
        otto_kit::t_owned!("islands-elapsed-minutes", count = (elapsed / 60) as f64)
    } else {
        otto_kit::t_owned!("islands-elapsed-hours", count = (elapsed / 3600) as f64)
    }
}

/// Expanded: the island itself grown into a full notification, drawn directly
/// into the same bubble — icon, title, body (or inline actions), time, close.
pub fn draw_card(canvas: &Canvas, activity: &Activity, w: f32, h: f32) {
    let pad = 10.0;
    let icon = activity.icon.as_str();

    // No background rect here — the subsurface itself is the near-black
    // bubble material (apply_island_style).

    // Icon
    let icon_size = 24.0;
    let icon_x = pad;
    let icon_y = pad;
    draw_app_icon(canvas, icon, icon_x, icon_y, icon_size);

    // Title
    let title_font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 12.0,
    }
    .font();
    let mut title_paint = Paint::default();
    title_paint.set_anti_alias(true);
    title_paint.set_color(Color::WHITE);
    let close_zone = 40.0;
    let text_x = icon_x + icon_size + 8.0;
    let max_w = w - text_x - close_zone;
    let title = truncate_text(&activity.title, &title_font, max_w);
    canvas.draw_str(&title, (text_x, pad + 13.0), &title_font, &title_paint);

    // Body, wrapped over as many lines as the card was sized for. The card
    // grows to fit it, so the text is readable on arrival rather than cut off
    // after a few words.
    if !activity.body.is_empty() {
        let body_font = body_font();
        let mut body_paint = Paint::default();
        body_paint.set_anti_alias(true);
        body_paint.set_color(Color::from_argb(180, 255, 255, 255));
        for (i, line) in card_body_lines(&activity.body, w).iter().enumerate() {
            let y = BODY_TOP_BASELINE + i as f32 * BODY_LINE_H;
            canvas.draw_str(line, (text_x, y), &body_font, &body_paint);
        }
    }

    // Inline action buttons, on their own row under the body.
    if !activity.actions.is_empty() {
        for (bx, by, bw, bh, _id, label) in card_action_rects(&activity.body, &activity.actions, w)
        {
            let mut btn_bg = Paint::default();
            btn_bg.set_anti_alias(true);
            btn_bg.set_color(Color::from_argb(50, 255, 255, 255));
            canvas.draw_rrect(
                RRect::new_rect_xy(Rect::from_xywh(bx, by, bw, bh), bh / 2.0, bh / 2.0),
                &btn_bg,
            );

            let btn_font = TextStyle {
                family: "Inter",
                weight: 600,
                size: 10.0,
            }
            .font();
            let mut btn_paint = Paint::default();
            btn_paint.set_anti_alias(true);
            btn_paint.set_color(Color::WHITE);
            canvas.draw_str(&label, (bx + 8.0, by + bh - 5.0), &btn_font, &btn_paint);
        }
    }

    // Elapsed time
    let hint_font = TextStyle {
        family: "Inter",
        weight: 400,
        size: 9.0,
    }
    .font();
    let mut hint_paint = Paint::default();
    hint_paint.set_anti_alias(true);
    hint_paint.set_color(Color::from_argb(120, 255, 255, 255));
    let time_str = elapsed_label(activity.created_at);
    let (tw, _) = hint_font.measure_str(&time_str, None);
    canvas.draw_str(
        &time_str,
        (w - close_zone - tw - 8.0, h - pad + 2.0),
        &hint_font,
        &hint_paint,
    );

    // Separator line before close zone
    let mut sep_paint = Paint::default();
    sep_paint.set_anti_alias(true);
    sep_paint.set_color(Color::from_argb(30, 255, 255, 255));
    sep_paint.set_stroke_width(1.0);
    let sep_x = w - close_zone;
    canvas.draw_line((sep_x, 0.0), (sep_x, h), &sep_paint);

    // Close button — right zone
    let close_font = TextStyle {
        family: "Inter",
        weight: 500,
        size: 9.0,
    }
    .font();
    let mut close_paint = Paint::default();
    close_paint.set_anti_alias(true);
    close_paint.set_color(Color::from_argb(180, 255, 255, 255));
    let close_label = otto_kit::t!("islands-close");
    let (cw, _) = close_font.measure_str(close_label, None);
    canvas.draw_str(
        close_label,
        (w - close_zone / 2.0 - cw / 2.0, h / 2.0 + 3.0),
        &close_font,
        &close_paint,
    );
}

// ---------------------------------------------------------------------------
// Helpers: icons, text
// ---------------------------------------------------------------------------
// (dialog and fingerprint rendering will be added in separate PRs)

// ---------------------------------------------------------------------------

fn draw_app_icon(canvas: &Canvas, icon_name: &str, x: f32, y: f32, size: f32) {
    if !icon_name.is_empty() {
        if let Some(icon) = named_icon_sized(icon_name, size as i32) {
            let dst = Rect::from_xywh(x, y, size, size);
            let r = size * 0.18;
            canvas.save();
            canvas.clip_rrect(
                RRect::new_rect_xy(dst, r, r),
                skia_safe::ClipOp::Intersect,
                true,
            );
            let src = Rect::from_xywh(0.0, 0.0, icon.width() as f32, icon.height() as f32);
            canvas.draw_image_rect(
                &icon,
                Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
                dst,
                &Paint::default(),
            );
            canvas.restore();
            return;
        }
    }
    draw_envelope(canvas, x, y, size);
}

fn draw_envelope(canvas: &Canvas, x: f32, y: f32, size: f32) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(200, 255, 255, 255));
    paint.set_style(skia_safe::paint::Style::Stroke);
    paint.set_stroke_width(1.2);

    let w = size;
    let h = size * 0.7;
    let oy = y + (size - h) / 2.0;
    canvas.draw_rect(Rect::from_xywh(x, oy, w, h), &paint);

    let mut b = skia_safe::PathBuilder::new();
    b.move_to((x, oy));
    b.line_to((x + w / 2.0, oy + h * 0.55));
    b.line_to((x + w, oy));
    canvas.draw_path(&b.detach(), &paint);
}

/// Lay out a row of inline action-button chips starting at `(text_x, pad + 28.0)`,
/// left to right, dropping any that don't fit within `max_w`. Returns
/// `(x, y, w, h, action_id, label)` per button in card-local coordinates —
/// shared by `draw_card` (drawing) and hit-testing (`main.rs`) so the two
/// never disagree about where a button is.
pub fn action_button_rects(
    actions: &[NotificationAction],
    text_x: f32,
    max_w: f32,
    y: f32,
) -> Vec<(f32, f32, f32, f32, String, String)> {
    const BTN_H: f32 = ACTION_ROW_H;
    const GAP: f32 = 6.0;
    let font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 10.0,
    }
    .font();

    let mut rects = Vec::new();
    let mut x = text_x;
    for action in actions {
        let (tw, _) = font.measure_str(&action.label, None);
        let bw = tw + 16.0;
        if x + bw > text_x + max_w {
            break;
        }
        rects.push((x, y, bw, BTN_H, action.id.clone(), action.label.clone()));
        x += bw + GAP;
    }
    rects
}

fn body_font() -> skia_safe::Font {
    TextStyle {
        family: "Inter",
        weight: 400,
        size: 11.0,
    }
    .font()
}

/// Where the text column starts, and how wide it is — shared by drawing, the
/// wrap measurement, and hit-testing so all three agree.
fn card_text_column(w: f32) -> (f32, f32) {
    let text_x = CARD_PAD + CARD_ICON + 8.0;
    (text_x, w - text_x - CARD_CLOSE_ZONE)
}

/// Break `text` into at most `max_lines` lines that each fit `max_width`.
/// The last line is ellipsised if there is more text than fits. Words longer
/// than the column are broken mid-word rather than overflowing.
fn wrap_text(text: &str, font: &skia_safe::Font, max_width: f32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();

    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if font.measure_str(&candidate, None).0 <= max_width {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            if lines.len() == max_lines {
                break;
            }
        }
        // The word alone doesn't fit the column: let truncate_text break it.
        if font.measure_str(word, None).0 > max_width {
            lines.push(truncate_text(word, font, max_width));
            if lines.len() == max_lines {
                break;
            }
        } else {
            line = word.to_string();
        }
    }
    if !line.is_empty() && lines.len() < max_lines {
        lines.push(line);
    }

    // Anything left over means we ran out of lines — mark the last one.
    let drawn: usize = lines.iter().map(|l| l.split_whitespace().count()).sum();
    if drawn < text.split_whitespace().count() {
        if let Some(last) = lines.last_mut() {
            *last = truncate_text(&format!("{last} …"), font, max_width);
        }
    }
    lines
}

/// The body lines a card will draw at width `w`.
fn card_body_lines(body: &str, w: f32) -> Vec<String> {
    if body.is_empty() {
        return Vec::new();
    }
    let (_, max_w) = card_text_column(w);
    wrap_text(body, &body_font(), max_w, MAX_BODY_LINES)
}

/// How tall this notification's card needs to be: the one-line card, plus a
/// line for each extra body line, plus the action row when it has actions.
/// Layout, drawing, and hit-testing all size the card through this.
pub fn card_height(activity: &Activity) -> f32 {
    let lines = card_body_lines(&activity.body, CARD_W).len().max(1);
    let mut h = CARD_H + (lines as f32 - 1.0) * BODY_LINE_H;
    if !activity.actions.is_empty() {
        h += ACTION_ROW_H + ACTION_ROW_GAP;
    }
    h
}

/// Baseline of the first body line.
const BODY_TOP_BASELINE: f32 = CARD_PAD + 28.0;

/// Top of the inline action row, sitting below the last body line.
fn action_row_y(body: &str, w: f32) -> f32 {
    let lines = card_body_lines(body, w).len().max(1);
    BODY_TOP_BASELINE + (lines as f32 - 1.0) * BODY_LINE_H + ACTION_ROW_GAP
}

/// The inline action buttons of a card of width `w`, in card-local
/// coordinates. Drawing and hit-testing both go through this so a button is
/// clickable exactly where it was painted.
pub fn card_action_rects(
    body: &str,
    actions: &[NotificationAction],
    w: f32,
) -> Vec<(f32, f32, f32, f32, String, String)> {
    let (text_x, max_w) = card_text_column(w);
    action_button_rects(actions, text_x, max_w, action_row_y(body, w))
}

fn truncate_text(text: &str, font: &skia_safe::Font, max_width: f32) -> String {
    let (width, _) = font.measure_str(text, None);
    if width <= max_width {
        return text.to_string();
    }
    let ellipsis = "…";
    let (ew, _) = font.measure_str(ellipsis, None);
    let available = max_width - ew;

    let mut result = String::new();
    for ch in text.chars() {
        result.push(ch);
        let (w, _) = font.measure_str(&result, None);
        if w > available {
            result.pop();
            break;
        }
    }
    result.push_str(ellipsis);
    result
}

// ---------------------------------------------------------------------------
// Surface style helpers
// ---------------------------------------------------------------------------

/// Moving and resizing get their own springs, because they read as different
/// kinds of motion: a bubble shoved aside by its neighbour should overshoot
/// and rock back, while the same bubble growing should just settle.
///
/// Bounce for being pushed along the row — overshoots the target and comes
/// back, enough to feel shoved rather than slid.
const MOVE_BOUNCE: f64 = 0.45;
const MOVE_DURATION: f64 = 0.6;
/// Bounce for growing and shrinking — barely any, so a bubble changing size
/// doesn't wobble while its content is being read.
const RESIZE_BOUNCE: f64 = 0.04;
const RESIZE_DURATION: f64 = 0.45;

pub fn apply_island_style(
    surface: &otto_kit::SubsurfaceSurface,
    radius: f64,
    gravity: ContentsGravity,
) {
    if let Some(ss) = surface.base_surface().surface_style() {
        ss.set_background_color(0.03, 0.03, 0.03, 1.0);
        ss.set_corner_radius(radius);
        ss.set_masks_to_bounds(ClipMode::Enabled);
        ss.set_shadow(0.2, 2.0, 0.0, 8.0, 0.0, 0.0, 0.0);
        ss.set_blend_mode(BlendMode::BackgroundBlur);
        ss.set_contents_gravity(gravity);
        ss.set_anchor_point(0.5, 0.5);
    }
}

pub fn set_size_and_position(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
) {
    if let Some(ss) = surface.base_surface().surface_style() {
        ss.set_size(w as f64 * buffer_scale(), h as f64 * buffer_scale());
        ss.set_position(x as f64 * buffer_scale(), y as f64 * buffer_scale());
    }
}

pub fn animate_to(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    radius: f64,
    delay: f64,
) {
    animate_to_with_opacity(surface, w, h, x, y, radius, None, delay);
}

pub fn animate_to_with_opacity(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    radius: f64,
    opacity: Option<f64>,
    delay: f64,
) {
    if let Some(scene_surface) = surface.base_surface().surface_style() {
        if let Some(scene) = AppContext::surface_style_manager() {
            let qh = AppContext::queue_handle();

            // Two transactions, so position and size can carry different
            // springs: the bubble is shoved to its new spot with overshoot
            // while it settles into its new size without wobbling.
            let move_timing = scene.create_timing_function(qh, ());
            move_timing.set_spring(MOVE_BOUNCE, 0.0);
            let move_anim = scene.begin_transaction(qh, ());
            move_anim.set_duration(MOVE_DURATION);
            if delay > 0.0 {
                move_anim.set_delay(delay);
            }
            move_anim.set_timing_function(&move_timing);
            scene_surface.set_position(x as f64 * buffer_scale(), y as f64 * buffer_scale());
            move_anim.commit();

            let resize_timing = scene.create_timing_function(qh, ());
            resize_timing.set_spring(RESIZE_BOUNCE, 0.0);
            let resize_anim = scene.begin_transaction(qh, ());
            resize_anim.set_duration(RESIZE_DURATION);
            if delay > 0.0 {
                resize_anim.set_delay(delay);
            }
            resize_anim.set_timing_function(&resize_timing);
            scene_surface.set_size(w as f64 * buffer_scale(), h as f64 * buffer_scale());
            scene_surface.set_corner_radius(radius);
            if let Some(o) = opacity {
                scene_surface.set_opacity(o);
            }
            resize_anim.commit();
        }
    }
}

/// Entrance animation: pop open from a small rounded shape.
///
/// The surface must already be sized and positioned at its final layout — this
/// only drives the transform, so the content never reflows. Starts as a small,
/// fully-rounded, transparent blob at the panel's centre (anchor is 0.5/0.5)
/// and springs out to full size, the corner radius relaxing to `radius` on
/// the way.
pub fn animate_enter_pop(surface: &otto_kit::SubsurfaceSurface, radius: f64) {
    /// Scale the panel starts at. Enough to read as "a shape opening up"
    /// without launching it across the row.
    const FROM_SCALE: f64 = 0.62;
    /// Corner radius at rest in the start state. Once divided by FROM_SCALE it
    /// is far larger than half the collapsed box, so the blob reads as a pill.
    const FROM_RADIUS: f64 = 44.0;
    /// Spring bounce — a little more than a resize settles with, since this is
    /// the island announcing itself, but nowhere near a pop.
    const BOUNCE: f64 = 0.2;

    if let Some(scene_surface) = surface.base_surface().surface_style() {
        // Start state, applied outside any transaction so it is instant.
        scene_surface.set_scale(FROM_SCALE, FROM_SCALE);
        scene_surface.set_corner_radius(FROM_RADIUS);
        scene_surface.set_opacity(0.0);

        if let Some(scene) = AppContext::surface_style_manager() {
            let qh = AppContext::queue_handle();

            let timing = scene.create_timing_function(qh, ());
            timing.set_spring(BOUNCE, 0.0);

            let anim = scene.begin_transaction(qh, ());
            anim.set_duration(0.55);
            anim.set_timing_function(&timing);

            scene_surface.set_scale(1.0, 1.0);
            scene_surface.set_corner_radius(radius);
            scene_surface.set_opacity(1.0);

            anim.commit();
        }
    }
}

/// Dismiss animation: scale up to `scale` while fading out to opacity 0.
pub fn animate_dismiss(surface: &otto_kit::SubsurfaceSurface, scale: f64) {
    if let Some(scene_surface) = surface.base_surface().surface_style() {
        if let Some(scene) = AppContext::surface_style_manager() {
            let qh = AppContext::queue_handle();

            let timing = scene.create_timing_function(qh, ());
            timing.set_spring(0.25, 0.0);

            let anim = scene.begin_transaction(qh, ());
            anim.set_duration(0.4);
            anim.set_timing_function(&timing);

            scene_surface.set_scale(scale, scale);
            scene_surface.set_opacity(0.0);

            anim.commit();
        }
    }
}

/// Paint one island's content: size the buffer to exactly `w x h` logical
/// points, then draw from the origin.
///
/// The surface style uses `ContentsGravity::TopLeft`, so buffer (0,0) is the
/// island's top-left corner and nothing here depends on the layer's current
/// (animating) size. Every mode gets a buffer its own size — a Mini pill no
/// longer carries a 460x140 slot of which a 28x28 corner is visible.
pub fn draw_content(
    surface: &mut otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    draw_fn: impl FnOnce(&Canvas),
) {
    let (bw, bh) = (w.ceil() as i32, h.ceil() as i32);
    if surface.base_surface().size() != (bw, bh) {
        surface.resize(bw, bh);
    }
    surface.draw(|canvas| {
        canvas.clear(Color::TRANSPARENT);
        draw_fn(canvas);
    });
}
