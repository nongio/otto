//! Incremental repaints near a frosted shape give the same pixels as a
//! fresh full render.
//!
//! The oracle: the output is drawn the way the single-plane composite path
//! draws it — the whole output tree through `SceneElement`, into
//! framebuffers kept between frames, repainting only what the element
//! reports damaged since that framebuffer was last drawn. With one
//! framebuffer that is a buffer age of 1; with several, rotated, it is the
//! swapchain age a real backend hands `OutputDamageTracker`. Each frame is
//! compared over the whole output with the same scene drawn from scratch on
//! a thread of its own (empty blur cache).
//!
//! What changes is a small solid patch whose colour flips every frame, at
//! a place relative to a frosted shape: beneath it, beside it within blur
//! reach, straddling its edge, or above it. The headless compositor imports
//! no client buffers, so solid layers stand in for client content.

#[cfg(feature = "headless")]
mod frost_damage_tests {
    use layers::prelude::Layer as SceneLayer;
    use layers::skia;
    use layers::types::Color;
    use otto::headless::{HeadlessConfig, HeadlessHandle};
    use otto_kit::testing::TestClient;
    use serial_test::serial;
    use smithay::backend::renderer::utils::CommitCounter;
    use std::cell::RefCell;

    const BEHIND: &str = "frost-damage-behind";
    const FRONT: &str = "frost-damage-front";
    const SIZE: (i32, i32) = (1920, 1080);
    /// Largest per-channel difference tolerated between an incremental
    /// frame and a fresh render (same tolerance as `titlebar_frost`).
    const TOLERANCE: u8 = 12;

    type Frames = (Vec<(skia::Surface, Option<CommitCounter>)>, usize);

    thread_local! {
        /// Framebuffers kept between frames, each with the commit it was last
        /// drawn at, and the index of the one drawn next. Lives on the
        /// compositor thread, where the frames are drawn.
        static FRAMES: RefCell<Frames> = const { RefCell::new((Vec::new(), 0)) };
        static PATCH: RefCell<Option<SceneLayer>> = const { RefCell::new(None) };
        /// The Vulkan renderer and its render targets, for `Gpu::Vulkan`.
        static VK: RefCell<Option<Vk>> = const { RefCell::new(None) };
    }

    /// Which Skia backend the frames are drawn with.
    #[derive(Clone, Copy, Debug)]
    enum Gpu {
        /// CPU raster surfaces.
        Raster,
        /// `SkiaVkRenderer` on the first Vulkan device with a render node
        /// (offscreen dmabuf-less targets, no DRM session): the renderer the
        /// udev backend draws with by default. The reference is drawn by
        /// the same renderer (see `frame_and_reference`).
        Vulkan,
    }

    type VkTargets = Vec<(otto::renderer::vulkan::SkiaVkTarget, Option<CommitCounter>)>;

    struct Vk {
        renderer: otto::renderer::vulkan::SkiaVkRenderer,
        targets: VkTargets,
        next: usize,
    }

    fn vk_device() -> smithay::backend::vulkan::PhysicalDevice {
        use smithay::backend::vulkan::{version::Version, Instance, PhysicalDevice};
        let instance = Instance::new(Version::VERSION_1_3, None).expect("vulkan instance");
        let phd = PhysicalDevice::enumerate(&instance)
            .expect("enumerate physical devices")
            .find(|phd| matches!(phd.render_node(), Ok(Some(_))))
            .expect("a Vulkan device with a render node");
        phd
    }

    /// Draws `paint` into `target` with `renderer` and reads it back as RGBA.
    fn vk_draw(
        renderer: &mut otto::renderer::vulkan::SkiaVkRenderer,
        target: &mut otto::renderer::vulkan::SkiaVkTarget,
        paint: impl FnOnce(&skia::Canvas),
    ) -> Vec<u8> {
        use smithay::backend::allocator::Fourcc;
        use smithay::backend::renderer::{ExportMem, Frame, Renderer};
        let sync = {
            let mut frame = renderer
                .render(target, SIZE.into(), smithay::utils::Transform::Normal)
                .expect("frame");
            paint(frame.skia_surface.canvas());
            frame.finish().expect("finish")
        };
        sync.wait().expect("sync point");
        let mapping = renderer
            .copy_framebuffer(
                target,
                smithay::utils::Rectangle::from_size(SIZE.into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        renderer.map_texture(&mapping).expect("map").to_vec()
    }

    #[derive(Clone, Copy, Debug)]
    enum Frost {
        /// The focused window's server-side titlebar.
        Titlebar,
        /// A whole client window with the `BackgroundBlur` material.
        Material,
    }

    #[derive(Clone, Copy, Debug)]
    enum Parent {
        /// The patch is content of the window behind the frost.
        Behind,
        /// The patch is in the overlay plane, painted above the frost.
        Above,
    }

    #[derive(Clone, Copy, Debug)]
    struct Case {
        frost: Frost,
        scale: f64,
        /// Patch rect in logical px, output space.
        patch: (f32, f32, f32, f32),
        parent: Parent,
        /// Framebuffers in rotation: the buffer age the frames are drawn at.
        buffers: usize,
        /// Where the front window goes, logical px.
        front_at: (i32, i32),
        draw: Draw,
        gpu: Gpu,
        /// Compare only over the front window, not the whole output.
        only_front: bool,
    }

    /// How a frame repaints its damage.
    #[derive(Clone, Copy, Debug)]
    enum Draw {
        /// `SceneElement::draw_scene`: one clipped pass per damage rect, frost
        /// grown when stale. The single-plane composite (winit, the udev
        /// composite fallback, Vulkan).
        Passes,
        /// Modelled on `SceneDmabufElement::render`, the udev KMS plane path.
        Region,
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

    /// Repaints `damage` of the output tree into `canvas` the way `draw` says.
    fn paint_damage(
        state: &otto::state::Otto<otto::headless::HeadlessData>,
        element: &otto::render_elements::scene_element::SceneElement,
        output_layer: &SceneLayer,
        canvas: &skia::Canvas,
        damage: &[smithay::utils::Rectangle<i32, smithay::utils::Physical>],
        draw: Draw,
    ) {
        match draw {
            Draw::Passes => {
                let dst = smithay::utils::Rectangle::new((0, 0).into(), SIZE.into());
                element.draw_scene(canvas, dst, damage);
            }
            Draw::Region => {
                // The way `SceneDmabufElement::render` draws a plane: one
                // pass under a clip of every damage rect at once, cleared, no
                // damage region handed to lay-rs, no frost growth.
                if !damage.is_empty() {
                    let save = canvas.save();
                    let mut region = skia::Region::new();
                    for r in damage.iter() {
                        region.op_rect(
                            skia::IRect::from_xywh(r.loc.x, r.loc.y, r.size.w, r.size.h),
                            skia::region::RegionOp::Union,
                        );
                    }
                    canvas.clip_region(&region, skia::ClipOp::Intersect);
                    canvas.clear(skia::Color4f::new(0.0, 0.0, 0.0, 0.0));
                    let scene = state.layers_engine.scene();
                    scene.with_arena(|arena| {
                        scene.with_renderable_arena(|renderables| {
                            layers::drawing::render_node_tree(
                                output_layer.id(),
                                arena,
                                renderables,
                                canvas,
                                1.0,
                                None,
                                None,
                                None,
                            );
                        })
                    });
                    canvas.restore_to_count(save);
                }
                state.layers_engine.clear_damage();
            }
        }
    }

    fn paint_whole(
        scene: &std::sync::Arc<layers::engine::scene::Scene>,
        root: layers::engine::NodeRef,
        canvas: &skia::Canvas,
        region: Option<&skia::Region>,
    ) {
        scene.with_arena(|arena| {
            scene.with_renderable_arena(|renderables| {
                layers::drawing::render_node_tree(
                    root,
                    arena,
                    renderables,
                    canvas,
                    1.0,
                    None,
                    region,
                    None,
                );
            })
        });
    }

    /// One frame drawn incrementally into the next framebuffer of the
    /// rotation, and the same scene drawn whole from scratch.
    fn frame_and_reference(
        handle: &HeadlessHandle,
        buffers: usize,
        draw: Draw,
        gpu: Gpu,
    ) -> (Vec<u8>, Vec<u8>) {
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
            let trace = std::env::var_os("OTTO_TEST_TRACE").is_some();

            let frame = match gpu {
                Gpu::Raster => FRAMES.with(|frames| {
                    let mut frames = frames.borrow_mut();
                    let (surfaces, next) = &mut *frames;
                    while surfaces.len() < buffers {
                        surfaces.push((skia::surfaces::raster_n32_premul(SIZE).unwrap(), None));
                    }
                    let (surface, last) = &mut surfaces[*next];
                    *next = (*next + 1) % buffers;
                    let damage = element.damage_since(scale, *last);
                    if trace {
                        eprintln!("  damage since {last:?}: {:?}", damage.as_ref());
                    }
                    *last = Some(element.current_commit());
                    paint_damage(
                        state,
                        &element,
                        &output_layer,
                        surface.canvas(),
                        &damage,
                        draw,
                    );
                    rgba(&surface.image_snapshot())
                }),
                Gpu::Vulkan => VK.with(|vk| {
                    use smithay::backend::allocator::Fourcc;
                    use smithay::backend::renderer::Offscreen;
                    let mut vk = vk.borrow_mut();
                    let vk = vk.get_or_insert_with(|| Vk {
                        renderer: otto::renderer::vulkan::SkiaVkRenderer::new(&vk_device())
                            .expect("vulkan renderer"),
                        targets: Vec::new(),
                        next: 0,
                    });
                    while vk.targets.len() < buffers {
                        let target = vk
                            .renderer
                            .create_buffer(Fourcc::Abgr8888, SIZE.into())
                            .expect("render target");
                        vk.targets.push((target, None));
                    }
                    let index = vk.next;
                    vk.next = (vk.next + 1) % buffers;
                    let last = vk.targets[index].1;
                    let damage = element.damage_since(scale, last);
                    if trace {
                        eprintln!("  damage since {last:?}: {:?}", damage.as_ref());
                    }
                    vk.targets[index].1 = Some(element.current_commit());
                    let Vk {
                        renderer, targets, ..
                    } = vk;
                    vk_draw(renderer, &mut targets[index].0, |canvas| {
                        paint_damage(state, &element, &output_layer, canvas, &damage, draw)
                    })
                }),
            };

            let scene = state.layers_engine.scene().clone();
            let root = output_layer.id();
            let reference = match gpu {
                // On a thread of its own, whose blur cache starts empty.
                Gpu::Raster => std::thread::spawn(move || {
                    let mut surface = skia::surfaces::raster_n32_premul(SIZE).unwrap();
                    paint_whole(&scene, root, surface.canvas(), None);
                    rgba(&surface.image_snapshot())
                })
                .join()
                .unwrap(),
                Gpu::Vulkan => {
                    // A second renderer on another thread cannot draw this
                    // scene: lay-rs keeps GPU images of the context that drew
                    // a layer (image-cached layers) in the shared scene, and
                    // a second context frees them under the first. So the
                    // reference is drawn whole by the same renderer into a
                    // target of its own. It replays the same kept blurs, so
                    // on Vulkan this oracle sees stale regions (damage that
                    // missed a buffer), not a replay that differs from a
                    // fresh blur. Its damage region is the output, as the
                    // composite path's passes are: lay-rs then keeps no blur
                    // from it that the frames would not have kept.
                    VK.with(|vk| {
                        use smithay::backend::allocator::Fourcc;
                        use smithay::backend::renderer::Offscreen;
                        let mut vk = vk.borrow_mut();
                        let vk = vk.as_mut().unwrap();
                        let mut target = vk
                            .renderer
                            .create_buffer(Fourcc::Abgr8888, SIZE.into())
                            .expect("reference target");
                        let scene = state.layers_engine.scene().clone();
                        let mut output = skia::Region::new();
                        output.set_rect(skia::IRect::from_wh(SIZE.0, SIZE.1));
                        vk_draw(&mut vk.renderer, &mut target, |canvas| {
                            canvas.clear(skia::Color4f::new(0.0, 0.0, 0.0, 0.0));
                            paint_whole(&scene, root, canvas, Some(&output))
                        })
                    })
                }
            };
            (frame, reference)
        })
    }

    /// The largest per-channel difference and where, plus the bounding box
    /// of the pixels past the tolerance.
    fn diff(a: &[u8], b: &[u8], over: skia::IRect) -> (u8, (i32, i32), Option<skia::IRect>) {
        let mut worst = (0u8, (0, 0));
        let mut bbox: Option<skia::IRect> = None;
        for y in over.top.max(0)..over.bottom.min(SIZE.1) {
            for x in over.left.max(0)..over.right.min(SIZE.0) {
                let i = ((y * SIZE.0 + x) * 4) as usize;
                let d = (0..3).map(|c| a[i + c].abs_diff(b[i + c])).max().unwrap();
                if d > worst.0 {
                    worst = (d, (x, y));
                }
                if d > TOLERANCE {
                    bbox = Some(match bbox {
                        Some(r) => skia::IRect::from_ltrb(
                            r.left.min(x),
                            r.top.min(y),
                            r.right.max(x + 1),
                            r.bottom.max(y + 1),
                        ),
                        None => skia::IRect::from_xywh(x, y, 1, 1),
                    });
                }
            }
        }
        (worst.0, worst.1, bbox)
    }

    /// Writes the frame, the reference and an amplified diff as PNGs to
    /// `OTTO_TEST_DIFF_DIR`, when set.
    fn save(name: &str, frame: &[u8], reference: &[u8]) {
        let Ok(dir) = std::env::var("OTTO_TEST_DIFF_DIR") else {
            return;
        };
        let info = skia::ImageInfo::new(
            SIZE,
            skia::ColorType::RGBA8888,
            skia::AlphaType::Unpremul,
            None,
        );
        let mut d = vec![0u8; frame.len()];
        for i in (0..frame.len()).step_by(4) {
            for c in 0..3 {
                d[i + c] = frame[i + c].abs_diff(reference[i + c]).saturating_mul(8);
            }
            d[i + 3] = 255;
        }
        let _ = std::fs::create_dir_all(&dir);
        for (suffix, px) in [("frame", frame), ("reference", reference), ("diff", &d[..])] {
            let image = skia::images::raster_from_data(
                &info,
                skia::Data::new_copy(px),
                (SIZE.0 * 4) as usize,
            )
            .unwrap();
            let data = image
                .encode(None, skia::EncodedImageFormat::PNG, 100)
                .unwrap();
            let _ = std::fs::write(
                std::path::Path::new(&dir).join(format!("{name}-{suffix}.png")),
                data.as_bytes(),
            );
        }
    }

    /// Runs one case and returns a failure description, if any frame
    /// differed from the fresh render.
    fn run(name: &str, case: Case) -> Option<String> {
        let red = wallpaper("frost-damage-red", skia::Color::from_argb(255, 200, 40, 40));
        let handle = HeadlessHandle::start(HeadlessConfig {
            scale: Some(case.scale),
            ..Default::default()
        });
        handle.with_state(|_| FRAMES.with(|f| *f.borrow_mut() = (Vec::new(), 0)));
        handle.settle(200);
        handle.set_background(&red);
        handle.settle(200);

        let mut client = TestClient::connect(&handle.socket_name).expect("connect");
        let _behind = client.create_toplevel(BEHIND, 1200, 800);
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(BEHIND, 0, 0);
        handle.settle(200);

        let front = client.create_toplevel(FRONT, 500, 300);
        let _ = client.roundtrip();
        handle.settle(300);
        let _style = match case.frost {
            Frost::Titlebar => {
                handle.decorate_window(FRONT);
                None
            }
            Frost::Material => {
                let surface = front.lock().unwrap().surface.clone();
                let style = client.request_material(&surface);
                front.lock().unwrap().commit_frame();
                style
            }
        };
        let _ = client.roundtrip();
        handle.settle(300);
        handle.move_window(FRONT, case.front_at.0, case.front_at.1);
        let _ = client.roundtrip();
        handle.settle(500);

        let scale = handle.output_scale() as f32;
        let (px, py, pw, ph) = case.patch;
        let parent = case.parent;
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
                (2400.0 * scale, 1600.0 * scale),
                Color::new_rgba255(40, 60, 200, 255),
            );
            // Static stripes both ways, so the frost has edges to blur in
            // either direction.
            for i in 0..14 {
                let _ = solid(
                    state,
                    window.base_layer(),
                    (i as f32 * 90.0 * scale, 0.0),
                    (30.0 * scale, 1600.0 * scale),
                    Color::new_rgba255(250, 220, 40, 255),
                );
                let _ = solid(
                    state,
                    window.base_layer(),
                    (0.0, (i as f32 * 70.0 + 20.0) * scale),
                    (2400.0 * scale, 20.0 * scale),
                    Color::new_rgba255(20, 160, 60, 255),
                );
            }
            let patch_parent = match parent {
                Parent::Behind => window.base_layer().clone(),
                Parent::Above => {
                    let ows = state.workspaces.output_workspaces.values().next().unwrap();
                    ows.overlay_plane.clone()
                }
            };
            let patch = solid(
                state,
                &patch_parent,
                (px * scale, py * scale),
                (pw * scale, ph * scale),
                Color::new_rgba255(255, 255, 255, 255),
            );
            PATCH.with(|p| *p.borrow_mut() = Some(patch));
        });

        // Draw the setup through the element into every framebuffer, so its
        // damage is consumed the way a backend would consume it.
        for _ in 0..(case.buffers + 1) {
            let _ = frame_and_reference(&handle, case.buffers, case.draw, case.gpu);
        }

        const COLOURS: [(u8, u8, u8); 4] = [(255, 255, 255), (0, 200, 0), (255, 0, 255), (0, 0, 0)];
        let mut failure = None;
        for i in 0..8 {
            let (r, g, b) = COLOURS[i % COLOURS.len()];
            handle.with_state(move |_| {
                PATCH.with(|p| {
                    p.borrow()
                        .as_ref()
                        .unwrap()
                        .set_background_color(Color::new_rgba255(r, g, b, 255), None)
                });
            });
            let (frame, reference) =
                frame_and_reference(&handle, case.buffers, case.draw, case.gpu);
            let over = if case.only_front {
                skia::IRect::from_xywh(
                    (case.front_at.0 as f32 * scale) as i32,
                    (case.front_at.1 as f32 * scale) as i32,
                    (500.0 * scale) as i32,
                    (300.0 * scale) as i32,
                )
            } else {
                skia::IRect::from_wh(SIZE.0, SIZE.1)
            };
            let (worst, at, bbox) = diff(&frame, &reference, over);
            if std::env::var_os("OTTO_TEST_TRACE").is_some() {
                eprintln!("  frame {i}: worst {worst} at {at:?}");
            }
            if worst > TOLERANCE {
                save(&format!("{name}-f{i}"), &frame, &reference);
                failure = Some(format!(
                    "{name}: frame {i} differs from a fresh render by {worst} at {at:?}, \
                     bad pixels in {bbox:?}"
                ));
                break;
            }
        }
        // The kept blurs hold images of the Vulkan context: drop them before
        // the renderer, and both before the compositor thread winds down.
        handle.query(|_| {
            layers::drawing::clear_blur_cache();
            VK.with(|vk| vk.borrow_mut().take());
        });
        handle.stop();
        failure
    }

    // The front window: logical (301, 303), 500x300. Its titlebar is the
    // top 34 logical px; a material window's frost is the whole window.
    fn cases(
        frost: Frost,
        scale: f64,
        buffers: usize,
        draw: Draw,
        gpu: Gpu,
    ) -> Vec<(String, Case)> {
        let c = |patch, parent| Case {
            frost,
            scale,
            patch,
            parent,
            buffers,
            front_at: (301, 303),
            draw,
            gpu,
            only_front: false,
        };
        let tag = format!("{frost:?}-{draw:?}-{gpu:?}-s{scale}-b{buffers}").to_lowercase();
        // The output in logical px.
        let (w, h) = (
            (SIZE.0 as f64 / scale).floor() as i32,
            (SIZE.1 as f64 / scale).floor() as i32,
        );
        vec![
            (
                format!("{tag}-beneath"),
                c((400.0, 310.0, 40.0, 15.0), Parent::Behind),
            ),
            (
                format!("{tag}-straddle"),
                c((260.0, 290.0, 120.0, 40.0), Parent::Behind),
            ),
            // Beside the shape, within any plausible blur radius of it.
            (
                format!("{tag}-beside-left"),
                c((281.0, 310.0, 16.0, 16.0), Parent::Behind),
            ),
            (
                format!("{tag}-beside-top"),
                c((400.0, 287.0, 40.0, 12.0), Parent::Behind),
            ),
            // Above the shape, inside it, across several stripes: the pass
            // does not reach the whole shape and the frost is not stale, so
            // the kept blur has to be replayed.
            (
                format!("{tag}-above"),
                c((330.0, 310.0, 240.0, 15.0), Parent::Above),
            ),
            (
                format!("{tag}-above-straddle"),
                c((260.0, 290.0, 120.0, 40.0), Parent::Above),
            ),
            // The same with the front window high up, wholly on the output
            // at every scale tried.
            (
                format!("{tag}-onscreen-above"),
                Case {
                    front_at: (301, 60),
                    ..c((330.0, 67.0, 240.0, 15.0), Parent::Above)
                },
            ),
            // The front window hanging off the bottom of the output.
            (
                format!("{tag}-offbottom-above"),
                Case {
                    front_at: (301, h - 150),
                    ..c((330.0, (h - 143) as f32, 240.0, 15.0), Parent::Above)
                },
            ),
            // Off the bottom, the patch straddling the window's top edge.
            (
                format!("{tag}-offbottom-above-straddle"),
                Case {
                    front_at: (301, h - 150),
                    ..c((330.0, (h - 175) as f32, 240.0, 50.0), Parent::Above)
                },
            ),
            // On the output, the same straddle.
            (
                format!("{tag}-onscreen-above-straddle"),
                Case {
                    front_at: (301, 60),
                    ..c((330.0, 35.0, 240.0, 50.0), Parent::Above)
                },
            ),
            // The front window under the middle of the output, where the
            // OSD and the app switcher panel sit, its top-left corner under
            // them; the patch far away.
            (
                format!("{tag}-under-osd"),
                Case {
                    front_at: (w / 2 - 20, h / 2 - 20),
                    ..c((20.0, 20.0, 40.0, 15.0), Parent::Above)
                },
            ),
            // Off the right edge.
            (
                format!("{tag}-offright-above"),
                Case {
                    front_at: (w - 250, 303),
                    ..c(((w - 240) as f32, 310.0, 230.0, 15.0), Parent::Above)
                },
            ),
            // Off the left edge.
            (
                format!("{tag}-offleft-above"),
                Case {
                    front_at: (-250, 303),
                    ..c((0.0, 310.0, 230.0, 15.0), Parent::Above)
                },
            ),
        ]
    }

    fn check_all(frost: Frost, buffers: usize, draw: Draw) {
        let gpu = match std::env::var("OTTO_TEST_GPU").as_deref() {
            Ok("vulkan") => Gpu::Vulkan,
            _ => Gpu::Raster,
        };
        let scales: Vec<f64> = std::env::var("OTTO_TEST_SCALES")
            .map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect())
            .unwrap_or_else(|_| vec![1.0, 1.5, 1.75, 2.0]);
        let only = std::env::var("OTTO_TEST_CASE").ok();
        let failures: Vec<String> = scales
            .iter()
            .flat_map(|scale| cases(frost, *scale, buffers, draw, gpu))
            .filter(|(name, _)| only.as_ref().is_none_or(|o| name.contains(o.as_str())))
            .filter_map(|(name, case)| {
                let r = run(&name, case);
                eprintln!("{name}: {}", r.as_deref().unwrap_or("ok"));
                r
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    fn check_one(name: &str, case: Case) {
        if let Some(failure) = run(name, case) {
            panic!("{failure}");
        }
    }

    /// A frosted window hanging off the bottom of a 2x output, a patch just
    /// above it. From the second frame on, the app switcher panel and the OSD
    /// (both `BackgroundBlur`, both at opacity 0, centred on the output)
    /// report damage on every frame, and that damage cuts across the top of
    /// the window. lay-rs never keeps a blur for this window — a pass's
    /// damage region never contains its bounds, which run past the output —
    /// while `SceneElement` records it as kept after the first full pass, so
    /// the pass is not grown over the window and the part of the frost it
    /// covers comes out blurred differently from the rest: a box with hard
    /// edges inside the frost (up to 49/255 off).
    #[test]
    #[serial]
    #[ignore = "repro: frost under a pass from the app switcher/OSD, partly off the output, \
                breaks into a box"]
    fn a_frosted_window_partly_off_the_output_keeps_its_frost_whole() {
        off_output_frost(Gpu::Raster);
    }

    /// The same, drawn with the Vulkan renderer on the machine's Vulkan
    /// device.
    #[test]
    #[serial]
    #[ignore = "needs a Vulkan GPU; repro as the raster one"]
    fn vulkan_a_frosted_window_partly_off_the_output_keeps_its_frost_whole() {
        off_output_frost(Gpu::Vulkan);
    }

    fn off_output_frost(gpu: Gpu) {
        check_one(
            "off-output-frost",
            Case {
                gpu,
                // The dock's rim differs on its own (the other repro).
                only_front: true,
                frost: Frost::Material,
                scale: 2.0,
                patch: (400.0, 287.0, 40.0, 12.0),
                parent: Parent::Behind,
                buffers: 1,
                front_at: (301, 303),
                draw: Draw::Passes,
            },
        );
    }

    /// Three framebuffers in rotation, nothing changing near the dock after
    /// setup. The first full frame blurs the dock's frost afresh and keeps
    /// it; the next two full frames replay the kept blur. Replayed, the
    /// frost differs from a fresh one along the shape's anti-aliased rim
    /// (up to 25-28/255): a buffer that holds a replay shows a different
    /// outline than one holding a fresh blur, and swapping between them —
    /// any partial update elsewhere at buffer age > 1 — makes the rim
    /// shimmer.
    #[test]
    #[serial]
    #[ignore = "repro: a replayed kept blur differs from a fresh blur along the shape's rim"]
    fn a_replayed_frost_matches_a_fresh_one() {
        replayed_frost(Gpu::Raster);
    }

    /// The same, drawn with the Vulkan renderer on the machine's Vulkan
    /// device.
    #[test]
    #[serial]
    #[ignore = "needs a Vulkan GPU; repro as the raster one"]
    fn vulkan_a_replayed_frost_matches_a_fresh_one() {
        replayed_frost(Gpu::Vulkan);
    }

    fn replayed_frost(gpu: Gpu) {
        check_one(
            "replayed-frost-rim",
            Case {
                gpu,
                only_front: false,
                frost: Frost::Material,
                scale: 1.0,
                patch: (400.0, 310.0, 40.0, 15.0),
                parent: Parent::Behind,
                buffers: 3,
                front_at: (301, 303),
                draw: Draw::Passes,
            },
        );
    }

    // The whole exploratory matrix: every case at scales 1, 1.5, 1.75 and 2
    // (`OTTO_TEST_SCALES=1,2` narrows it, `OTTO_TEST_CASE=<substring>`
    // picks cases, `OTTO_TEST_DIFF_DIR` saves frame/reference/diff PNGs,
    // `OTTO_TEST_TRACE=1` prints the damage per frame). Several minutes
    // each, and they fail on the two repros above.

    #[test]
    #[serial]
    #[ignore = "exploratory matrix, slow"]
    fn matrix_titlebar_frost_age_1() {
        check_all(Frost::Titlebar, 1, Draw::Passes);
    }

    #[test]
    #[serial]
    #[ignore = "exploratory matrix, slow"]
    fn matrix_titlebar_frost_age_3() {
        check_all(Frost::Titlebar, 3, Draw::Passes);
    }

    #[test]
    #[serial]
    #[ignore = "exploratory matrix, slow"]
    fn matrix_material_frost_age_1() {
        check_all(Frost::Material, 1, Draw::Passes);
    }

    #[test]
    #[serial]
    #[ignore = "exploratory matrix, slow"]
    fn matrix_material_frost_age_3() {
        check_all(Frost::Material, 3, Draw::Passes);
    }

    #[test]
    #[serial]
    #[ignore = "exploratory matrix, slow"]
    fn matrix_titlebar_frost_region() {
        check_all(Frost::Titlebar, 3, Draw::Region);
    }

    #[test]
    #[serial]
    #[ignore = "exploratory matrix, slow"]
    fn matrix_material_frost_region() {
        check_all(Frost::Material, 3, Draw::Region);
    }
}
