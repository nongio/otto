//! Access-style dialog: a modal permission/choice panel rendered as a dropdown
//! below the island bar.
//!
//! Mirrors the semantics of `org.freedesktop.impl.portal.Access`: a caller
//! presents a request with title/subtitle/body/icon and zero or more choice
//! groups; the user confirms (grant) or cancels (deny), and a
//! [`DialogResponse`] is returned over the channel held in the request.
//!
//! See `specs/portal-access-dialog.md`.

use otto_kit::icons::named_icon_sized;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::typography::TextStyle;
use otto_kit::SubsurfaceSurface;
use skia_safe::{Canvas, Color, Paint, RRect, Rect};
use tokio::sync::oneshot;

use crate::renderer::BUFFER_SCALE;

pub type DialogId = u64;

// ---------------------------------------------------------------------------
// Geometry constants (logical units — same space the canvas draws in)
// ---------------------------------------------------------------------------

pub const DIALOG_W: f32 = 320.0;
const PAD: f32 = 18.0;
const ICON: f32 = 44.0;
const ICON_GAP: f32 = 10.0;
const TITLE_GAP: f32 = 6.0;
const SUBTITLE_GAP: f32 = 4.0;
const BODY_GAP: f32 = 8.0;
const GROUP_LABEL_H: f32 = 18.0;
const OPTION_H: f32 = 44.0;
const OPTION_GAP: f32 = 6.0;
const OPTION_RADIUS: f32 = 11.0;
/// Space kept clear at the right edge of an option row for the checkmark.
/// Reserved on unselected rows too, so labels don't reflow as selection moves.
const CHECK_GUTTER: f32 = 30.0;
const BTN_H: f32 = 36.0;
const BTN_GAP: f32 = 10.0;
const BTN_RADIUS: f32 = 10.0;
pub const PANEL_RADIUS: f32 = 20.0;

/// Subsurface buffer dimensions (logical units passed to `SubsurfaceSurface::new`).
/// Sized generously so a tall stack of choices fits without reallocation.
pub const DIALOG_BUF_W: i32 = 360;
pub const DIALOG_BUF_H: i32 = 640;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// One selectable option within a choice group.
#[derive(Clone, Debug)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    pub icon: String,
}

/// A single-select group of options (e.g. "output" → list of connectors).
#[derive(Clone, Debug)]
pub struct ChoiceGroup {
    pub id: String,
    pub label: String,
    pub options: Vec<ChoiceOption>,
    /// Index of the initially-selected option.
    pub default: usize,
}

/// The user's decision, returned to the caller.
#[derive(Clone, Debug, Default)]
pub struct DialogResponse {
    /// `0` granted/confirmed, `1` cancelled/denied, `2` ended (withdrawn/error).
    pub response: u32,
    /// `(group_id, selected_option_id)` for each choice group.
    pub results: Vec<(String, String)>,
}

impl DialogResponse {
    pub fn ended() -> Self {
        Self {
            response: 2,
            results: Vec::new(),
        }
    }
}

/// A pending dialog request. Owns the one-shot channel used to deliver the
/// decision back to the (async) D-Bus caller.
pub struct DialogRequest {
    pub id: DialogId,
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub icon: String,
    pub grant_label: String,
    pub deny_label: String,
    pub modal: bool,
    pub choices: Vec<ChoiceGroup>,
    pub response_tx: Option<oneshot::Sender<DialogResponse>>,
}

impl DialogRequest {
    /// A clone-able display snapshot (without the response channel) for the UI.
    pub fn view(&self) -> DialogView {
        DialogView {
            id: self.id,
            app_id: self.app_id.clone(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            body: self.body.clone(),
            icon: self.icon.clone(),
            grant_label: self.grant_label.clone(),
            deny_label: self.deny_label.clone(),
            modal: self.modal,
            choices: self.choices.clone(),
        }
    }

    /// True once the caller has abandoned the request (receiver dropped).
    pub fn is_withdrawn(&self) -> bool {
        self.response_tx.as_ref().is_none_or(|tx| tx.is_closed())
    }
}

/// Display snapshot of a [`DialogRequest`] used by the render/UI layer.
#[derive(Clone)]
pub struct DialogView {
    pub id: DialogId,
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub icon: String,
    pub grant_label: String,
    pub deny_label: String,
    pub modal: bool,
    pub choices: Vec<ChoiceGroup>,
}

/// What was hit by a pointer, in panel-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogHit {
    Grant,
    Deny,
    Option { group: usize, option: usize },
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Computed geometry for a dialog panel, in panel-local coordinates
/// (origin at the panel top-left, logical units).
pub struct DialogLayout {
    pub width: f32,
    pub height: f32,
    pub grant_rect: Rect,
    pub deny_rect: Rect,
    /// `(group_idx, option_idx, row_rect)` for every option row.
    pub option_rects: Vec<(usize, usize, Rect)>,
    // Internal draw anchors (local coords).
    icon_present: bool,
    title_y: f32,
    subtitle_y: Option<f32>,
    body_y: Option<f32>,
    group_label_ys: Vec<(usize, f32)>,
}

/// Compute the panel layout for the given request view.
pub fn dialog_layout(view: &DialogView) -> DialogLayout {
    let w = DIALOG_W;
    let mut y = PAD;

    let icon_present = !view.icon.is_empty();
    if icon_present {
        y += ICON + ICON_GAP;
    }

    // Title (one line, ~18px tall).
    let title_y = y;
    y += 18.0 + TITLE_GAP;

    let subtitle_y = if view.subtitle.is_empty() {
        None
    } else {
        let sy = y;
        y += 15.0 + SUBTITLE_GAP;
        Some(sy)
    };

    let body_y = if view.body.is_empty() {
        None
    } else {
        let by = y;
        y += 16.0 + BODY_GAP;
        Some(by)
    };

    // Choice groups.
    let mut option_rects = Vec::new();
    let mut group_label_ys = Vec::new();
    for (gi, group) in view.choices.iter().enumerate() {
        if group.options.is_empty() {
            continue;
        }
        if !group.label.is_empty() {
            group_label_ys.push((gi, y));
            y += GROUP_LABEL_H;
        }
        for (oi, _opt) in group.options.iter().enumerate() {
            let rect = Rect::from_xywh(PAD, y, w - PAD * 2.0, OPTION_H);
            option_rects.push((gi, oi, rect));
            y += OPTION_H + OPTION_GAP;
        }
        y += 4.0; // extra gap after a group
    }

    // Buttons row.
    y += 2.0;
    let btn_w = (w - PAD * 2.0 - BTN_GAP) / 2.0;
    let deny_rect = Rect::from_xywh(PAD, y, btn_w, BTN_H);
    let grant_rect = Rect::from_xywh(PAD + btn_w + BTN_GAP, y, btn_w, BTN_H);
    y += BTN_H + PAD;

    DialogLayout {
        width: w,
        height: y,
        grant_rect,
        deny_rect,
        option_rects,
        icon_present,
        title_y,
        subtitle_y,
        body_y,
        group_label_ys,
    }
}

fn in_rect(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

/// Hit-test a point in panel-local coordinates.
pub fn hit_test(layout: &DialogLayout, lx: f32, ly: f32) -> Option<DialogHit> {
    if in_rect(&layout.grant_rect, lx, ly) {
        return Some(DialogHit::Grant);
    }
    if in_rect(&layout.deny_rect, lx, ly) {
        return Some(DialogHit::Deny);
    }
    for (gi, oi, rect) in &layout.option_rects {
        if in_rect(rect, lx, ly) {
            return Some(DialogHit::Option {
                group: *gi,
                option: *oi,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn font(size: f32, weight: i32) -> skia_safe::Font {
    TextStyle {
        family: "Inter",
        weight,
        size,
    }
    .font()
}

/// Truncate `text` to fit `max_w`, appending an ellipsis when it doesn't.
///
/// Window titles are arbitrary and frequently longer than the panel — without
/// this they run under the checkmark and off the rounded edge. Walks back by
/// character (not byte) so multi-byte titles can't be split mid-codepoint.
fn ellipsize(text: &str, f: &skia_safe::Font, max_w: f32) -> String {
    if f.measure_str(text, None).0 <= max_w {
        return text.to_string();
    }
    const ELLIPSIS: &str = "…";
    let ellipsis_w = f.measure_str(ELLIPSIS, None).0;
    // Nothing sensible fits — an ellipsis alone still beats overflowing.
    if ellipsis_w > max_w {
        return ELLIPSIS.to_string();
    }
    let budget = max_w - ellipsis_w;
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if f.measure_str(&text[..next], None).0 > budget {
            break;
        }
        end = next;
    }
    // Don't leave a dangling space before the ellipsis.
    format!("{}{ELLIPSIS}", text[..end].trim_end())
}

/// Centered variant: truncates to `max_w`, then centers whatever survived.
fn draw_text_centered_clamped(
    canvas: &Canvas,
    text: &str,
    cx: f32,
    baseline_y: f32,
    f: &skia_safe::Font,
    color: Color,
    max_w: f32,
) {
    draw_text_centered(canvas, &ellipsize(text, f, max_w), cx, baseline_y, f, color);
}

fn draw_text_centered(
    canvas: &Canvas,
    text: &str,
    cx: f32,
    baseline_y: f32,
    f: &skia_safe::Font,
    color: Color,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    let (tw, _) = f.measure_str(text, None);
    canvas.draw_str(text, (cx - tw / 2.0, baseline_y), f, &paint);
}

/// Draw the dialog content into the (already background-styled) subsurface
/// canvas. Content is centered within the buffer, matching the pill/card model.
/// `selected[gi]` is the chosen option index for group `gi`.
pub fn draw_dialog(canvas: &Canvas, view: &DialogView, selected: &[usize], layout: &DialogLayout) {
    // Center the panel within the buffer (same convention as `draw_centered`).
    let ox = (DIALOG_BUF_W as f32 - layout.width) / 2.0;
    let oy = (DIALOG_BUF_H as f32 - layout.height) / 2.0;

    canvas.clear(Color::TRANSPARENT);
    canvas.save();
    canvas.translate((ox, oy));

    let w = layout.width;
    let cx = w / 2.0;
    // Every centered run of text is clamped to the panel's content width.
    let text_max_w = w - PAD * 2.0;
    let theme = otto_kit::AppContext::current_theme();
    // Text is black on light, white on dark — the theme decides. The dialog
    // takes it fully opaque: `text_primary` is deliberately soft (0xD9) for
    // chrome that sits on solid fills, but this panel floats over a blurred
    // backdrop of arbitrary content, where that softness reads as washed out.
    let text = opaque(theme.text_primary);
    // Same reasoning for the supporting text, but the hierarchy is preserved:
    // subtitle and body stay lighter than the title, just not ghostly.
    let dim = at_least_opaque(theme.text_secondary, 0xC0);
    let dim2 = at_least_opaque(theme.text_tertiary, 0x99);
    let accent = theme.accent_blue;
    // Labels drawn on top of the accent fill stay white in both schemes.
    let on_accent = Color::WHITE;

    // No panel fill here: the surface style paints the translucent material over
    // a blurred backdrop (see [`apply_dialog_style`]) and clips the rounded
    // corners, so the canvas only carries content.

    // Icon.
    if layout.icon_present {
        let ix = cx - ICON / 2.0;
        draw_icon(canvas, &view.icon, ix, PAD, ICON);
    }

    // Title.
    draw_text_centered_clamped(
        canvas,
        &view.title,
        cx,
        layout.title_y + 14.0,
        &font(15.0, 700),
        text,
        text_max_w,
    );

    // Subtitle.
    if let Some(sy) = layout.subtitle_y {
        draw_text_centered_clamped(
            canvas,
            &view.subtitle,
            cx,
            sy + 12.0,
            &font(12.0, 500),
            dim,
            text_max_w,
        );
    }

    // Body.
    if let Some(by) = layout.body_y {
        draw_text_centered_clamped(
            canvas,
            &view.body,
            cx,
            by + 12.0,
            &font(12.0, 400),
            dim2,
            text_max_w,
        );
    }

    // Group labels.
    for (gi, gy) in &layout.group_label_ys {
        if let Some(group) = view.choices.get(*gi) {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(dim2);
            let gf = font(11.0, 600);
            canvas.draw_str(
                ellipsize(&group.label, &gf, text_max_w),
                (PAD, gy + 12.0),
                &gf,
                &paint,
            );
        }
    }

    // Option rows.
    for (gi, oi, rect) in &layout.option_rects {
        let Some(group) = view.choices.get(*gi) else {
            continue;
        };
        let Some(opt) = group.options.get(*oi) else {
            continue;
        };
        let is_selected = selected.get(*gi).copied().unwrap_or(group.default) == *oi;

        let mut row_bg = Paint::default();
        row_bg.set_anti_alias(true);
        row_bg.set_color(if is_selected {
            accent
        } else {
            theme.fill_secondary
        });
        canvas.draw_rrect(
            RRect::new_rect_xy(*rect, OPTION_RADIUS, OPTION_RADIUS),
            &row_bg,
        );

        let mut text_x = rect.left + 14.0;
        if !opt.icon.is_empty() {
            let sz = 24.0;
            draw_icon(
                canvas,
                &opt.icon,
                rect.left + 10.0,
                rect.center_y() - sz / 2.0,
                sz,
            );
            text_x = rect.left + 10.0 + sz + 10.0;
        }

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(if is_selected { on_accent } else { text });
        // Reserve the checkmark gutter on every row, selected or not, so a
        // label doesn't reflow when the selection moves.
        let label_font = font(13.0, 500);
        let label_max_w = (rect.right - CHECK_GUTTER) - text_x;
        canvas.draw_str(
            ellipsize(&opt.label, &label_font, label_max_w),
            (text_x, rect.center_y() + 4.5),
            &label_font,
            &paint,
        );

        // Checkmark on the selected row.
        if is_selected {
            let mut ck = Paint::default();
            ck.set_anti_alias(true);
            ck.set_color(on_accent);
            ck.set_style(skia_safe::paint::Style::Stroke);
            ck.set_stroke_width(2.0);
            ck.set_stroke_cap(skia_safe::paint::Cap::Round);
            let mx = rect.right - 22.0;
            let my = rect.center_y();
            let mut p = skia_safe::PathBuilder::new();
            p.move_to((mx - 5.0, my));
            p.line_to((mx - 1.5, my + 4.0));
            p.line_to((mx + 5.0, my - 5.0));
            canvas.draw_path(&p.detach(), &ck);
        }
    }

    // Deny button.
    draw_button(
        canvas,
        &layout.deny_rect,
        &view.deny_label,
        theme.fill_secondary,
        text,
    );
    // Grant button (accent).
    draw_button(
        canvas,
        &layout.grant_rect,
        &view.grant_label,
        accent,
        on_accent,
    );

    canvas.restore();
}

fn draw_button(canvas: &Canvas, rect: &Rect, label: &str, bg: Color, text: Color) {
    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(true);
    bg_paint.set_color(bg);
    canvas.draw_rrect(RRect::new_rect_xy(*rect, BTN_RADIUS, BTN_RADIUS), &bg_paint);
    // Caller-supplied labels ("Share", but also whatever an app passes as
    // grant_label) must not spill past the button's rounded edge.
    draw_text_centered_clamped(
        canvas,
        label,
        rect.center_x(),
        rect.center_y() + 4.5,
        &font(13.0, 600),
        text,
        rect.width() - 16.0,
    );
}

fn draw_icon(canvas: &Canvas, icon_name: &str, x: f32, y: f32, size: f32) {
    if let Some(icon) = named_icon_sized(icon_name, size as i32) {
        let dst = Rect::from_xywh(x, y, size, size);
        let r = size * 0.22;
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
    }
}

/// Apply the frosted-panel surface style to a dialog subsurface: a translucent
/// theme material over a blurred backdrop, rounded clipping, drop shadow, and
/// center anchor. [`draw_dialog`] draws only the content on top.
/// Drop a colour's transparency, keeping its RGB.
fn opaque(c: Color) -> Color {
    Color::from_argb(0xFF, c.r(), c.g(), c.b())
}

/// Raise a colour's alpha to at least `min_alpha`, keeping its RGB.
fn at_least_opaque(c: Color, min_alpha: u8) -> Color {
    Color::from_argb(c.a().max(min_alpha), c.r(), c.g(), c.b())
}

pub fn apply_dialog_style(surface: &SubsurfaceSurface) {
    // `material_medium` is tuned for menus, which are small and sit close to
    // what they belong to. This panel is large, modal, and carries a decision —
    // it needs the backdrop legible behind it but the content unmistakably in
    // front, so it runs a good deal more solid than the shared material.
    let c = at_least_opaque(otto_kit::AppContext::current_theme().material_medium, 0xE0);
    if let Some(ss) = surface.base_surface().surface_style() {
        ss.set_background_color(
            c.r() as f64 / 255.0,
            c.g() as f64 / 255.0,
            c.b() as f64 / 255.0,
            c.a() as f64 / 255.0,
        );
        ss.set_corner_radius(PANEL_RADIUS as f64 * BUFFER_SCALE);
        ss.set_masks_to_bounds(ClipMode::Enabled);
        ss.set_shadow(0.35, 24.0, 0.0, 8.0, 0.0, 0.0, 0.0);
        ss.set_blend_mode(BlendMode::BackgroundBlur);
        ss.set_contents_gravity(ContentsGravity::Center);
        ss.set_anchor_point(0.5, 0.5);
    }
}
