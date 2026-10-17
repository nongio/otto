//! The keyboard shortcuts sheet: every command and its key, on a card over
//! the window, from Help › Keyboard Shortcuts or Ctrl+/. Escape or a click
//! puts it away.

use otto_kit::components::label::Label;
use otto_kit::prelude::*;
use otto_kit::skia::RRect;
use otto_kit::typography::styles;
use otto_kit::TextAlign;

use crate::viewer::Viewer;

/// Room around the card's edge, and between its columns.
const PAD: f32 = 24.0;
/// One shortcut's row.
const ROW_H: f32 = 24.0;
/// A section's title, with the room above it.
const TITLE_H: f32 = 34.0;
/// The sheet's own title, and the note on Cmd under it.
const HEADING_H: f32 = 62.0;
/// How wide a column is at most; two fit side by side when there is room.
const COLUMN_W: f32 = 300.0;

/// Paint the sheet over the window when it is open.
pub fn draw(canvas: &Canvas, viewer: &Viewer, theme: &Theme) {
    if !viewer.shortcuts_open {
        return;
    }
    let (width, height) = viewer.size;
    let mut scrim = Paint::default();
    scrim.set_color(Color::from_argb(0x55, 0, 0, 0));
    canvas.draw_rect(Rect::from_wh(width, height), &scrim);

    let sections = crate::commands::sheet();
    let columns = if width >= 2.0 * COLUMN_W + 3.0 * PAD {
        2
    } else {
        1
    };
    let column_w = COLUMN_W.min(width - 2.0 * PAD - (columns as f32 - 1.0) * PAD);
    // Sections fill the first column to about half, then the second.
    let section_h = |rows: usize| TITLE_H + rows as f32 * ROW_H;
    let total: f32 = sections.iter().map(|(_, rows)| section_h(rows.len())).sum();
    let mut split = sections.len();
    if columns == 2 {
        let mut run = 0.0;
        for (index, (_, rows)) in sections.iter().enumerate() {
            run += section_h(rows.len());
            if run >= total / 2.0 {
                split = index + 1;
                break;
            }
        }
    }
    let column_h = |range: &[(String, Vec<(String, String)>)]| -> f32 {
        range.iter().map(|(_, rows)| section_h(rows.len())).sum()
    };
    let body_h = column_h(&sections[..split]).max(column_h(&sections[split..]));
    let card_w = columns as f32 * column_w + (columns as f32 + 1.0) * PAD;
    let card_h = (HEADING_H + body_h + 2.0 * PAD).min(height - 2.0 * PAD);
    let card = Rect::from_xywh(
        (width - card_w) / 2.0,
        ((height - card_h) / 2.0).max(PAD),
        card_w,
        card_h,
    );

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.card_material());
    canvas.draw_rrect(RRect::new_rect_xy(card, 14.0, 14.0), &paint);

    canvas.save();
    canvas.clip_rect(card, None, true);
    Label::new(otto_kit::t_owned!("studio-shortcuts"))
        .with_style(styles::TITLE_3_EMPHASIZED)
        .with_color(theme.text_primary)
        .with_width(card_w - 2.0 * PAD)
        .centered_on(card.left + PAD, card.top + PAD + HEADING_H / 2.0 - 6.0)
        .render(canvas);
    Label::new(otto_kit::t_owned!("studio-sheet-cmd"))
        .with_style(styles::FOOTNOTE)
        .with_color(theme.text_secondary)
        .with_width(card_w - 2.0 * PAD)
        .centered_on(card.left + PAD, card.top + PAD + HEADING_H - 12.0)
        .render(canvas);
    for (column, range) in [&sections[..split], &sections[split..]]
        .into_iter()
        .enumerate()
    {
        let left = card.left + PAD + column as f32 * (column_w + PAD);
        let mut y = card.top + PAD + HEADING_H;
        for (title, rows) in range {
            Label::new(title.clone())
                .with_style(styles::FOOTNOTE)
                .with_color(theme.text_secondary)
                .with_width(column_w)
                .centered_on(left, y + TITLE_H - 12.0)
                .render(canvas);
            y += TITLE_H;
            for (label, keys) in rows {
                let middle = y + ROW_H / 2.0;
                Label::new(label.clone())
                    .with_style(styles::BODY)
                    .with_color(theme.text_primary)
                    .with_width(column_w * 0.62)
                    .centered_on(left, middle)
                    .render(canvas);
                Label::new(keys.clone())
                    .with_style(styles::BODY)
                    .with_color(theme.text_secondary)
                    .with_width(column_w * 0.38)
                    .with_align(TextAlign::Right)
                    .centered_on(left + column_w * 0.62, middle)
                    .render(canvas);
                y += ROW_H;
            }
        }
    }
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paint the sheet over a window, light and dark, into
    /// `OTTO_RENDER_OUT` (default `.`), to look at.
    #[test]
    #[ignore = "writes PNGs to look at"]
    fn render_the_sheet() {
        let out = std::path::PathBuf::from(
            std::env::var("OTTO_RENDER_OUT").unwrap_or_else(|_| ".".into()),
        );
        for (dark, size) in [(false, (960.0, 720.0)), (true, (560.0, 820.0))] {
            let mut viewer = Viewer::new(std::path::PathBuf::from("/tmp/a.png"), size);
            viewer.shortcuts_open = true;
            let theme = if dark { Theme::dark() } else { Theme::light() };
            let mut surface = otto_kit::skia::surfaces::raster_n32_premul((
                (size.0 * 2.0) as i32,
                (size.1 * 2.0) as i32,
            ))
            .expect("a raster surface");
            let canvas = surface.canvas();
            canvas.scale((2.0, 2.0));
            canvas.clear(Color::from_rgb(0xD8, 0xD8, 0xDC));
            draw(canvas, &viewer, &theme);
            let png = surface
                .image_snapshot()
                .encode(None, otto_kit::skia::EncodedImageFormat::PNG, None)
                .expect("a png");
            let name = if dark {
                "sheet-dark.png"
            } else {
                "sheet-light.png"
            };
            std::fs::write(out.join(name), png.as_bytes()).expect("written");
        }
    }
}
