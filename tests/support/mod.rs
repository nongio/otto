//! Helpers shared by the headless tests that draw the scene themselves
//! (`frost_damage`, `titlebar_frost`, `desk_blur`). Each test crate uses
//! only some of them.
#![allow(dead_code)]

use layers::prelude::Layer as SceneLayer;
use layers::skia;
use layers::types::Color;

/// Writes a 64×64 PNG of `colour` to a file of its own and returns its path.
/// The name carries the process id, so concurrent test runs do not overwrite
/// each other's wallpaper.
pub fn wallpaper(name: &str, colour: skia::Color) -> String {
    let mut surface = skia::surfaces::raster_n32_premul((64, 64)).unwrap();
    surface.canvas().clear(colour);
    let data = surface
        .image_snapshot()
        .encode(None, skia::EncodedImageFormat::PNG, 100)
        .unwrap();
    let path = std::env::temp_dir().join(format!("otto-test-{name}-{}.png", std::process::id()));
    std::fs::write(&path, data.as_bytes()).unwrap();
    path.to_string_lossy().into_owned()
}

/// A solid layer of `size` physical px at `at` under `parent`.
pub fn solid(
    state: &otto::state::Otto<otto::headless::HeadlessData>,
    parent: &SceneLayer,
    at: (f32, f32),
    size: (f32, f32),
    colour: Color,
) -> SceneLayer {
    let layer = state.layers_engine.new_layer();
    layer.set_layout_style(layers::taffy::Style {
        position: layers::taffy::Position::Absolute,
        ..Default::default()
    });
    layer.set_position(at, None);
    layer.set_size(layers::types::Size::points(size.0, size.1), None);
    layer.set_background_color(colour, None);
    let _ = parent.add_sublayer(&layer);
    layer
}

/// The image's pixels as unpremultiplied RGBA.
pub fn rgba(image: &skia::Image) -> Vec<u8> {
    let info = skia::ImageInfo::new(
        (image.width(), image.height()),
        skia::ColorType::RGBA8888,
        skia::AlphaType::Unpremul,
        None,
    );
    let mut px = vec![0u8; (image.width() * image.height() * 4) as usize];
    assert!(image.read_pixels(
        &info,
        &mut px,
        (image.width() * 4) as usize,
        (0, 0),
        skia::image::CachingHint::Disallow
    ));
    px
}
