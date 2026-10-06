//! Offscreen render of every pane to a PNG.
//!
//! Useful for iterating on layout without a compositor session, and for
//! comparing light against dark side by side.

use skia_safe::{surfaces, Color, EncodedImageFormat, Paint, Rect};

use otto_kit::components::item_list::rows::RowIcons;
use otto_kit::theme::Theme;

use crate::model;
use crate::sidebar_search::{self, FieldView, Search};
use crate::view::{self, Settings, WINDOW_H, WINDOW_W};
use crate::widgets;

const MARGIN: f32 = 40.0;
const CAPTION_H: f32 = 26.0;
const COLS: usize = 2;

/// `out` defaults to `otto-settings.png`; `only` limits it to one pane by name.
/// `query` draws the sidebar's search field holding it, with the list of what
/// it finds dropped over the window.
pub fn render_to_png(out: Option<&String>, only: Option<&String>, query: Option<&String>) {
    let out = out
        .cloned()
        .unwrap_or_else(|| "otto-settings.png".to_string());

    let mut cells: Vec<(String, Settings, Option<Search>)> = Vec::new();
    for (i, pane) in model::panes().iter().enumerate() {
        if let Some(only) = only {
            if !pane.name.eq_ignore_ascii_case(only) {
                continue;
            }
        }
        for dark in [false, true] {
            let scheme = if dark { "dark" } else { "light" };
            let mut settings = Settings::new(i, dark);
            let search = query.map(|query| {
                let mut search = Search::new(dark);
                search.input.set_value(query.clone());
                search.input.state.set_focused(true);
                search.refresh(&model::panes());
                settings = Settings::new(i, dark).with_search_field(Some(FieldView {
                    input: search.input.clone(),
                    focused: true,
                }));
                search
            });
            cells.push((format!("{} · {scheme}", pane.name), settings, search));
        }
    }

    if cells.is_empty() {
        eprintln!("no pane matched; known panes:");
        for pane in model::panes() {
            eprintln!("  {}", pane.name);
        }
        std::process::exit(1);
    }

    let cell_w = WINDOW_W + MARGIN * 2.0;
    let cell_h = WINDOW_H + MARGIN * 2.0 + CAPTION_H;
    let cols = COLS.min(cells.len());
    let rows = cells.len().div_ceil(cols);
    let width = (cell_w * cols as f32) as i32;
    let height = (cell_h * rows as f32) as i32;

    let mut surface = surfaces::raster_n32_premul((width, height)).expect("raster surface");
    let canvas = surface.canvas();

    let icons = RowIcons::default();
    for (i, (caption, settings, search)) in cells.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        canvas.save();
        canvas.translate((col as f32 * cell_w, row as f32 * cell_h));

        // A flat wallpaper stand-in so the window edge and shadow are judged
        // against something realistic.
        let mut bg = Paint::default();
        bg.set_color(if settings.dark {
            Color::from_rgb(0x1B, 0x1F, 0x27)
        } else {
            Color::from_rgb(0x7E, 0x91, 0xAD)
        });
        canvas.draw_rect(Rect::from_wh(cell_w, cell_h), &bg);

        widgets::text_centered_y(
            canvas,
            caption,
            MARGIN,
            MARGIN / 2.0,
            otto_kit::typography::styles::CAPTION_1,
            Color::from_argb(0xD0, 0xFF, 0xFF, 0xFF),
        );

        view::render_on_desktop(canvas, settings, MARGIN, MARGIN + CAPTION_H);
        if let Some(search) = search {
            let theme = if settings.dark {
                Theme::dark()
            } else {
                Theme::light()
            };
            canvas.save();
            canvas.translate((MARGIN, MARGIN + CAPTION_H));
            sidebar_search::paint_panel(
                canvas,
                (settings.width, settings.height),
                settings.dark,
                &theme,
                search,
                &icons,
            );
            canvas.restore();
        }
        canvas.restore();
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write(&out, data.as_bytes()).expect("write png");
    println!("wrote {out} ({width}x{height})");
}
