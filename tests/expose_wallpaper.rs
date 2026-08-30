//! The exposé backdrop must show the wallpaper that is actually set.

#[cfg(feature = "headless")]
mod expose_wallpaper_tests {
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use serial_test::serial;

    /// Paint the plane subtrees in scanout order, the way the udev backend
    /// composites them. `draw_scene` from the scene root alone misses them:
    /// each plane root is parentless.
    fn paint_planes(
        engine: &std::sync::Arc<layers::prelude::Engine>,
        state: &otto::state::Otto<otto::headless::HeadlessData>,
        canvas: &layers::skia::Canvas,
    ) {
        for ows in state.workspaces.output_workspaces.values() {
            for root in [
                &ows.background_plane,
                &ows.windows_plane,
                &ows.expose_layer,
                &ows.overlay_plane,
            ] {
                layers::drawing::draw_scene(canvas, engine.scene(), root.id);
            }
        }
    }

    /// A point on the exposé backdrop, below the selector strip, and a point
    /// inside the first workspace preview in the strip.
    const BACKDROP: (i32, i32) = (100, 600);
    const PREVIEW: (i32, i32) = (746, 115);

    /// Write a solid-colour PNG and return its path.
    fn wallpaper(name: &str, colour: layers::skia::Color) -> String {
        use layers::skia;
        let mut surface = skia::surfaces::raster_n32_premul((64, 64)).unwrap();
        surface.canvas().clear(colour);
        let image = surface.image_snapshot();
        let data = image
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let path = std::env::temp_dir().join(format!("otto-test-{name}.png"));
        std::fs::write(&path, data.as_bytes()).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn pixel(handle: &HeadlessHandle, at: (i32, i32)) -> (u8, u8, u8) {
        use layers::skia;
        handle.query(move |state| {
            let engine = state.layers_engine.clone();
            let mut surface = skia::surfaces::raster_n32_premul((1920, 1080)).unwrap();
            let canvas = surface.canvas();
            canvas.clear(skia::Color::from_argb(255, 0, 0, 0));
            paint_planes(&engine, state, canvas);
            let image = surface.image_snapshot();
            let info = skia::ImageInfo::new(
                (1, 1),
                skia::ColorType::RGBA8888,
                skia::AlphaType::Unpremul,
                None,
            );
            let mut px = [0u8; 4];
            assert!(image.read_pixels(&info, &mut px, 4, at, skia::image::CachingHint::Disallow));
            (px[0], px[1], px[2])
        })
    }

    fn set_wallpaper(handle: &HeadlessHandle, path: &str) {
        handle.set_background(path);
        handle.settle(200);
    }

    #[test]
    #[serial]
    fn the_expose_backdrop_shows_a_wallpaper_set_while_it_was_closed() {
        let red = wallpaper("red", layers::skia::Color::from_argb(255, 255, 0, 0));
        let blue = wallpaper("blue", layers::skia::Color::from_argb(255, 0, 0, 255));

        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.settle(200);

        set_wallpaper(&handle, &red);
        handle.toggle_expose();
        handle.settle(400);
        assert_eq!(pixel(&handle, BACKDROP), (255, 0, 0), "first exposé, red");
        handle.toggle_expose();
        handle.settle(400);

        set_wallpaper(&handle, &blue);
        handle.toggle_expose();
        handle.settle(400);
        assert_eq!(
            pixel(&handle, BACKDROP),
            (0, 0, 255),
            "the exposé backdrop is still showing the old wallpaper"
        );
        assert_eq!(
            pixel(&handle, PREVIEW),
            (0, 0, 255),
            "the workspace preview is still showing the old wallpaper"
        );

        handle.stop();
    }
}
