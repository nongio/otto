use super::{MenuItem, MenuItemIcon, MenuItemKind, MenuItemStyle, MenuSlider, VisualState};

use crate::components::slider::{self, SliderInteraction, KNOB_RADIUS};
use crate::{components::icon::Icon, Renderable};
use skia_safe::{Canvas, Font, Paint, Point, RRect, Rect};

/// Room before a toggle's label for its check.
pub const CHECK_GUTTER: f32 = 16.0;

/// The round toggle of a slider row.
const TOGGLE_DIAMETER: f32 = 24.0;
/// The glyph inside the toggle.
const TOGGLE_ICON: f32 = 14.0;
/// The column a slider row's name is drawn in; a longer name is cut short.
const SLIDER_LABEL_WIDTH: f32 = 76.0;
/// The column the percentage is drawn in, right-aligned: room for "100%".
const READOUT_WIDTH: f32 = 36.0;
/// The space between the parts of a slider row.
const SLIDER_GAP: f32 = 10.0;

/// Where the parts of a slider row sit, in the row's coordinates. Drawing
/// and hit-testing both come through [`slider_layout`], so a press lands on
/// what is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderLayout {
    /// The round toggle, when the row has one.
    pub toggle: Option<Rect>,
    /// The name's column, when the row has a name.
    pub label: Option<Rect>,
    /// The track. The knob's centre stays inside it.
    pub track: Rect,
    /// The percentage's column.
    pub readout: Rect,
}

/// Lay out the slider row at `(x, y)`, `width` by `height`.
pub fn slider_layout(
    slider: &MenuSlider,
    style: &MenuItemStyle,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> SliderLayout {
    let cy = y + height / 2.0;
    let mut left = x + style.horizontal_padding;
    let toggle = slider.toggle.as_ref().map(|_| {
        let rect = Rect::from_xywh(
            left,
            cy - TOGGLE_DIAMETER / 2.0,
            TOGGLE_DIAMETER,
            TOGGLE_DIAMETER,
        );
        left += TOGGLE_DIAMETER + SLIDER_GAP;
        rect
    });
    let label = slider.label.as_ref().map(|_| {
        let rect = Rect::from_xywh(left, y, SLIDER_LABEL_WIDTH, height);
        left += SLIDER_LABEL_WIDTH + SLIDER_GAP;
        rect
    });
    let right = x + width - style.horizontal_padding;
    let readout = Rect::from_xywh(right - READOUT_WIDTH, y, READOUT_WIDTH, height);
    let track_left = left + KNOB_RADIUS;
    let track_right = (readout.left - SLIDER_GAP - KNOB_RADIUS).max(track_left);
    let track = Rect::from_ltrb(track_left, y, track_right, y + height);
    SliderLayout {
        toggle,
        label,
        track,
        readout,
    }
}

/// A value from 0.0 to 1.0 as the percentage a slider row shows.
pub fn slider_readout(value: f32) -> String {
    crate::t_owned!(
        "menu-slider-percent",
        percent = (value.clamp(0.0, 1.0) * 100.0).round() as f64
    )
}

/// Pure rendering functions for MenuItem
pub struct MenuItemRenderer;

impl MenuItemRenderer {
    /// Render a menu item at the given position
    pub fn render(
        canvas: &Canvas,
        data: &MenuItem,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
    ) {
        match &data.kind {
            MenuItemKind::Separator => Self::draw_separator(canvas, style, x, y, width),
            MenuItemKind::Slider(slider) => {
                Self::draw_slider(canvas, data, slider, style, x, y, width);
            }
            MenuItemKind::Header { label } => {
                Self::draw_header(canvas, label, style, x, y, data.height);
            }
            _ => Self::draw_item(canvas, data, style, x, y, width),
        }
    }

    /// Draw a separator line
    fn draw_separator(canvas: &Canvas, style: &MenuItemStyle, x: f32, y: f32, width: f32) {
        let mut paint = Paint::default();
        paint.set_color(style.separator_color);
        paint.set_anti_alias(true);
        paint.set_stroke_width(1.0);

        let line_y = y + style.separator_height / 2.0;
        canvas.draw_line(
            Point::new(x + style.horizontal_padding, line_y),
            Point::new(x + width - style.horizontal_padding, line_y),
            &paint,
        );
    }

    /// A caption over a group: small, emphasised and in the secondary tone,
    /// set low in its row so it sits on the rows it names.
    fn draw_header(
        canvas: &Canvas,
        label: &str,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        height: f32,
    ) {
        let font = crate::typography::styles::CAPTION_1_EMPHASIZED.font();
        let mut paint = Paint::default();
        paint.set_color(style.theme.text_secondary);
        paint.set_anti_alias(true);
        let (_, metrics) = font.metrics();
        let baseline = y + height - metrics.descent - 3.0;
        crate::typography::draw_runs(
            canvas,
            label,
            Point::new(x + style.horizontal_padding, baseline),
            &font,
            &paint,
        );
    }

    /// A slider row: the toggle, the name, the track and the percentage.
    #[allow(clippy::too_many_arguments)]
    fn draw_slider(
        canvas: &Canvas,
        data: &MenuItem,
        slider: &MenuSlider,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
    ) {
        let theme = &style.theme;
        let layout = slider_layout(slider, style, x, y, width, data.height);
        let enabled = data.is_enabled();

        // The keyboard is on this row: a quiet band, since the accent pill an
        // action wears would swallow the accent track.
        if data.visual_state() == VisualState::Hovered {
            let mut band = Paint::default();
            band.set_color(theme.fill_secondary);
            band.set_anti_alias(true);
            canvas.draw_rrect(
                RRect::new_rect_xy(
                    Rect::from_xywh(x, y, width, data.height),
                    style.border_radius,
                    style.border_radius,
                ),
                &band,
            );
        }

        if let (Some(rect), Some(toggle)) = (layout.toggle, slider.toggle.as_ref()) {
            let lit = toggle.on && enabled;
            let mut fill = Paint::default();
            fill.set_anti_alias(true);
            fill.set_color(if lit {
                theme.accent
            } else {
                theme.fill_secondary
            });
            canvas.draw_circle(rect.center(), TOGGLE_DIAMETER / 2.0, &fill);
            let tint = if lit {
                skia_safe::Color::WHITE
            } else {
                style.text_color_normal
            };
            Self::draw_icon_at(
                canvas,
                &toggle.icon,
                tint,
                rect.center_x() - TOGGLE_ICON / 2.0,
                rect.center_y() - TOGGLE_ICON / 2.0,
                TOGGLE_ICON,
            );
        }

        let text_color = if enabled {
            style.text_color_normal
        } else {
            style.text_color_disabled
        };
        if let (Some(rect), Some(label)) = (layout.label, slider.label.as_deref()) {
            let font = style.font();
            let fitted = crate::typography::ellipsize_by(label, rect.width(), |piece| {
                crate::typography::measure_runs(&font, piece)
            });
            Self::draw_label(
                canvas,
                &fitted,
                &font,
                text_color,
                rect.left - style.horizontal_padding,
                y,
                data.height,
                style,
                0.0,
            );
        }

        let interaction = if enabled {
            SliderInteraction::Normal
        } else {
            SliderInteraction::Disabled
        };
        slider::draw(
            canvas,
            layout.track,
            slider.value,
            0.0,
            1.0,
            None,
            interaction,
            theme,
        );

        let font = style.shortcut_font();
        let readout = slider_readout(slider.value);
        let (readout_width, _) = font.measure_str(&readout, None);
        let (_, metrics) = font.metrics();
        let baseline =
            y + (data.height - (metrics.descent - metrics.ascent)) / 2.0 - metrics.ascent;
        let mut paint = Paint::default();
        paint.set_color(style.shortcut_color_normal);
        paint.set_anti_alias(true);
        canvas.draw_str(
            &readout,
            Point::new(layout.readout.right - readout_width, baseline),
            &font,
            &paint,
        );
    }

    /// Draw an action or submenu item
    fn draw_item(
        canvas: &Canvas,
        data: &MenuItem,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
    ) {
        let label = data.label().unwrap_or("");

        // Draw background if hovered
        if data.visual_state() == VisualState::Hovered {
            Self::draw_hover_background(canvas, style, x, y, width, data.height);
        }

        // Get text color based on state
        let text_color = match data.visual_state() {
            VisualState::Normal => style.text_color_normal,
            VisualState::Hovered => style.text_color_hovered,
            VisualState::Disabled => style.text_color_disabled,
        };

        // Create font
        let font = style.font();
        let shortcut_font = style.shortcut_font();

        // Draw label (offset by icon if present)
        let icon_size = 16.0;
        let icon_gap = 6.0;
        let gutter = if data.check_gutter { CHECK_GUTTER } else { 0.0 };
        let label_x_offset = gutter
            + if data.icon.is_some() {
                icon_size + icon_gap
            } else {
                0.0
            };
        if data.checked == Some(true) {
            Self::draw_check(
                canvas,
                text_color,
                x + style.horizontal_padding,
                y,
                data.height,
            );
        }
        Self::draw_label(
            canvas,
            label,
            &font,
            text_color,
            x,
            y,
            data.height,
            style,
            label_x_offset,
        );

        // Draw icon before label
        if let Some(icon) = &data.icon {
            Self::draw_icon(
                canvas,
                icon,
                text_color,
                x + gutter,
                y,
                data.height,
                style,
                icon_size,
            );
        }

        // Draw submenu indicator or shortcut
        if data.has_submenu() {
            Self::draw_submenu_indicator(canvas, text_color, x, y, width, data.height, style);
        } else if let Some(shortcut_text) = data.shortcut() {
            Self::draw_shortcut(
                canvas,
                shortcut_text,
                &shortcut_font,
                data.visual_state(),
                x,
                y,
                width,
                data.height,
                style,
            );
        }
    }

    /// A check mark in the gutter at `left`, for a toggle that is on.
    fn draw_check(canvas: &Canvas, color: skia_safe::Color, left: f32, y: f32, height: f32) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Stroke);
        paint.set_stroke_width(1.6);
        paint.set_stroke_cap(skia_safe::paint::Cap::Round);
        paint.set_stroke_join(skia_safe::paint::Join::Round);
        let mid = y + height / 2.0;
        let mut path = skia_safe::PathBuilder::new();
        path.move_to((left + 1.5, mid));
        path.line_to((left + 4.5, mid + 3.0));
        path.line_to((left + 10.0, mid - 3.5));
        canvas.draw_path(&path.detach(), &paint);
    }

    /// Draw hover background
    fn draw_hover_background(
        canvas: &Canvas,
        style: &MenuItemStyle,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        let mut bg_paint = Paint::default();
        bg_paint.set_color(style.bg_color_hovered);
        bg_paint.set_anti_alias(true);

        let bg_rect = RRect::new_rect_xy(
            Rect::from_xywh(x, y, width, height),
            style.border_radius,
            style.border_radius,
        );
        canvas.draw_rrect(bg_rect, &bg_paint);
    }

    /// Draw item label
    #[allow(clippy::too_many_arguments)]
    fn draw_label(
        canvas: &Canvas,
        label: &str,
        font: &Font,
        color: skia_safe::Color,
        x: f32,
        y: f32,
        height: f32,
        style: &MenuItemStyle,
        extra_x_offset: f32,
    ) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.set_anti_alias(true);

        // Center text vertically using font metrics
        let (_line_spacing, metrics) = font.metrics();
        let font_height = metrics.descent - metrics.ascent;
        let baseline_y = y + (height - font_height) / 2.0 - metrics.ascent;

        // A menu is where the interface most often has to draw a script its
        // own font has none of — a language picker names every language in its
        // own — so the label is drawn run by run, each in a face that has its
        // characters. See [`crate::typography::text_runs`].
        crate::typography::draw_runs(
            canvas,
            label,
            Point::new(x + style.horizontal_padding + extra_x_offset, baseline_y),
            font,
            &paint,
        );
    }

    /// Draw icon before label
    #[allow(clippy::too_many_arguments)]
    fn draw_icon(
        canvas: &Canvas,
        icon: &MenuItemIcon,
        tint: skia_safe::Color,
        x: f32,
        y: f32,
        height: f32,
        style: &MenuItemStyle,
        icon_size: f32,
    ) {
        let icon_x = x + style.horizontal_padding;
        let icon_y = y + (height - icon_size) / 2.0;
        Self::draw_icon_at(canvas, icon, tint, icon_x, icon_y, icon_size);
    }

    /// Draw `icon` in an `icon_size` square at `(icon_x, icon_y)`.
    fn draw_icon_at(
        canvas: &Canvas,
        icon: &MenuItemIcon,
        tint: skia_safe::Color,
        icon_x: f32,
        icon_y: f32,
        icon_size: f32,
    ) {
        match icon {
            MenuItemIcon::Named(name) => {
                tracing::trace!(
                    "menu_item render: named icon {:?} at ({}, {})",
                    name,
                    icon_x,
                    icon_y
                );
                // Otto's Solar glyph in the item's tint, else the XDG theme
                // (same as tray icons), else the embedded SVG set. Loaded at
                // physical size for HiDPI crispness.
                let scale = crate::app_runner::context::AppContext::scale_factor().max(1);
                let load_size = (icon_size as i32) * scale;
                let image = crate::icons::solar_icon(
                    name,
                    load_size,
                    crate::icons::SolarStyle::Linear,
                    tint,
                )
                .or_else(|| crate::icons::named_icon_sized(name, load_size));
                if let Some(img) = image {
                    let dst = Rect::from_xywh(icon_x, icon_y, icon_size, icon_size);
                    let paint = Paint::default();
                    let dst_px = (icon_size * scale as f32, icon_size * scale as f32);
                    canvas.draw_image_rect_with_sampling_options(
                        &img,
                        None,
                        dst,
                        crate::utils::icon_sampling((img.width(), img.height()), dst_px),
                        &paint,
                    );
                } else {
                    // Fall back to embedded SVG icon set
                    canvas.save();
                    canvas.translate((icon_x, icon_y));
                    Icon::new(name.as_str())
                        .with_size(icon_size)
                        .with_color(tint)
                        .render(canvas);
                    canvas.restore();
                }
            }
            MenuItemIcon::Themed(names) => {
                let scale = crate::app_runner::context::AppContext::scale_factor().max(1);
                let load_size = (icon_size as i32) * scale;
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                if let Some(img) = crate::icons::cached_icon_chain_at(
                    &names,
                    load_size,
                    crate::icons::FULL_COLOUR_SIZE,
                ) {
                    let dst = Rect::from_xywh(icon_x, icon_y, icon_size, icon_size);
                    let dst_px = (icon_size * scale as f32, icon_size * scale as f32);
                    canvas.draw_image_rect_with_sampling_options(
                        &img,
                        None,
                        dst,
                        crate::utils::icon_sampling((img.width(), img.height()), dst_px),
                        &Paint::default(),
                    );
                }
            }
            MenuItemIcon::Pixmap {
                data,
                width,
                height: h,
            } => {
                if *width <= 0 || *h <= 0 {
                    return;
                }
                // Convert ARGB32 big-endian network order → native BGRA8888
                let mut native = data.clone();
                for chunk in native.chunks_exact_mut(4) {
                    let a = chunk[0];
                    let r = chunk[1];
                    let g = chunk[2];
                    let b = chunk[3];
                    chunk[0] = b;
                    chunk[1] = g;
                    chunk[2] = r;
                    chunk[3] = a;
                }
                let info = skia_safe::ImageInfo::new(
                    (*width, *h),
                    skia_safe::ColorType::BGRA8888,
                    skia_safe::AlphaType::Premul,
                    None,
                );
                let row_bytes = (*width as usize) * 4;
                tracing::debug!(
                    "menu_item render: pixmap icon {}x{} ({} bytes) at ({}, {})",
                    width,
                    h,
                    data.len(),
                    icon_x,
                    icon_y
                );
                if let Some(img) = skia_safe::images::raster_from_data(
                    &info,
                    skia_safe::Data::new_copy(&native),
                    row_bytes,
                ) {
                    let dst = Rect::from_xywh(icon_x, icon_y, icon_size, icon_size);
                    let mut paint = Paint::default();
                    paint.set_anti_alias(true);
                    let scale = crate::app_runner::context::AppContext::scale_factor().max(1);
                    let dst_px = (icon_size * scale as f32, icon_size * scale as f32);
                    canvas.draw_image_rect_with_sampling_options(
                        &img,
                        None,
                        dst,
                        crate::utils::icon_sampling((img.width(), img.height()), dst_px),
                        &paint,
                    );
                } else {
                    tracing::warn!(
                        "menu_item render: failed to create skia image from pixmap {}x{}",
                        width,
                        h
                    );
                }
            }
        }
    }

    /// Draw submenu indicator (chevron)
    fn draw_submenu_indicator(
        canvas: &Canvas,
        color: skia_safe::Color,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        style: &MenuItemStyle,
    ) {
        let icon_size = 16.0;
        let icon_y = y + (height - icon_size) / 2.0;
        let icon_x = x + width - icon_size - style.horizontal_padding;

        canvas.save();
        canvas.translate((icon_x, icon_y));
        Icon::new("chevron-right")
            .with_size(icon_size)
            .with_color(color)
            .render(canvas);
        canvas.restore();
    }

    /// Draw shortcut text
    #[allow(clippy::too_many_arguments)]
    fn draw_shortcut(
        canvas: &Canvas,
        shortcut: &str,
        font: &Font,
        visual_state: VisualState,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        style: &MenuItemStyle,
    ) {
        let shortcut_color = match visual_state {
            VisualState::Hovered => style.shortcut_color_hovered,
            _ => style.shortcut_color_normal,
        };

        let mut paint = Paint::default();
        paint.set_color(shortcut_color);
        paint.set_anti_alias(true);

        let (shortcut_width, _) = font.measure_str(shortcut, None);

        let (_line_spacing, metrics) = font.metrics();
        let font_height = metrics.descent - metrics.ascent;
        let baseline_y = y + (height - font_height) / 2.0 - metrics.ascent;

        canvas.draw_str(
            shortcut,
            Point::new(
                x + width - shortcut_width - style.horizontal_padding,
                baseline_y,
            ),
            font,
            &paint,
        );
    }
}
