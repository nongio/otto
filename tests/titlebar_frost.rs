//! A focused window's frosted titlebar stays right while what is behind it
//! keeps changing.
//!
//! Rendered the way the single-plane composite path renders an output (the
//! Vulkan renderer): the whole output tree through `SceneElement`, into a
//! framebuffer kept between frames, repainting only what the element reports
//! damaged. That path keeps the titlebar's blurred backdrop between frames
//! and replays it in a pass that covers only part of the bar; a pass that
//! instead blurs part of the bar afresh leaves a seam.
//!
//! Each frame is compared, over the titlebar, against the same scene drawn
//! whole on a thread of its own, whose blur cache starts empty — so its frost
//! is blurred from scratch.
//!
//! The headless compositor imports no client buffers, so solid layers stand
//! in for what the window behind draws.

#[cfg(feature = "headless")]
mod titlebar_frost_tests {
    use layers::prelude::Layer as SceneLayer;
    use layers::skia;
    use layers::types::Color;
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use std::cell::RefCell;

    const BEHIND: &str = "titlebar-frost-behind";
    const FRONT: &str = "titlebar-frost-front";
    const SIZE: (i32, i32) = (1920, 1080);

    thread_local! {
        /// The composite path's framebuffer and the commit it was last drawn
        /// at. Lives on the compositor thread, where the frames are drawn.
        static FRAME: RefCell<Option<(
            skia::Surface,
            Option<smithay::backend::renderer::utils::CommitCounter>,
        )>> = const { RefCell::new(None) };
        /// What changes behind the titlebar, frame after frame.
        static TICK: RefCell<Option<SceneLayer>> = const { RefCell::new(None) };
    }

    fn wallpaper(name: &str, colour: skia::Color) -> String {
        let mut surface = skia::surfaces::raster_n32_premul((64, 64)).unwrap();
        surface.canvas().clear(colour);
        let data = surface
            .image_snapshot()
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let path = std::env::temp_dir().join(format!("otto-test-{name}.png"));
        std::fs::write(&path, data.as_bytes()).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn solid(
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

    fn rgba(image: &skia::Image) -> Vec<u8> {
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

    /// One frame drawn the way the udev composite path draws it, and the
    /// same scene drawn whole from scratch on another thread, both as RGBA,
    /// plus the titlebar's bounds in physical px.
    fn frame_and_reference(handle: &HeadlessHandle) -> (Vec<u8>, Vec<u8>, skia::IRect) {
        use smithay::backend::renderer::element::Element;
        handle.query(move |state| {
            if state.scene_element.size == (0.0, 0.0) {
                state.scene_element.set_size(SIZE.0 as f32, SIZE.1 as f32);
            }
            state.scene_element.update();
            let ows = state.workspaces.output_workspaces.values().next().unwrap();
            let output_layer = ows.output_layer.clone();
            let element = state.scene_element.for_output_layer(&output_layer);
            let output = state.workspaces.outputs().next().unwrap().clone();
            let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());
            let window = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == FRONT)
                .cloned()
                .expect("the front window is mapped");
            let bar = state
                .workspaces
                .get_window_view(&window.id())
                .expect("the front window has a view")
                .decoration_layer
                .render_bounds_transformed();
            let bar: skia::IRect = skia::RoundOut::round_out(&bar);

            let frame = FRAME.with(|frame| {
                let mut frame = frame.borrow_mut();
                let (surface, last) = frame.get_or_insert_with(|| {
                    (skia::surfaces::raster_n32_premul(SIZE).unwrap(), None)
                });
                let damage = element.damage_since(scale, *last);
                *last = Some(element.current_commit());
                let dst = smithay::utils::Rectangle::new((0, 0).into(), SIZE.into());
                element.draw_scene(surface.canvas(), dst, &damage);
                rgba(&surface.image_snapshot())
            });

            let scene = state.layers_engine.scene().clone();
            let root = output_layer.id();
            let reference = std::thread::spawn(move || {
                let mut surface = skia::surfaces::raster_n32_premul(SIZE).unwrap();
                scene.with_arena(|arena| {
                    scene.with_renderable_arena(|renderables| {
                        layers::drawing::render_node_tree(
                            root,
                            arena,
                            renderables,
                            surface.canvas(),
                            1.0,
                            None,
                            None,
                            None,
                        );
                    })
                });
                rgba(&surface.image_snapshot())
            })
            .join()
            .unwrap();
            (frame, reference, bar)
        })
    }

    /// The largest per-channel difference between the two images over `rect`,
    /// and where it is.
    fn worst(a: &[u8], b: &[u8], rect: skia::IRect) -> (u8, (i32, i32)) {
        let mut worst = (0u8, (0, 0));
        for y in rect.top.max(0)..rect.bottom.min(SIZE.1) {
            for x in rect.left.max(0)..rect.right.min(SIZE.0) {
                let i = ((y * SIZE.0 + x) * 4) as usize;
                for c in 0..3 {
                    let d = a[i + c].abs_diff(b[i + c]);
                    if d > worst.0 {
                        worst = (d, (x, y));
                    }
                }
            }
        }
        worst
    }

    fn set_tick(handle: &HeadlessHandle, colour: Color) {
        handle.with_state(move |_| {
            TICK.with(|t| {
                t.borrow()
                    .as_ref()
                    .unwrap()
                    .set_background_color(colour, None)
            });
        });
    }

    /// Steps the window behind through `frames` colour changes of a small
    /// patch half under the bar, and checks every frame against the reference.
    fn check_frames(handle: &HeadlessHandle, what: &str, frames: usize) {
        const COLOURS: [(u8, u8, u8); 4] = [(255, 255, 255), (0, 200, 0), (255, 0, 255), (0, 0, 0)];
        let mut previous: Option<Vec<u8>> = None;
        for i in 0..frames {
            let (r, g, b) = COLOURS[i % COLOURS.len()];
            set_tick(handle, Color::new_rgba255(r, g, b, 255));
            let (frame, reference, bar) = frame_and_reference(handle);
            let (diff, at) = worst(&frame, &reference, bar);
            assert!(
                diff <= 12,
                "{what}, frame {i}: the frosted titlebar ({bar:?}) differs from a fresh \
                 render by {diff} at {at:?}"
            );
            // The patch shows through the frost: the bar changes with it, or
            // this checks nothing.
            if let Some(previous) = previous.as_ref() {
                let (moved, _) = worst(previous, &reference, bar);
                assert!(
                    moved > 12,
                    "{what}, frame {i}: the bar ({bar:?}) did not change with the patch behind it"
                );
            }
            previous = Some(reference);
        }
    }

    #[test]
    #[serial]
    fn a_frosted_titlebar_follows_what_changes_behind_it() {
        // The scale the bar was seen broken at: at a fractional scale the
        // bar's edges fall between pixels, and the rounding of its bounds
        // decides whether its blur is kept.
        let red = wallpaper(
            "titlebar-frost-red",
            skia::Color::from_argb(255, 200, 40, 40),
        );
        let handle = HeadlessHandle::start(HeadlessConfig {
            scale: Some(1.75),
            ..Default::default()
        });
        handle.settle(200);
        handle.set_background(&red);
        handle.settle(200);

        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let _behind = client.create_toplevel(BEHIND, 1200, 800);
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(BEHIND, 0, 0);
        handle.settle(200);

        let _front = client.create_toplevel(FRONT, 500, 300);
        let _ = client.roundtrip();
        handle.settle(300);
        handle.decorate_window(FRONT);
        handle.settle(300);
        handle.move_window(FRONT, 301, 303);
        let _ = client.roundtrip();
        handle.settle(500);
        assert_eq!(
            handle.window_decoration_active(FRONT),
            Some(true),
            "the front window holds focus, so its bar is frosted"
        );

        let scale = handle.output_scale() as f32;
        handle.with_state(move |state| {
            let window = state
                .workspaces
                .spaces_elements()
                .find(|w| w.xdg_title() == BEHIND)
                .cloned()
                .expect("the window behind is mapped");
            let _fill = solid(
                state,
                window.base_layer(),
                (0.0, 0.0),
                (2400.0, 1600.0),
                Color::new_rgba255(40, 60, 200, 255),
            );
            // A patch straddling the front bar's left end: part of it is
            // beneath the bar, part of it beside it.
            let tick = solid(
                state,
                window.base_layer(),
                (260.0 * scale, 290.0 * scale),
                (120.0 * scale, 40.0 * scale),
                Color::new_rgba255(255, 255, 255, 255),
            );
            TICK.with(|t| *t.borrow_mut() = Some(tick));
        });
        handle.settle(100);

        // First frame: everything is new, the whole output is drawn.
        let (frame, reference, bar) = frame_and_reference(&handle);
        let (diff, at) = worst(&frame, &reference, bar);
        assert!(diff <= 12, "first frame: off by {diff} at {at:?}");

        check_frames(&handle, "on screen", 6);

        // Past the left edge of the output: the bar is clipped by it.
        handle.move_window(FRONT, -121, 303);
        let _ = client.roundtrip();
        handle.settle(300);
        handle.with_state(move |_| {
            TICK.with(|t| {
                t.borrow()
                    .as_ref()
                    .unwrap()
                    .set_position((0.0, 290.0 * scale), None)
            });
        });
        handle.settle(100);
        let _ = frame_and_reference(&handle);
        check_frames(&handle, "partly off the output", 6);

        handle.stop();
    }
}
