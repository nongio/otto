//! A frosted window blurs the window behind it, with or without a desk — a
//! transparent, full-output surface in the bottom layer-shell layer — under
//! both of them.
//!
//! Rendered the way the single-plane composite path renders an output (the
//! Vulkan renderer, and any output without plane decomposition): the whole
//! output tree through `SceneElement`, into a framebuffer kept between frames,
//! repainting only what the element reports damaged. That is the path that
//! keeps a blurred backdrop between frames, so a stale frost shows up here.
//!
//! The headless compositor imports no client buffers, so solid layers stand
//! in for what the clients draw: a fill for the window behind, a panel for
//! the desk.

#[cfg(feature = "headless")]
mod desk_blur_tests {
    use layers::prelude::Layer as SceneLayer;
    use layers::types::Color;
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::{KeyboardInteractivity, Layer, TestClient};
    use serial_test::serial;
    use std::cell::RefCell;
    use std::time::Duration;

    const BEHIND: &str = "desk-blur-behind";
    const FRONT: &str = "desk-blur-front";
    const SIZE: (i32, i32) = (1920, 1080);

    thread_local! {
        /// The composite path's framebuffer and the commit it was last drawn
        /// at. Lives on the compositor thread, where the frames are drawn.
        static FRAME: RefCell<Option<(
            layers::skia::Surface,
            Option<smithay::backend::renderer::utils::CommitCounter>,
        )>> = const { RefCell::new(None) };
        /// Stand-in for the content of the window behind.
        static FILL: RefCell<Option<SceneLayer>> = const { RefCell::new(None) };
        /// Stand-in for the desk's content.
        static PANEL: RefCell<Option<SceneLayer>> = const { RefCell::new(None) };
    }

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

    /// A solid layer of `size` physical px at `at` under `parent`.
    fn solid(
        state: &otto::state::Otto<otto::headless::HeadlessData>,
        parent: &SceneLayer,
        key: &str,
        at: (f32, f32),
        size: (f32, f32),
        colour: Color,
    ) -> SceneLayer {
        let layer = state.layers_engine.new_layer();
        layer.set_key(key);
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

    /// Draw one frame of the output the way the udev composite path does and
    /// read back the pixel at `at` (physical px).
    fn composite_frame(handle: &HeadlessHandle, at: (i32, i32)) -> (u8, u8, u8) {
        use layers::skia;
        use smithay::backend::renderer::element::Element;
        handle.query(move |state| {
            if state.scene_element.size == (0.0, 0.0) {
                state.scene_element.set_size(SIZE.0 as f32, SIZE.1 as f32);
            }
            // The tick the udev loop runs before a frame: it turns engine
            // damage into the element's damage.
            state.scene_element.update();
            let ows = state.workspaces.output_workspaces.values().next().unwrap();
            let element = state.scene_element.for_output_layer(&ows.output_layer);
            let output = state.workspaces.outputs().next().unwrap().clone();
            let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());
            FRAME.with(|frame| {
                let mut frame = frame.borrow_mut();
                let (surface, last) = frame.get_or_insert_with(|| {
                    (skia::surfaces::raster_n32_premul(SIZE).unwrap(), None)
                });
                let damage = element.damage_since(scale, *last);
                *last = Some(element.current_commit());
                let dst = smithay::utils::Rectangle::new((0, 0).into(), SIZE.into());
                element.draw_scene(surface.canvas(), dst, &damage);
                let image = surface.image_snapshot();
                let info = skia::ImageInfo::new(
                    (1, 1),
                    skia::ColorType::RGBA8888,
                    skia::AlphaType::Unpremul,
                    None,
                );
                let mut px = [0u8; 4];
                assert!(image.read_pixels(
                    &info,
                    &mut px,
                    4,
                    at,
                    skia::image::CachingHint::Disallow
                ));
                (px[0], px[1], px[2])
            })
        })
    }

    fn set_fill(handle: &HeadlessHandle, colour: Color) {
        handle.with_state(move |_| {
            FILL.with(|f| {
                f.borrow()
                    .as_ref()
                    .unwrap()
                    .set_background_color(colour, None)
            });
        });
    }

    fn set_panel(handle: &HeadlessHandle, colour: Color) {
        handle.with_state(move |_| {
            PANEL.with(|p| {
                if let Some(panel) = p.borrow().as_ref() {
                    panel.set_background_color(colour, None);
                }
            });
        });
    }

    /// The frost over the window behind, through a run of frames in which
    /// the desk repaints and the window behind changes colour.
    fn frost_over(with_desk: bool) {
        let red = wallpaper(
            "desk-blur-red",
            layers::skia::Color::from_argb(255, 255, 0, 0),
        );
        let handle = HeadlessHandle::start(HeadlessConfig::default());
        handle.settle(200);
        handle.set_background(&red);
        handle.settle(200);

        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let desk = with_desk.then(|| {
            client.create_layer_surface(
                "otto-desk",
                Layer::Bottom,
                SIZE.0 as u32,
                SIZE.1 as u32,
                KeyboardInteractivity::None,
            )
        });
        let _behind = client.create_toplevel(BEHIND, 1200, 800);
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(BEHIND, 0, 0);
        handle.settle(200);

        let front = client.create_toplevel(FRONT, 400, 400);
        let _ = client.roundtrip();
        handle.settle(300);
        let front_surface = front.lock().unwrap().surface.clone();
        let _style = client.request_material(&front_surface);
        front.lock().unwrap().commit_frame();
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(FRONT, 200, 200);
        handle.settle(200);

        handle.with_state(move |state| {
            let window = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == BEHIND)
                .cloned()
                .expect("the window behind is mapped");
            let fill = solid(
                state,
                window.base_layer(),
                "desk-blur-fill",
                (0.0, 0.0),
                (2400.0, 1600.0),
                Color::new_rgba255(0, 0, 255, 255),
            );
            FILL.with(|f| *f.borrow_mut() = Some(fill));
            if with_desk {
                let ows = state.workspaces.output_workspaces.values().next().unwrap();
                let desk = ows
                    .layer_shell_bottom
                    .children()
                    .into_iter()
                    .next()
                    .expect("the desk is in the bottom layer");
                // Under the frosted window, where a desk panel would be.
                let panel = solid(
                    state,
                    &desk,
                    "desk-blur-panel",
                    (300.0, 300.0),
                    (1200.0, 600.0),
                    Color::new_rgba255(255, 255, 0, 255),
                );
                PANEL.with(|p| *p.borrow_mut() = Some(panel));
            }
        });
        handle.settle(100);

        let scale = handle.output_scale();
        let at = ((400.0 * scale) as i32, (400.0 * scale) as i32);
        let blueish = |(r, g, b): (u8, u8, u8)| b > r && b > g;
        let greenish = |(r, g, b): (u8, u8, u8)| g > r && g > b;

        let px = composite_frame(&handle, at);
        assert!(
            blueish(px),
            "first frame: the frost shows {px:?}, not the window behind"
        );

        // The desk repaints under everything: the frost does not change.
        for (i, colour) in [(0, 255, 255), (255, 0, 255), (255, 255, 0)]
            .iter()
            .enumerate()
        {
            set_panel(
                &handle,
                Color::new_rgba255(colour.0, colour.1, colour.2, 255),
            );
            if let Some(desk) = desk.as_ref() {
                desk.lock().unwrap().commit_frame();
            }
            let _ = client.roundtrip();
            handle.wait(Duration::from_millis(20));
            let px = composite_frame(&handle, at);
            assert!(blueish(px), "desk repaint {i}: the frost shows {px:?}");
        }

        // The window behind changes: the frost follows it, desk repaints or not.
        set_fill(&handle, Color::new_rgba255(0, 255, 0, 255));
        let px = composite_frame(&handle, at);
        assert!(
            greenish(px),
            "the frost kept {px:?} after the window behind changed"
        );
        for i in 0..3 {
            set_panel(&handle, Color::new_rgba255(255, 0, 0, 255));
            let px = composite_frame(&handle, at);
            assert!(
                greenish(px),
                "desk repaint {i} after the change: the frost shows {px:?}"
            );
        }

        handle.stop();
    }

    #[test]
    #[serial]
    fn a_frosted_window_blurs_the_window_behind_it() {
        frost_over(false);
    }

    #[test]
    #[serial]
    fn a_frosted_window_blurs_the_window_behind_it_over_the_desk() {
        frost_over(true);
    }
}
