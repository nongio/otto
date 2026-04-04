use crate::activity::Activity;
use otto_kit::desktop_entry::lookup_app;
use otto_kit::icons::named_icon_sized;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::typography::TextStyle;
use otto_kit::AppContext;
use skia_safe::{Canvas, Color, Paint, RRect, Rect};

// ---------------------------------------------------------------------------
// Constants (from spec)
// ---------------------------------------------------------------------------

pub const MINI_W: f32 = 44.0;
pub const MINI_H: f32 = 28.0;
pub const COMPACT_W: f32 = 240.0;
pub const COMPACT_H: f32 = 36.0;
pub const CARD_W: f32 = 300.0;
pub const CARD_H: f32 = 60.0;
pub const CARD_GAP: f32 = 4.0;
pub const CARD_RADIUS: f32 = 10.0;
pub const HOVER_GROW: f32 = 4.0;

pub const BUFFER_SCALE: f64 = 2.0;
pub const SLOT_BUF_W: i32 = 460;
pub const SLOT_BUF_H: i32 = 140;

// ---------------------------------------------------------------------------
// Drawing: group pill (Compact mode)
// ---------------------------------------------------------------------------

/// Resolve an app_id to a human-readable display name via XDG desktop entries.
fn app_display_name(app_id: &str) -> String {
    lookup_app(app_id)
        .map(|info| info.name)
        .unwrap_or_else(|| app_id.to_string())
}

/// Compute the width needed for a notification pill based on its content.
/// Measures both rows (app name + title) and uses the wider one.
pub fn pill_width(app_id: &str, title: &str, count: usize) -> f32 {
    let pad = 8.0;
    let icon_size = COMPACT_H - pad * 2.0;
    let text_x = pad + icon_size + 6.0;
    let badge_w = if count > 1 { 8.0 + 18.0 } else { 0.0 };

    // Top row: app name (9px)
    let name = app_display_name(app_id);
    let app_font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 9.0,
    }
    .font();
    let (name_w, _) = app_font.measure_str(&name, None);

    // Bottom row: notification title (11px)
    let display_title = if title.is_empty() { &name } else { title };
    let title_font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 11.0,
    }
    .font();
    let (title_w, _) = title_font.measure_str(display_title, None);

    let text_w = name_w.max(title_w);
    (text_x + text_w + badge_w + pad).clamp(MINI_W, 340.0)
}

pub fn draw_pill(
    canvas: &Canvas,
    app_id: &str,
    icon: &str,
    title: &str,
    count: usize,
    _expanded: bool,
    w: f32,
    h: f32,
) {
    let pad = 8.0;
    let icon_size = h - pad * 2.0;
    let icon_x = pad;
    let icon_y = (h - icon_size) / 2.0;
    draw_app_icon(canvas, icon, icon_x, icon_y, icon_size);

    let text_x = icon_x + icon_size + 6.0;
    let badge_w = if count > 1 { 8.0 + 18.0 } else { 0.0 };
    let max_w = w - text_x - badge_w - pad;

    // Top row: app name (small, dimmer)
    let name = app_display_name(app_id);
    let app_font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 9.0,
    }
    .font();
    let mut app_paint = Paint::default();
    app_paint.set_anti_alias(true);
    app_paint.set_color(Color::from_argb(140, 255, 255, 255));
    let app_label = truncate_text(&name, &app_font, max_w);
    canvas.draw_str(&app_label, (text_x, h / 2.0 - 3.0), &app_font, &app_paint);

    // Bottom row: notification title (bold, white)
    let title_font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 11.0,
    }
    .font();
    let mut title_paint = Paint::default();
    title_paint.set_anti_alias(true);
    title_paint.set_color(Color::WHITE);
    let display_title = if title.is_empty() { &name } else { title };
    let title_label = truncate_text(display_title, &title_font, max_w);
    canvas.draw_str(
        &title_label,
        (text_x, h / 2.0 + 9.0),
        &title_font,
        &title_paint,
    );

    // Count badge
    if count > 1 {
        draw_count_badge(canvas, w - pad - 18.0, (h - 14.0) / 2.0, count);
    }
}

// ---------------------------------------------------------------------------
// Drawing: peek pill (two-line: app name + notification title)
// ---------------------------------------------------------------------------
// Drawing: group circle (Mini mode)
// ---------------------------------------------------------------------------

/// Width for a mini pill: smaller when just one notification.
pub fn mini_width(count: usize) -> f32 {
    if count > 1 {
        MINI_W
    } else {
        MINI_H // square pill (icon only)
    }
}

pub fn draw_mini(canvas: &Canvas, icon: &str, count: usize, _w: f32, h: f32) {
    let pad = 6.0;
    let icon_size = h - pad * 2.0;
    let icon_x = pad;
    let icon_y = (h - icon_size) / 2.0;
    draw_app_icon(canvas, icon, icon_x, icon_y, icon_size);

    if count > 1 {
        let count_text = format!("{count}");
        let font = TextStyle {
            family: "Inter",
            weight: 600,
            size: 11.0,
        }
        .font();
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::WHITE);
        canvas.draw_str(
            &count_text,
            (icon_x + icon_size + 4.0, h / 2.0 + 4.0),
            &font,
            &paint,
        );
    }
}

// ---------------------------------------------------------------------------
// Drawing: notification card
// ---------------------------------------------------------------------------

pub fn draw_card(canvas: &Canvas, activity: &Activity, group_icon: &str, w: f32, h: f32) {
    let theme = AppContext::current_theme();
    let pad = 10.0;

    // Use the activity icon if available, otherwise fall back to group icon.
    let icon = if activity.icon.is_empty() {
        group_icon
    } else {
        &activity.icon
    };

    // Background — uses theme material
    let mut bg = Paint::default();
    bg.set_anti_alias(true);
    bg.set_color(theme.material_medium);
    canvas.draw_rrect(
        RRect::new_rect_xy(Rect::from_xywh(0.0, 0.0, w, h), CARD_RADIUS, CARD_RADIUS),
        &bg,
    );

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
    title_paint.set_color(theme.text_primary);
    let close_zone = 40.0;
    let text_x = icon_x + icon_size + 8.0;
    let max_w = w - text_x - close_zone;
    let title = truncate_text(&activity.title, &title_font, max_w);
    canvas.draw_str(&title, (text_x, pad + 13.0), &title_font, &title_paint);

    // Body
    if !activity.body.is_empty() {
        let body_font = TextStyle {
            family: "Inter",
            weight: 400,
            size: 11.0,
        }
        .font();
        let mut body_paint = Paint::default();
        body_paint.set_anti_alias(true);
        body_paint.set_color(theme.text_secondary);
        let body = truncate_text(&activity.body, &body_font, max_w);
        canvas.draw_str(&body, (text_x, pad + 28.0), &body_font, &body_paint);
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
    hint_paint.set_color(theme.text_tertiary);
    let elapsed = activity.created_at.elapsed().as_secs();
    let time_str = if elapsed < 60 {
        "just now".to_string()
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else {
        format!("{}h ago", elapsed / 3600)
    };
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
    sep_paint.set_color(Color::from_argb(20, 0, 0, 0));
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
    close_paint.set_color(theme.text_secondary);
    let (cw, _) = close_font.measure_str("Close", None);
    canvas.draw_str(
        "Close",
        (w - close_zone / 2.0 - cw / 2.0, h / 2.0 + 3.0),
        &close_font,
        &close_paint,
    );
}

// ---------------------------------------------------------------------------
// Helpers: icons, badges, text
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

fn draw_count_badge(canvas: &Canvas, x: f32, y: f32, count: usize) {
    let badge_w = 18.0_f32;
    let badge_h = 14.0_f32;
    let badge_r = 7.0_f32;

    let mut bg = Paint::default();
    bg.set_anti_alias(true);
    bg.set_color(Color::from_argb(80, 255, 255, 255));
    canvas.draw_rrect(
        RRect::new_rect_xy(Rect::from_xywh(x, y, badge_w, badge_h), badge_r, badge_r),
        &bg,
    );

    let text = format!("{count}");
    let font = TextStyle {
        family: "Inter",
        weight: 600,
        size: 10.0,
    }
    .font();
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::WHITE);
    let (cw, _) = font.measure_str(&text, None);
    canvas.draw_str(&text, (x + (badge_w - cw) / 2.0, y + 11.0), &font, &paint);
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

pub fn apply_island_style(
    surface: &otto_kit::SubsurfaceSurface,
    radius: f64,
    gravity: ContentsGravity,
) {
    if let Some(ss) = surface.base_surface().surface_style() {
        ss.set_background_color(0.03, 0.03, 0.03, 1.0);
        ss.set_corner_radius(radius * BUFFER_SCALE);
        ss.set_masks_to_bounds(ClipMode::Enabled);
        ss.set_shadow(0.2, 2.0, 0.0, 8.0, 0.0, 0.0, 0.0);
        ss.set_blend_mode(BlendMode::BackgroundBlur);
        ss.set_contents_gravity(gravity);
        ss.set_anchor_point(0.5, 0.5);
    }
}

/// Style for notification cards — theme material background with blur.
pub fn apply_card_style(surface: &otto_kit::SubsurfaceSurface) {
    let theme = AppContext::current_theme();
    let c = theme.material_medium;
    if let Some(ss) = surface.base_surface().surface_style() {
        ss.set_background_color(
            c.r() as f64 / 255.0,
            c.g() as f64 / 255.0,
            c.b() as f64 / 255.0,
            c.a() as f64 / 255.0,
        );
        ss.set_corner_radius(CARD_RADIUS as f64 * BUFFER_SCALE);
        ss.set_masks_to_bounds(ClipMode::Enabled);
        ss.set_shadow(0.25, 16.0, 0.0, 1.0, 0.0, 0.0, 0.0);
        ss.set_blend_mode(BlendMode::BackgroundBlur);
        ss.set_contents_gravity(ContentsGravity::Center);
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
        ss.set_size(w as f64 * BUFFER_SCALE, h as f64 * BUFFER_SCALE);
        ss.set_position(x as f64 * BUFFER_SCALE, y as f64 * BUFFER_SCALE);
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

            let timing = scene.create_timing_function(qh, ());
            timing.set_spring(0.15, 0.0);

            let anim = scene.begin_transaction(qh, ());
            anim.set_duration(0.8);
            if delay > 0.0 {
                anim.set_delay(delay);
            }
            anim.set_timing_function(&timing);

            scene_surface.set_size(w as f64 * BUFFER_SCALE, h as f64 * BUFFER_SCALE);
            scene_surface.set_position(x as f64 * BUFFER_SCALE, y as f64 * BUFFER_SCALE);
            scene_surface.set_corner_radius(radius * BUFFER_SCALE);
            if let Some(o) = opacity {
                scene_surface.set_opacity(o);
            }

            anim.commit();
        }
    }
}

/// Animate only position and opacity (size is set instantly, not animated).
pub fn animate_position_opacity(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    opacity: f64,
    delay: f64,
) {
    animate_position_opacity_duration(surface, w, h, x, y, opacity, delay, 0.3);
}

pub fn animate_position_opacity_slow(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    opacity: f64,
    delay: f64,
) {
    animate_position_opacity_duration(surface, w, h, x, y, opacity, delay, 0.8);
}

fn animate_position_opacity_duration(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    opacity: f64,
    delay: f64,
    duration: f64,
) {
    if let Some(scene_surface) = surface.base_surface().surface_style() {
        // Set size immediately (outside any transaction).
        scene_surface.set_size(w as f64 * BUFFER_SCALE, h as f64 * BUFFER_SCALE);

        if let Some(scene) = AppContext::surface_style_manager() {
            let qh = AppContext::queue_handle();

            let timing = scene.create_timing_function(qh, ());
            timing.set_spring(0.0, 0.0);

            let anim = scene.begin_transaction(qh, ());
            anim.set_duration(duration);
            if delay > 0.0 {
                anim.set_delay(delay);
            }
            anim.set_timing_function(&timing);

            scene_surface.set_position(x as f64 * BUFFER_SCALE, y as f64 * BUFFER_SCALE);
            scene_surface.set_opacity(opacity);

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

/// Pulse animation: instantly grow by `grow` pixels, then spring back to target size.
/// With center anchor, position stays the same — only size changes.
pub fn animate_pulse(
    surface: &otto_kit::SubsurfaceSurface,
    w: f32,
    h: f32,
    cx: f32,
    cy: f32,
    radius: f64,
    grow: f32,
) {
    // Step 1: instantly set to enlarged size (center anchor keeps it centered).
    set_size_and_position(surface, w + grow, h + grow, cx, cy);

    // Step 2: spring back to target size.
    animate_to(surface, w, h, cx, cy, radius, 0.0);
}

/// Draw content centered in the subsurface buffer.
pub fn draw_centered(
    surface: &otto_kit::SubsurfaceSurface,
    content_w: f32,
    content_h: f32,
    draw_fn: impl FnOnce(&Canvas),
) {
    surface.draw(|canvas| {
        let tx = (SLOT_BUF_W as f32 - content_w) / 2.0;
        let ty = (SLOT_BUF_H as f32 - content_h) / 2.0;
        canvas.save();
        canvas.translate((tx, ty));
        draw_fn(canvas);
        canvas.restore();
    });
}
