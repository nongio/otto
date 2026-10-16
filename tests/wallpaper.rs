//! The desktop wallpaper, end to end: what reaches the screen, at the size
//! and in the layout that was set, and the same pixels however a frame is
//! repainted (nongio/otto#361).

#[cfg(feature = "headless")]
mod wallpaper_tests {
    use layers::skia;
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use serial_test::serial;

    /// The reporter's screen: 21:9, so a 16:9 photo is cropped top and
    /// bottom, which used to centre it on a half pixel.
    const SCREEN: (i32, i32) = (3440, 1440);
    /// How far in from the edge the background layer's rounded corners
    /// reach.
    const CORNER: i32 = 24;

    fn start() -> HeadlessHandle {
        let handle = HeadlessHandle::start(HeadlessConfig {
            width: SCREEN.0,
            height: SCREEN.1,
        });
        // The configuration outlives a session in this process: start every
        // test from the defaults, not from the last test's layout.
        handle.with_state(|state| {
            for id in [
                "background_image",
                "background_color",
                "background_mode",
                "background_position_x",
                "background_position_y",
                "background_zoom",
            ] {
                otto::settings::reset(state, id).unwrap_or_else(|err| panic!("{id}: {err}"));
            }
        });
        handle.settle(200);
        handle
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("otto-wallpaper-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// A PNG of pseudo-random pixels: each differs from its neighbours, so
    /// any resampling, or a pixel read from the wrong place, shows.
    fn noise_png(name: &str, w: i32, h: i32) -> String {
        png(name, w, h, |x, y| {
            let mut s = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
            s ^= s >> 15;
            s = s.wrapping_mul(0x2C1B_3C6D);
            s ^= s >> 12;
            [
                (s & 0xff) as u8,
                ((s >> 8) & 0xff) as u8,
                ((s >> 16) & 0xff) as u8,
            ]
        })
    }

    fn solid_png(name: &str, w: i32, h: i32, rgb: [u8; 3]) -> String {
        png(name, w, h, |_, _| rgb)
    }

    fn png(name: &str, w: i32, h: i32, f: impl Fn(i32, i32) -> [u8; 3]) -> String {
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let info = skia::ImageInfo::new(
            (w, h),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        let image =
            skia::images::raster_from_data(&info, skia::Data::new_copy(&data), (w * 4) as usize)
                .unwrap();
        let encoded = image
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let path = temp_path(name);
        std::fs::write(&path, encoded.as_bytes()).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn output_name(handle: &HeadlessHandle) -> String {
        handle.query(|state| {
            state
                .workspaces
                .outputs()
                .next()
                .map(|o| o.name())
                .expect("an output")
        })
    }

    fn frame(handle: &HeadlessHandle, position: usize) -> skia::Image {
        handle
            .background_frame(position)
            .unwrap_or_else(|| panic!("workspace {position} shows no wallpaper"))
    }

    /// Paint the first output's background plane — the wallpaper — into a
    /// raster surface the size of the screen, under `clips` one pass each,
    /// the way `SceneElement::draw` repaints damage rects. `None` paints the
    /// whole screen.
    fn paint(
        handle: &HeadlessHandle,
        onto: Option<Vec<u8>>,
        clips: Option<Vec<skia::IRect>>,
    ) -> Vec<u8> {
        handle.query(move |state| {
            let engine = state.layers_engine.clone();
            let ows = state
                .workspaces
                .output_workspaces
                .values()
                .next()
                .expect("an output");
            let info = skia::ImageInfo::new(
                SCREEN,
                skia::ColorType::RGBA8888,
                skia::AlphaType::Premul,
                None,
            );
            let mut pixels = onto.unwrap_or_else(|| vec![0u8; (SCREEN.0 * SCREEN.1 * 4) as usize]);
            {
                let mut surface = skia::surfaces::wrap_pixels(
                    &info,
                    pixels.as_mut_slice(),
                    SCREEN.0 as usize * 4,
                    None,
                )
                .expect("a surface over the pixels");
                let canvas = surface.canvas();
                let clips = clips.unwrap_or_else(|| vec![skia::IRect::from_wh(SCREEN.0, SCREEN.1)]);
                for clip in clips {
                    let save = canvas.save();
                    canvas.clip_irect(clip, None);
                    layers::drawing::draw_scene(canvas, engine.scene(), ows.background_plane.id);
                    canvas.restore_to_count(save);
                }
            }
            pixels
        })
    }

    fn frame_pixels(frame: &skia::Image) -> Vec<u8> {
        let info = skia::ImageInfo::new(
            (frame.width(), frame.height()),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        let mut data = vec![0u8; (frame.width() * frame.height() * 4) as usize];
        assert!(frame.read_pixels(
            &info,
            data.as_mut_slice(),
            frame.width() as usize * 4,
            (0, 0),
            skia::image::CachingHint::Disallow,
        ));
        data
    }

    fn at(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
        let i = ((y * SCREEN.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    fn set(handle: &HeadlessHandle, id: &'static str, value: otto::settings::value::SettingValue) {
        handle.with_state(move |state| {
            otto::settings::set(state, id, value).unwrap_or_else(|err| panic!("{id}: {err}"));
        });
        handle.settle(200);
    }

    /// The #361 case. A 16:9 photo on a 21:9 screen reaches the screen as a
    /// frame of exactly the screen's size, drawn 1:1: every pixel on screen
    /// is the frame's own, nothing filtered on the way.
    #[test]
    #[serial]
    fn the_wallpaper_is_drawn_pixel_for_pixel() {
        let handle = start();
        handle.set_background(&noise_png("photo.png", 3840, 2160));
        handle.settle(400);

        let frame = frame(&handle, 0);
        assert_eq!((frame.width(), frame.height()), SCREEN);
        let expected = frame_pixels(&frame);
        let painted = paint(&handle, None, None);
        for y in (CORNER..SCREEN.1 - CORNER).step_by(7) {
            for x in (CORNER..SCREEN.0 - CORNER).step_by(5) {
                let i = ((y * SCREEN.0 + x) * 4) as usize;
                assert_eq!(painted[i..i + 4], expected[i..i + 4], "({x},{y})");
            }
        }
        handle.stop();
    }

    /// Repainting only the damage around a moving window gives exactly the
    /// pixels a whole-screen paint does. Rects at odd offsets and sizes, as
    /// a dragged window's damage has.
    #[test]
    #[serial]
    fn a_repaint_cropped_to_damage_matches_the_full_paint() {
        let handle = start();
        handle.set_background(&noise_png("drag.png", 3840, 2160));
        handle.settle(400);

        let full = paint(&handle, None, None);
        let clips = vec![
            skia::IRect::from_xywh(411, 233, 500, 460),
            skia::IRect::from_xywh(1017, 301, 777, 555),
            skia::IRect::from_xywh(2999, 1201, 441, 239),
            skia::IRect::from_xywh(0, 0, 3, 1440),
        ];
        // Over a black screen, so a pixel the clipped passes missed shows as
        // well as one they got wrong.
        let partial = paint(&handle, Some(vec![0u8; full.len()]), Some(clips.clone()));
        for clip in clips {
            for y in clip.top..clip.bottom {
                for x in clip.left..clip.right {
                    assert_eq!(at(&full, x, y), at(&partial, x, y), "({x},{y}) in {clip:?}");
                }
            }
        }
        handle.stop();
    }

    /// The layout settings reach the screen: fitted, a wide image sits on
    /// the background colour.
    #[test]
    #[serial]
    fn fit_shows_the_background_colour_around_the_image() {
        use otto::settings::value::SettingValue;
        let handle = start();
        handle.set_background(&solid_png("square.png", 400, 400, [255, 0, 0]));
        set(
            &handle,
            "background_color",
            SettingValue::Str("#0000ff".into()),
        );
        set(&handle, "background_mode", SettingValue::Str("fit".into()));

        let painted = paint(&handle, None, None);
        let middle = SCREEN.1 / 2;
        assert_eq!(at(&painted, SCREEN.0 / 2, middle), [255, 0, 0, 255]);
        assert_eq!(
            at(&painted, 100, middle),
            [0, 0, 255, 255],
            "the bar is the colour"
        );
        assert_eq!(at(&painted, SCREEN.0 - 100, middle), [0, 0, 255, 255]);
        handle.stop();
    }

    /// sway's `output … bg` sets one output's wallpaper, writes it to the
    /// config, and `reset` takes it back to the global one.
    #[test]
    #[serial]
    fn output_bg_overrides_one_output() {
        let handle = start();
        let global = solid_png("global.png", 64, 64, [0, 255, 0]);
        let own = solid_png("own.png", 64, 64, [255, 0, 0]);
        handle.set_background(&global);
        let name = output_name(&handle);

        for result in handle.run_command(&format!("output {name} bg {own} stretch #123456")) {
            result.expect("the command runs");
        }
        handle.settle(400);
        let painted = paint(&handle, None, None);
        assert_eq!(at(&painted, 500, 500), [255, 0, 0, 255]);
        let (image, mode, color) = handle
            .output_background_override(&name)
            .expect("the override is in the config");
        assert_eq!(image.as_deref(), Some(own.as_str()));
        assert_eq!(mode.as_deref(), Some("stretch"));
        assert_eq!(color.as_deref(), Some("#123456"));

        for result in handle.run_command(&format!("output {name} bg reset")) {
            result.expect("the command runs");
        }
        handle.settle(400);
        let painted = paint(&handle, None, None);
        assert_eq!(at(&painted, 500, 500), [0, 255, 0, 255]);
        handle.stop();
    }

    /// An image that cannot be shown is refused, and nothing is written.
    #[test]
    #[serial]
    fn an_unshowable_image_is_refused() {
        let handle = start();
        let global = solid_png("kept.png", 64, 64, [0, 255, 0]);
        handle.set_background(&global);
        let broken = temp_path("broken.png");
        std::fs::write(&broken, b"not a picture").unwrap();

        let results = handle.run_command(&format!("output * bg {} fill", broken.display()));
        assert!(results[0].is_err(), "a broken image is refused");
        let results = handle.run_command("background /nonexistent/otto/none.png");
        assert!(results[0].is_err(), "a missing image is refused");
        assert_eq!(handle.background_image_setting(), global);
        handle.stop();
    }

    /// A workspace's own wallpaper shows on it alone, and travels with it
    /// when the workspaces are reordered.
    #[test]
    #[serial]
    fn a_workspace_wallpaper_travels_with_its_workspace() {
        let handle = start();
        let global = solid_png("everyone.png", 64, 64, [0, 255, 0]);
        let mine = solid_png("mine.png", 64, 64, [255, 0, 255]);
        handle.set_background(&global);
        handle.with_state(|state| {
            state.workspaces.add_workspace();
        });
        handle.set_workspace(0);
        handle.settle(400);

        for result in handle.run_command(&format!("background {mine}")) {
            result.expect("the command runs");
        }
        handle.settle(400);
        let pixel = |handle: &HeadlessHandle, position| {
            frame_pixels(&frame(handle, position))[..4].to_vec()
        };
        assert_eq!(pixel(&handle, 0), [255, 0, 255, 255]);
        assert_eq!(pixel(&handle, 1), [0, 255, 0, 255]);

        let name = output_name(&handle);
        handle.with_state(move |state| {
            assert!(state.workspaces.reorder_workspace_on_output(&name, 0, 1));
        });
        handle.settle(400);
        assert_eq!(pixel(&handle, 0), [0, 255, 0, 255]);
        assert_eq!(pixel(&handle, 1), [255, 0, 255, 255], "the override moved");
        handle.stop();
    }
}
