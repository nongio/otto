//! Render the picker's card to PNGs, without taking over a screen.
//!
//! The picker is a fullscreen overlay that grabs the keyboard, which is a
//! hostile thing to start while working on how it looks. This draws the same
//! scene into an offscreen raster surface, for a given query:
//!
//! ```sh
//! cargo run -p otto-emoji --example preview -- /tmp/emoji "" cat
//! ```
//!
//! The frosted material is the compositor's, so it cannot appear here; the
//! preview paints a flat stand-in behind the scene in its place.

use std::path::PathBuf;

use layers::prelude::Engine;
use otto_emoji::data::{Table, Tone, GROUPS};
use otto_emoji::view::{field_style, Layout, Pane, FIELD_H};
use otto_emoji::{rank, Cell, Palette, CARD_H, CARD_W};
use otto_kit::components::text_input::TextInput;

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/otto-emoji-preview"));
    std::fs::create_dir_all(&out_dir).expect("cannot create the output directory");
    let queries: Vec<String> = args.collect();
    let queries = if queries.is_empty() {
        vec![String::new(), "cat".into(), "zzzz".into()]
    } else {
        queries
    };

    let dark = true;
    let scale = 2.0;
    let engine = Engine::create(CARD_W, CARD_H);
    let mut palette = Palette::new(engine.clone(), None, dark, scale);
    palette.set_size(1440.0, 930.0);

    let mut table = Table::load();
    table.retain_drawable(|emoji| palette.can_draw(emoji.first_codepoint()));

    for query in &queries {
        let mut input = TextInput::editing(query.clone(), field_style(dark));
        input.state.placeholder = "Search emoji…".to_string();
        input.set_size(CARD_W, FIELD_H);

        let mut cells = Vec::new();
        let mut panes = Vec::new();
        if query.trim().is_empty() {
            for (group_index, _) in GROUPS.iter().enumerate() {
                let first = cells.len();
                for index in table.in_group(group_index) {
                    cells.push(Cell {
                        emoji: index,
                        text: table.emoji[index].with_tone(Tone::None).to_string(),
                    });
                }
                panes.push(Pane {
                    tab: group_index + 1,
                    first,
                    count: cells.len() - first,
                });
            }
        } else {
            for index in rank(&table.emoji, query) {
                cells.push(Cell {
                    emoji: index,
                    text: table.emoji[index].text.clone(),
                });
            }
            panes.push(Pane {
                tab: 0,
                first: 0,
                count: cells.len(),
            });
        }
        let layout = Layout::new(panes);
        let searching = !query.trim().is_empty();
        let selected = (!cells.is_empty()).then_some(3.min(cells.len().saturating_sub(1)));
        let empty = (searching && cells.is_empty()).then_some("No emoji found");
        let name = selected
            .map(|index| table.emoji[cells[index].emoji].name)
            .unwrap_or("");

        palette.update_field(&input);
        palette.update_tabs((!searching).then_some(1));
        let scrolls = vec![0.0_f32; layout.pane_count()];
        // OTTO_EMOJI_PAN lets the preview sit mid-pan, to check that panes
        // really do sit side by side rather than overlapping.
        let pan: f32 = std::env::var("OTTO_EMOJI_PAN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        palette.update_grid(&cells, &layout, pan * CARD_W, &scrolls, selected, empty);
        palette.update_footer(name, Tone::Medium);
        for _ in 0..60 {
            engine.update(0.016);
        }

        let mut surface = skia_safe::surfaces::raster_n32_premul((
            (CARD_W * scale) as i32,
            (CARD_H * scale) as i32,
        ))
        .expect("cannot create the raster surface");
        surface.canvas().scale((scale, scale));
        surface.canvas().clear(if dark {
            skia_safe::Color::from_argb(255, 34, 34, 38)
        } else {
            skia_safe::Color::from_argb(255, 240, 240, 244)
        });
        layers::prelude::draw_scene(surface.canvas(), engine.scene(), palette.card_layer().id());

        let file = if query.is_empty() {
            "empty".to_string()
        } else {
            query.replace(' ', "_")
        };
        let path = out_dir.join(format!("{file}.png"));
        let image = surface.image_snapshot();
        let data = image
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .expect("cannot encode the preview");
        std::fs::write(&path, data.as_bytes()).expect("cannot write the preview");
        println!("{} ({} cells)", path.display(), cells.len());
    }
}
