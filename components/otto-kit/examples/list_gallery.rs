//! Offscreen gallery of the grouped-list card and the source-list sidebar.
//!
//! Renders a mock settings window straight to a PNG with Skia's raster
//! backend — no compositor, no Wayland — so the two components can be judged
//! against the settings app's approved look without running it.
//!
//! ```sh
//! cargo run -p otto-kit --example list_gallery -- /tmp/list_gallery.png
//! ```

use otto_kit::components::list::{self, ListLayout, ListRow};
use otto_kit::components::slider::{self, SliderInteraction};
use otto_kit::components::source_list::{self, SourceListItem, SourceListLayout};
use otto_kit::components::toggle::{self, ToggleInteraction};
use otto_kit::prelude::*;
use skia_safe::{surfaces, EncodedImageFormat, PaintStyle, Point, RRect};

const SIDEBAR_W: f32 = 180.0;
const PANE_W: f32 = 380.0;
const PANE_H: f32 = 420.0;
const CELL_W: f32 = SIDEBAR_W + PANE_W + 40.0;
const CELL_H: f32 = PANE_H + 40.0;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "list_gallery.png".to_string());

    let scale = 2.0_f32;
    let cols = 2;
    let width = (CELL_W * cols as f32 * scale) as i32;
    let height = (CELL_H * scale) as i32;

    let mut surface = surfaces::raster_n32_premul((width, height)).expect("raster surface");
    let canvas = surface.canvas();
    canvas.scale((scale, scale));

    for (i, dark) in [false, true].into_iter().enumerate() {
        canvas.save();
        canvas.translate((i as f32 * CELL_W, 0.0));
        draw_cell(canvas, dark);
        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&out, data.as_bytes()).expect("write png");
    println!("wrote {out} ({width}x{height})");
}

fn draw_cell(canvas: &Canvas, dark: bool) {
    let theme = if dark { Theme::dark() } else { Theme::light() };

    let mut bg = Paint::default();
    bg.set_color(if dark {
        Color::from_rgb(0x1E, 0x22, 0x2B)
    } else {
        Color::from_rgb(0x8C, 0x9E, 0xB8)
    });
    canvas.draw_rect(Rect::from_wh(CELL_W, CELL_H), &bg);

    canvas.save();
    canvas.translate((20.0, 20.0));
    draw_window(canvas, &theme, dark);
    canvas.restore();
}

/// Rounded window body containing the sidebar and the pane content.
fn draw_window(canvas: &Canvas, theme: &Theme, dark: bool) {
    // +20 for the trailing margin matching the leading one `draw_pane` is
    // translated by, so a row's right edge lands inside the window.
    let window_w = SIDEBAR_W + PANE_W + 20.0;
    let frame = Rect::from_wh(window_w, PANE_H);
    let rrect = RRect::new_rect_xy(frame, 12.0, 12.0);

    canvas.save();
    canvas.clip_rrect(rrect, skia_safe::ClipOp::Intersect, true);

    let mut body = Paint::default();
    body.set_anti_alias(true);
    body.set_color(if dark {
        Color::from_rgb(0x24, 0x26, 0x2B)
    } else {
        Color::from_rgb(0xFA, 0xFA, 0xFA)
    });
    canvas.draw_rect(frame, &body);

    let mut sidebar_bg = Paint::default();
    sidebar_bg.set_anti_alias(true);
    sidebar_bg.set_color(if dark {
        Color::from_rgb(0x1C, 0x1E, 0x22)
    } else {
        Color::from_rgb(0xEE, 0xEE, 0xF0)
    });
    canvas.draw_rect(Rect::from_wh(SIDEBAR_W, PANE_H), &sidebar_bg);

    draw_sidebar(canvas, theme);

    canvas.save();
    canvas.translate((SIDEBAR_W + 20.0, 20.0));
    draw_pane(canvas, theme, dark);
    canvas.restore();

    let mut divider = Paint::default();
    divider.set_color(theme.fill_tertiary);
    divider.set_stroke_width(1.0);
    canvas.draw_line(
        Point::new(SIDEBAR_W, 0.0),
        Point::new(SIDEBAR_W, PANE_H),
        &divider,
    );

    canvas.restore();

    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(if dark {
        Color::from_argb(0x40, 0xFF, 0xFF, 0xFF)
    } else {
        Color::from_argb(0x33, 0x00, 0x00, 0x00)
    });
    canvas.draw_rrect(
        RRect::new_rect_xy(frame.with_inset((0.5, 0.5)), 12.0, 12.0),
        &edge,
    );
}

fn draw_sidebar(canvas: &Canvas, theme: &Theme) {
    let items = [
        SourceListItem::new("General"),
        SourceListItem::new("Displays"),
        SourceListItem::new("Dock & Menu Bar"),
        SourceListItem::new("Sound"),
    ];
    let layout = SourceListLayout::compute(items.len(), 0.0, 16.0, SIDEBAR_W);

    source_list::draw(
        canvas,
        &layout,
        &items,
        Some(1),
        theme,
        |canvas, index, rect, tint| {
            // Stand-in glyph: a filled circle, since the real icon set lives in
            // otto-settings' own `glyphs` module and this gallery has no
            // business depending on it.
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_color(tint);
            let r = rect.width() / 2.0 - (index as f32 * 0.6).min(3.0);
            canvas.draw_circle(rect.center(), r.max(4.0), &paint);
        },
    );
}

/// Vertically-centred sub-rect of `height` inside `rect` — controls that
/// have their own reference size (toggle, slider) draw at whatever height
/// the rect they are handed is, so a caller centres one of that size inside
/// the row-height slot [`list::trailing_rect`] hands back.
fn center_v(rect: Rect, height: f32) -> Rect {
    Rect::from_xywh(
        rect.left,
        rect.center_y() - height / 2.0,
        rect.width(),
        height,
    )
}

fn draw_pane(canvas: &Canvas, theme: &Theme, dark: bool) {
    let background = list::default_card_background(dark);
    let mut y = 0.0;

    // Card 1: single-line rows, one with a trailing toggle.
    let rows_a = vec![
        ListRow::new("Wi-Fi"),
        ListRow::new("Bluetooth"),
        ListRow::new("AirDrop"),
    ];
    let layout_a = ListLayout::compute(&rows_a, true, 0.0, y, PANE_W);
    list::draw(
        canvas,
        &layout_a,
        &rows_a,
        Some("NETWORK"),
        theme,
        background,
        |canvas, index, rect| {
            // Every row gets a toggle here so the trailing slot is exercised on
            // every row, not just one. `trailing_rect` hands back the row's full
            // height — the control sizes itself, so centre its own rect inside.
            let slot = center_v(list::trailing_rect(rect, toggle::WIDTH), toggle::HEIGHT);
            let on = index != 2;
            toggle::draw(
                canvas,
                slot,
                toggle::knob_fraction_for(on),
                ToggleInteraction::Normal,
                theme,
            );
        },
    );
    y += layout_a.total_height() + 22.0;

    // Card 2: detail rows, with a slider stand-in on the trailing edge of one
    // and a plain value on the other — proving the trailing slot is generic.
    let rows_b = vec![
        ListRow::new("Cursor size").with_detail("Affects the pointer everywhere"),
        ListRow::new("Auto-hide").with_detail("Show the dock on mouse-over"),
    ];
    let layout_b = ListLayout::compute(&rows_b, true, 0.0, y, PANE_W);
    list::draw(
        canvas,
        &layout_b,
        &rows_b,
        Some("DOCK"),
        theme,
        background,
        |canvas, index, rect| {
            if index == 0 {
                // The readout paints past the slider's own rect (see
                // `slider::draw`), so the track sits well clear of the card's
                // trailing edge rather than flush against it like the toggle.
                let readout_w = styles::SUBHEADLINE.font().measure_str("65%", None).0;
                let slot = list::trailing_rect(rect, 90.0);
                let slot = Rect::from_xywh(
                    slot.left - readout_w - 8.0,
                    slot.top,
                    slot.width(),
                    slot.height(),
                );
                let slot = center_v(slot, slider::KNOB_RADIUS * 2.0);
                slider::draw(
                    canvas,
                    slot,
                    0.65,
                    0.0,
                    1.0,
                    Some("65%"),
                    SliderInteraction::Normal,
                    theme,
                );
            } else {
                let slot = center_v(list::trailing_rect(rect, toggle::WIDTH), toggle::HEIGHT);
                toggle::draw(
                    canvas,
                    slot,
                    toggle::knob_fraction_for(true),
                    ToggleInteraction::Normal,
                    theme,
                );
            }
        },
    );
}
