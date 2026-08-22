//! Render the launcher's card to PNGs, without taking over a screen.
//!
//! The launcher is a fullscreen overlay that grabs the keyboard, which is a
//! hostile thing to start while working on how it looks — and impossible to
//! type into from a script without the keystrokes landing somewhere else if it
//! is not up yet. This draws the same scene into an offscreen raster surface,
//! for a given query:
//!
//! ```sh
//! cargo run -p otto-launcher --example preview -- /tmp/launcher "term"
//! ```
//!
//! The frosted material is the compositor's, so it cannot appear here; the
//! preview paints a flat stand-in behind the scene in its place.

use std::path::PathBuf;

use layers::prelude::Engine;
use otto_kit::components::text_input::TextInput;
use otto_launcher::{field_style, rank, Apps, Item, Palette, Source, CARD_W, MAX_CARD_H};

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/otto-launcher-preview"));
    std::fs::create_dir_all(&out_dir).expect("cannot create the output directory");
    let queries: Vec<String> = args.collect();
    let queries = if queries.is_empty() {
        vec![String::new(), "term".into(), "zzzz".into()]
    } else {
        queries
    };

    let dark = true;
    let engine = Engine::create(CARD_W, MAX_CARD_H);
    let mut palette = Palette::new(engine.clone(), None, dark);
    palette.set_size(1440.0, 930.0);

    let mut apps = Apps::load(0);
    let items: Vec<Item> = apps.items();
    let labels = [apps.label()];

    for query in &queries {
        let mut input = TextInput::editing(query.clone(), field_style(dark));
        input.state.placeholder = "Search apps and windows…".to_string();
        input.set_size(CARD_W, otto_launcher::FIELD_H);

        let matches = rank(&items, query);
        let shown: Vec<&Item> = matches.iter().map(|m| &items[m.index]).collect();
        let empty = (!query.trim().is_empty()).then_some("No results");
        palette.update(&input, &shown, &labels, 0, 0, empty);
        for _ in 0..60 {
            engine.update(0.016);
        }

        let mut surface =
            skia_safe::surfaces::raster_n32_premul((CARD_W as i32, MAX_CARD_H as i32))
                .expect("cannot create the raster surface");
        // Stand in for the compositor's frosted material.
        surface.canvas().clear(if dark {
            skia_safe::Color::from_argb(255, 34, 34, 38)
        } else {
            skia_safe::Color::from_argb(255, 240, 240, 244)
        });
        layers::prelude::draw_scene(surface.canvas(), engine.scene(), palette.card_layer().id());

        let name = if query.is_empty() {
            "empty".to_string()
        } else {
            query.replace(' ', "_")
        };
        let path = out_dir.join(format!("{name}.png"));
        let image = surface.image_snapshot();
        let data = image
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .expect("cannot encode the preview");
        std::fs::write(&path, data.as_bytes()).expect("cannot write the preview");
        println!("{} ({} matches)", path.display(), matches.len());
    }
}
