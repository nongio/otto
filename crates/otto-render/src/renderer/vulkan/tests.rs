//! GPU tests of the Vulkan renderer. They need a Vulkan device with a render
//! node, so they are ignored by default:
//! `cargo test --features vulkan --lib -- --ignored vulkan`.

use std::os::fd::OwnedFd;

use smithay::{
    backend::{
        allocator::{
            dmabuf::{AsDmabuf, Dmabuf, DmabufFlags},
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Allocator, Buffer, Fourcc, Modifier,
        },
        drm::DrmDeviceFd,
        renderer::{Bind, Color32F, ExportMem, Frame, ImportDma, ImportMem, Offscreen, Renderer},
        vulkan::{version::Version, Instance, PhysicalDevice},
    },
    utils::{DeviceFd, Rectangle, Transform},
};

use super::SkiaVkRenderer;

const SIZE: i32 = 64;

fn first_physical_device() -> PhysicalDevice {
    let instance = Instance::new(Version::VERSION_1_3, None).expect("vulkan instance");
    let phd = PhysicalDevice::enumerate(&instance)
        .expect("enumerate physical devices")
        .find(|phd| matches!(phd.render_node(), Ok(Some(_))))
        .expect("a physical device with a render node");
    phd
}

fn full() -> Rectangle<i32, smithay::utils::Physical> {
    Rectangle::from_size((SIZE, SIZE).into())
}

/// Reads the framebuffer back as `Abgr8888`, i.e. R, G, B, A bytes.
fn pixel(data: &[u8], x: i32, y: i32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_offscreen_frame_draws_and_reads_back() {
    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");

    // A 16x16 green texture from memory (Abgr8888 is R, G, B, A in memory).
    let green: Vec<u8> = [0u8, 255, 0, 255].repeat(16 * 16);
    let texture = renderer
        .import_memory(&green, Fourcc::Abgr8888, (16, 16).into(), false)
        .expect("import memory");

    let sync = {
        let mut frame = renderer
            .render(&mut target, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .clear(Color32F::new(1.0, 0.0, 0.0, 1.0), &[full()])
            .expect("clear");
        let quadrant = Rectangle::from_size((SIZE / 2, SIZE / 2).into());
        frame
            .draw_solid(
                Rectangle::new((SIZE / 2, 0).into(), (SIZE / 2, SIZE / 2).into()),
                &[quadrant],
                Color32F::new(0.0, 0.0, 1.0, 1.0),
            )
            .expect("draw solid");
        frame
            .render_texture_from_to(
                &texture,
                Rectangle::from_size((16.0, 16.0).into()),
                Rectangle::new((0, SIZE / 2).into(), (SIZE / 2, SIZE / 2).into()),
                &[quadrant],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("render texture");
        frame.finish().expect("finish")
    };
    assert!(
        sync.export().is_some(),
        "the frame's sync point is a sync file"
    );
    sync.wait().expect("sync point signals");

    let mapping = renderer
        .copy_framebuffer(
            &target,
            Rectangle::from_size((SIZE, SIZE).into()),
            Fourcc::Abgr8888,
        )
        .expect("copy framebuffer");
    let data = renderer.map_texture(&mapping).expect("map").to_vec();

    assert_eq!(pixel(&data, 8, 8), [255, 0, 0, 255], "cleared red");
    assert_eq!(pixel(&data, 48, 8), [0, 0, 255, 255], "solid blue");
    assert_eq!(pixel(&data, 8, 48), [0, 255, 0, 255], "green texture");
    assert_eq!(pixel(&data, 48, 48), [255, 0, 0, 255], "cleared red");

    // `update_memory` rewrites the texture in place.
    let white: Vec<u8> = [255u8; 4].repeat(16 * 16);
    renderer
        .update_memory(&texture, &white, Rectangle::from_size((16, 16).into()))
        .expect("update memory");
    let sync = {
        let mut frame = renderer
            .render(&mut target, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .render_texture_from_to(
                &texture,
                Rectangle::from_size((16.0, 16.0).into()),
                full(),
                &[full()],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("render texture");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    let mapping = renderer
        .copy_framebuffer(
            &target,
            Rectangle::from_size((SIZE, SIZE).into()),
            Fourcc::Abgr8888,
        )
        .expect("copy framebuffer");
    let data = renderer.map_texture(&mapping).expect("map").to_vec();
    assert_eq!(
        pixel(&data, 40, 40),
        [255, 255, 255, 255],
        "updated texture"
    );
}

#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_gbm_dmabuf_renders_and_imports_back() {
    let phd = first_physical_device();
    let mut renderer = SkiaVkRenderer::new(&phd).expect("renderer");

    let node = phd.render_node().ok().flatten().expect("render node");
    let path = node.dev_path().expect("render node path");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open render node");
    let fd = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
    let gbm = GbmDevice::new(fd).expect("gbm device");
    let mut allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING);

    let mut modifiers: Vec<Modifier> = renderer
        .dmabuf_render_formats()
        .iter()
        .filter(|format| format.code == Fourcc::Argb8888)
        .map(|format| format.modifier)
        .collect();
    assert!(!modifiers.is_empty(), "Argb8888 is renderable");
    modifiers.retain(|modifier| *modifier != Modifier::Invalid);
    let buffer = allocator
        .create_buffer(SIZE as u32, SIZE as u32, Fourcc::Argb8888, &modifiers)
        .expect("gbm buffer");
    let mut dmabuf = buffer.export().expect("export dmabuf");

    let mut framebuffer = renderer.bind(&mut dmabuf).expect("bind dmabuf");
    let sync = {
        let mut frame = renderer
            .render(&mut framebuffer, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .clear(Color32F::new(1.0, 0.0, 0.0, 1.0), &[full()])
            .expect("clear");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");

    // A second frame damages only the left half: the right half keeps the
    // red of the first, as a reused swapchain slot must.
    let mut framebuffer = renderer.bind(&mut dmabuf).expect("bind dmabuf again");
    let sync = {
        let mut frame = renderer
            .render(&mut framebuffer, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        let left = Rectangle::from_size((SIZE / 2, SIZE).into());
        frame
            .draw_solid(left, &[left], Color32F::new(0.0, 1.0, 0.0, 1.0))
            .expect("draw solid");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");

    let texture = renderer
        .import_dmabuf(&dmabuf, None)
        .expect("import dmabuf");
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");
    let sync = {
        let mut frame = renderer
            .render(&mut target, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &[full()])
            .expect("clear");
        frame
            .render_texture_from_to(
                &texture,
                Rectangle::from_size((SIZE as f64, SIZE as f64).into()),
                full(),
                &[full()],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("render texture");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");

    let mapping = renderer
        .copy_framebuffer(
            &target,
            Rectangle::from_size((SIZE, SIZE).into()),
            Fourcc::Abgr8888,
        )
        .expect("copy framebuffer");
    let data = renderer.map_texture(&mapping).expect("map").to_vec();
    for (x, y) in [(0, 0), (SIZE / 4, SIZE / 2), (SIZE / 2 - 1, SIZE - 1)] {
        assert_eq!(pixel(&data, x, y), [0, 255, 0, 255], "green at {x},{y}");
    }
    for (x, y) in [
        (SIZE / 2, 0),
        (SIZE * 3 / 4, SIZE / 2),
        (SIZE - 1, SIZE - 1),
    ] {
        assert_eq!(pixel(&data, x, y), [255, 0, 0, 255], "red at {x},{y}");
    }

    // The screenshare path: copy the frame just rendered (the offscreen
    // target, green and red) into another dmabuf and read that back.
    let second = allocator
        .create_buffer(SIZE as u32, SIZE as u32, Fourcc::Argb8888, &modifiers)
        .expect("gbm buffer");
    let second = second.export().expect("export dmabuf");
    renderer
        .blit_current_frame(&second, full(), full())
        .expect("blit current frame");
    let copied = renderer
        .import_dmabuf(&second, None)
        .expect("import copied dmabuf");
    let mut readback = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");
    let sync = {
        let mut frame = renderer
            .render(&mut readback, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .render_texture_from_to(
                &copied,
                Rectangle::from_size((SIZE as f64, SIZE as f64).into()),
                full(),
                &[full()],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("render texture");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    let mapping = renderer
        .copy_framebuffer(
            &readback,
            Rectangle::from_size((SIZE, SIZE).into()),
            Fourcc::Abgr8888,
        )
        .expect("copy framebuffer");
    let data = renderer.map_texture(&mapping).expect("map").to_vec();
    assert_eq!(
        pixel(&data, SIZE / 4, SIZE / 2),
        [0, 255, 0, 255],
        "copied green"
    );
    assert_eq!(
        pixel(&data, SIZE * 3 / 4, SIZE / 2),
        [255, 0, 0, 255],
        "copied red"
    );
}

/// Samples `texture` onto a transparent frame and returns the pixel at (8, 8).
fn sample(renderer: &mut SkiaVkRenderer, texture: &super::SkiaVkTexture) -> [u8; 4] {
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");
    let sync = {
        let mut frame = renderer
            .render(&mut target, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full()])
            .expect("clear");
        frame
            .render_texture_from_to(
                texture,
                Rectangle::from_size((16.0, 16.0).into()),
                full(),
                &[full()],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("render texture");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    let mapping = renderer
        .copy_framebuffer(
            &target,
            Rectangle::from_size((SIZE, SIZE).into()),
            Fourcc::Abgr8888,
        )
        .expect("copy framebuffer");
    let data = renderer.map_texture(&mapping).expect("map").to_vec();
    pixel(&data, 8, 8)
}

/// A 16x16 green Xrgb8888 dmabuf whose X byte is 0, imported as a texture.
fn xrgb_dmabuf_alpha_zero(renderer: &mut SkiaVkRenderer) -> super::SkiaVkTexture {
    // The renderer cannot draw into Xrgb8888, so
    // an Argb8888 buffer gets green with alpha 0 (a clear is premultiplied
    // and would store black, so it is drawn from an unpremultiplied
    // texture over a transparent clear) and its planes are wrapped as
    // Xrgb8888, which is the X byte at 0.
    let phd = first_physical_device();
    let node = phd.render_node().ok().flatten().expect("render node");
    let path = node.dev_path().expect("render node path");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open render node");
    let fd = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
    let gbm = GbmDevice::new(fd).expect("gbm device");
    let mut allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING);
    let mut modifiers: Vec<Modifier> = renderer
        .dmabuf_render_formats()
        .iter()
        .filter(|format| format.code == Fourcc::Argb8888)
        .map(|format| format.modifier)
        .collect();
    assert!(!modifiers.is_empty(), "Argb8888 is renderable");
    modifiers.retain(|modifier| *modifier != Modifier::Invalid);
    let buffer = allocator
        .create_buffer(16, 16, Fourcc::Argb8888, &modifiers)
        .expect("gbm buffer");
    let mut dmabuf = buffer.export().expect("export dmabuf");
    // Abgr8888 is R, G, B, A in memory; green with alpha 0 lands as is.
    let green_alpha_zero = renderer
        .import_memory(
            &[0u8, 255, 0, 0].repeat(16 * 16),
            Fourcc::Abgr8888,
            (16, 16).into(),
            false,
        )
        .expect("import memory");
    let mut framebuffer = renderer.bind(&mut dmabuf).expect("bind dmabuf");
    let sync = {
        let mut frame = renderer
            .render(&mut framebuffer, (16, 16).into(), Transform::Normal)
            .expect("frame");
        let all = Rectangle::from_size((16, 16).into());
        frame
            .clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[all])
            .expect("clear");
        frame
            .render_texture_from_to(
                &green_alpha_zero,
                Rectangle::from_size((16.0, 16.0).into()),
                all,
                &[all],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("render texture");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    let mut builder = Dmabuf::builder(
        (16, 16),
        Fourcc::Xrgb8888,
        dmabuf.format().modifier,
        DmabufFlags::empty(),
    );
    for ((fd, offset), stride) in dmabuf.handles().zip(dmabuf.offsets()).zip(dmabuf.strides()) {
        builder.add_plane(
            fd.try_clone_to_owned().expect("dup plane fd"),
            offset,
            stride,
        );
    }
    let xrgb = builder.build().expect("xrgb view of the buffer");
    renderer.import_dmabuf(&xrgb, None).expect("import dmabuf")
}

/// An `X` format is offered next to its alpha twin and draws opaque
/// whatever the padding byte holds. The device only lists alpha formats;
/// without the twins XWayland has no format for a depth-24 window, so it
/// never attaches a buffer and the window stays empty.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_x_formats_are_offered_and_draw_opaque() {
    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let offered = |code: Fourcc| renderer.dmabuf_formats().iter().any(|f| f.code == code);
    assert!(offered(Fourcc::Argb8888), "Argb8888 is a texture format");
    assert!(offered(Fourcc::Xrgb8888), "Xrgb8888 comes with it");
    assert!(offered(Fourcc::Xbgr8888), "Xbgr8888 comes with Abgr8888");

    // Xrgb8888 is B, G, R, X in memory; X left at 0.
    let green: Vec<u8> = [0u8, 255, 0, 0].repeat(16 * 16);
    let texture = renderer
        .import_memory(&green, Fourcc::Xrgb8888, (16, 16).into(), false)
        .expect("import memory");
    assert_eq!(
        sample(&mut renderer, &texture),
        [0, 255, 0, 255],
        "memory texture with a zero X byte"
    );

    // The same through a dmabuf.
    let texture = xrgb_dmabuf_alpha_zero(&mut renderer);
    assert_eq!(
        sample(&mut renderer, &texture),
        [0, 255, 0, 255],
        "dmabuf texture with a zero X byte"
    );
}

/// Windows draw their texture through an image shader with the layer's
/// sampling filter. Every filter, at 1:1 and upscaled, has to show a
/// zero-X-byte dmabuf as opaque.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_opaque_dmabuf_is_opaque_with_every_filter() {
    use layers::skia;

    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let texture = xrgb_dmabuf_alpha_zero(&mut renderer);
    let filters = [
        ("nearest", skia::SamplingOptions::default()),
        (
            "linear",
            skia::SamplingOptions::new(skia::FilterMode::Linear, skia::MipmapMode::None),
        ),
        (
            "bicubic",
            skia::SamplingOptions::from(skia::CubicResampler::catmull_rom()),
        ),
    ];
    let mut failures = Vec::new();
    for (name, sampling) in filters {
        for draw_size in [16.0f32, 48.0] {
            let mut target = renderer
                .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
                .expect("offscreen target");
            let sync = {
                let mut frame = renderer
                    .render(&mut target, (SIZE, SIZE).into(), Transform::Normal)
                    .expect("frame");
                frame
                    .clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[full()])
                    .expect("clear");
                let scale = draw_size / 16.0;
                let mut matrix = skia::Matrix::new_identity();
                matrix.pre_scale((scale, scale), None);
                let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
                paint.set_shader(
                    texture
                        .image
                        .to_shader(
                            (skia::TileMode::Clamp, skia::TileMode::Clamp),
                            sampling,
                            &matrix,
                        )
                        .map(|shader| {
                            if texture.padding_alpha {
                                crate::renderer::draw::opaque_shader(shader)
                            } else {
                                shader
                            }
                        }),
                );
                frame
                    .skia_surface
                    .canvas()
                    .draw_rect(skia::Rect::from_wh(draw_size, draw_size), &paint);
                frame.finish().expect("finish")
            };
            sync.wait().expect("sync point signals");
            let mapping = renderer
                .copy_framebuffer(
                    &target,
                    Rectangle::from_size((SIZE, SIZE).into()),
                    Fourcc::Abgr8888,
                )
                .expect("copy framebuffer");
            let data = renderer.map_texture(&mapping).expect("map").to_vec();
            let centre = (draw_size / 2.0) as i32;
            let got = pixel(&data, centre, centre);
            println!("{name} 16->{draw_size}: {got:?}");
            if got != [0, 255, 0, 255] {
                failures.push(format!("{name} at 16->{draw_size}: {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "see-through draws: {failures:#?}");
}

/// Exposé draws every window through a mirror of an `image_cached` subtree.
/// The mirror has to show the window's texture like the window itself does.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_image_cached_mirror_shows_the_texture() {
    use layers::{drawing::render_node_tree, engine::Engine, prelude::taffy, types::Size};

    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");
    let green: Vec<u8> = [0u8, 255, 0, 255].repeat(16 * 16);
    let texture = renderer
        .import_memory(&green, Fourcc::Abgr8888, (16, 16).into(), false)
        .expect("import memory");

    let engine = Engine::create(SIZE as f32, SIZE as f32);
    let root = engine.new_layer();
    root.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    root.set_size(Size::points(SIZE as f32, SIZE as f32), None);
    engine.add_layer(&root).unwrap();

    // The window: an image-cached layer whose child draws the texture.
    let window = engine.new_layer();
    window.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    window.set_size(Size::points(16.0, 16.0), None);
    window.set_image_cached(true);
    engine.append_layer(&window, root.id).unwrap();
    let surface = engine.new_layer();
    surface.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    surface.set_size(Size::points(16.0, 16.0), None);
    let image = texture.image.clone();
    surface.set_draw_content(move |canvas: &layers::skia::Canvas, w: f32, h: f32| {
        canvas.draw_image_rect(
            &image,
            None,
            layers::skia::Rect::from_wh(w, h),
            &layers::skia::Paint::default(),
        );
        layers::skia::Rect::from_wh(w, h)
    });
    engine.append_layer(&surface, window.id).unwrap();

    // The exposé preview: a mirror of the window at the opposite corner.
    let preview = engine.new_layer();
    preview.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    preview.set_position((32.0, 32.0), None);
    preview.set_size(Size::points(16.0, 16.0), None);
    preview.set_draw_content(window.as_content());
    preview.set_picture_cached(false);
    window.add_follower_node(&preview);
    engine.append_layer(&preview, root.id).unwrap();

    let read = |renderer: &mut SkiaVkRenderer, target: &mut _| {
        let mapping = renderer
            .copy_framebuffer(
                target,
                Rectangle::from_size((SIZE, SIZE).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        renderer.map_texture(&mapping).expect("map").to_vec()
    };

    for frame_no in 0..3 {
        engine.update(0.016);
        let sync = {
            let mut frame = renderer
                .render(&mut target, (SIZE, SIZE).into(), Transform::Normal)
                .expect("frame");
            frame
                .clear(Color32F::new(1.0, 0.0, 0.0, 1.0), &[full()])
                .expect("clear");
            let canvas = frame.skia_surface.canvas();
            let scene = engine.scene();
            scene.with_arena(|arena| {
                scene.with_renderable_arena(|renderables| {
                    let mut region = layers::skia::Region::new();
                    region.set_rect(layers::skia::IRect::from_wh(SIZE, SIZE));
                    render_node_tree(
                        root.id,
                        arena,
                        renderables,
                        canvas,
                        1.0,
                        None,
                        Some(&region),
                        None,
                    );
                })
            });
            frame.finish().expect("finish")
        };
        sync.wait().expect("sync point signals");
        engine.clear_damage();
        let data = read(&mut renderer, &mut target);
        assert_eq!(
            pixel(&data, 8, 8),
            [0, 255, 0, 255],
            "frame {frame_no}: the window shows the texture"
        );
        assert_eq!(
            pixel(&data, 40, 40),
            [0, 255, 0, 255],
            "frame {frame_no}: the mirror shows the texture"
        );
    }
}

/// A depth-24 XWayland window is an Xrgb8888 dmabuf whose X byte is 0. Drawn
/// through the window layer tree (image shader in a picture-cached layer,
/// image-cached parent, exposé mirror) it has to stay opaque.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_layer_tree_draws_xrgb_dmabuf_opaque() {
    use layers::{drawing::render_node_tree, engine::Engine, prelude::taffy, skia, types::Size};

    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");
    let texture = xrgb_dmabuf_alpha_zero(&mut renderer);

    let engine = Engine::create(SIZE as f32, SIZE as f32);
    let root = engine.new_layer();
    root.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    root.set_size(Size::points(SIZE as f32, SIZE as f32), None);
    engine.add_layer(&root).unwrap();

    let window = engine.new_layer();
    window.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    window.set_size(Size::points(16.0, 16.0), None);
    window.set_image_cached(true);
    engine.append_layer(&window, root.id).unwrap();
    let surface = engine.new_layer();
    surface.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    surface.set_size(Size::points(16.0, 16.0), None);
    assert!(texture.padding_alpha, "an X format's alpha byte is padding");
    let image = texture.image.clone();
    let padding_alpha = texture.padding_alpha;
    surface.set_draw_content(move |canvas: &skia::Canvas, w: f32, h: f32| {
        let mut matrix = skia::Matrix::new_identity();
        matrix.pre_scale((w / 16.0, h / 16.0), None);
        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_shader(
            image
                .to_shader(
                    (skia::TileMode::Clamp, skia::TileMode::Clamp),
                    skia::SamplingOptions::from(skia::CubicResampler::catmull_rom()),
                    &matrix,
                )
                .map(|shader| {
                    if padding_alpha {
                        crate::renderer::draw::opaque_shader(shader)
                    } else {
                        shader
                    }
                }),
        );
        canvas.draw_rect(skia::Rect::from_wh(w, h), &paint);
        skia::Rect::from_wh(w, h)
    });
    engine.append_layer(&surface, window.id).unwrap();

    let preview = engine.new_layer();
    preview.set_layout_style(taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    });
    preview.set_position((32.0, 32.0), None);
    preview.set_size(Size::points(16.0, 16.0), None);
    preview.set_draw_content(window.as_content());
    preview.set_picture_cached(false);
    window.add_follower_node(&preview);
    engine.append_layer(&preview, root.id).unwrap();

    let render_round = |renderer: &mut SkiaVkRenderer, target: &mut _| {
        engine.update(0.016);
        let sync = {
            let mut frame = renderer
                .render(target, (SIZE, SIZE).into(), Transform::Normal)
                .expect("frame");
            frame
                .clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &[full()])
                .expect("clear");
            let canvas = frame.skia_surface.canvas();
            let scene = engine.scene();
            scene.with_arena(|arena| {
                scene.with_renderable_arena(|renderables| {
                    let mut region = skia::Region::new();
                    region.set_rect(skia::IRect::from_wh(SIZE, SIZE));
                    render_node_tree(
                        root.id,
                        arena,
                        renderables,
                        canvas,
                        1.0,
                        None,
                        Some(&region),
                        None,
                    );
                })
            });
            frame.finish().expect("finish")
        };
        sync.wait().expect("sync point signals");
        engine.clear_damage();
        let mapping = renderer
            .copy_framebuffer(
                target,
                Rectangle::from_size((SIZE, SIZE).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        let data = renderer.map_texture(&mapping).expect("map").to_vec();
        (pixel(&data, 8, 8), pixel(&data, 40, 40))
    };

    let mut failures = Vec::new();
    for round in 0..3 {
        let (win, mirror) = render_round(&mut renderer, &mut target);
        println!("round {round} (image_cached): window {win:?} mirror {mirror:?}");
        if win != [0, 255, 0, 255] {
            failures.push(format!("round {round} (image_cached) window: {win:?}"));
        }
        if mirror != [0, 255, 0, 255] {
            failures.push(format!("round {round} (image_cached) mirror: {mirror:?}"));
        }
    }

    window.set_image_cached(false);
    let (win, mirror) = render_round(&mut renderer, &mut target);
    println!("round 3 (not image_cached): window {win:?} mirror {mirror:?}");
    if win != [0, 255, 0, 255] {
        failures.push(format!("round 3 (not image_cached) window: {win:?}"));
    }
    if mirror != [0, 255, 0, 255] {
        failures.push(format!("round 3 (not image_cached) mirror: {mirror:?}"));
    }

    assert!(failures.is_empty(), "see-through draws: {failures:#?}");
}

/// Exposé draws client dmabufs into a plane surface, between composite
/// frames. The plane buffer has to show the client's pixels every round.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_plane_surface_draws_dmabuf_textures() {
    use layers::skia;

    let phd = first_physical_device();
    let mut renderer = SkiaVkRenderer::new(&phd).expect("renderer");

    let node = phd.render_node().ok().flatten().expect("render node");
    let path = node.dev_path().expect("render node path");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open render node");
    let fd = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
    let gbm = GbmDevice::new(fd).expect("gbm device");
    let mut allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING);
    let mut modifiers: Vec<Modifier> = renderer
        .dmabuf_render_formats()
        .iter()
        .filter(|format| format.code == Fourcc::Argb8888)
        .map(|format| format.modifier)
        .collect();
    assert!(!modifiers.is_empty(), "Argb8888 is renderable");
    modifiers.retain(|modifier| *modifier != Modifier::Invalid);

    // The client buffer: green.
    let client = allocator
        .create_buffer(SIZE as u32, SIZE as u32, Fourcc::Argb8888, &modifiers)
        .expect("gbm buffer");
    let mut client = client.export().expect("export dmabuf");
    let mut framebuffer = renderer.bind(&mut client).expect("bind dmabuf");
    let sync = {
        let mut frame = renderer
            .render(&mut framebuffer, (SIZE, SIZE).into(), Transform::Normal)
            .expect("frame");
        frame
            .draw_solid(full(), &[full()], Color32F::new(0.0, 1.0, 0.0, 1.0))
            .expect("draw solid");
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    drop(framebuffer);

    // The plane buffer, wrapped once like a plane slot.
    let plane = allocator
        .create_buffer(SIZE as u32, SIZE as u32, Fourcc::Argb8888, &modifiers)
        .expect("gbm buffer");
    let plane = plane.export().expect("export dmabuf");
    let (mut surface, release) = renderer
        .create_surface_from_dmabuf(&plane)
        .expect("plane surface");

    let mut composite = renderer
        .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
        .expect("offscreen target");

    for round in 0..3 {
        // Otto re-imports the client buffer on commit.
        let tex = renderer
            .import_dmabuf(&client, None)
            .expect("import client dmabuf");

        if round > 0 {
            // A composite frame drawing the same texture, as exposé
            // interleaves them with plane renders.
            let sync = {
                let mut frame = renderer
                    .render(&mut composite, (SIZE, SIZE).into(), Transform::Normal)
                    .expect("frame");
                frame
                    .clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &[full()])
                    .expect("clear");
                frame
                    .render_texture_from_to(
                        &tex,
                        Rectangle::from_size((SIZE as f64, SIZE as f64).into()),
                        full(),
                        &[full()],
                        &[],
                        Transform::Normal,
                        1.0,
                    )
                    .expect("render texture");
                frame.finish().expect("finish")
            };
            sync.wait().expect("sync point signals");
            let mapping = renderer
                .copy_framebuffer(
                    &composite,
                    Rectangle::from_size((SIZE, SIZE).into()),
                    Fourcc::Abgr8888,
                )
                .expect("copy framebuffer");
            let data = renderer.map_texture(&mapping).expect("map").to_vec();
            assert_eq!(
                pixel(&data, SIZE / 2, SIZE / 2),
                [0, 255, 0, 255],
                "round {round}: the composite frame shows the client"
            );
        }

        // The plane render, as SceneDmabufElement does it.
        surface.gr_context.reset(None);
        {
            let canvas = surface.surface.canvas();
            canvas.clear(skia::Color4f::new(1.0, 0.0, 0.0, 1.0));
            canvas.draw_image_rect(
                &tex.image,
                None,
                skia::Rect::from_wh(SIZE as f32, SIZE as f32),
                &skia::Paint::default(),
            );
        }
        let skia_surface = &mut surface;
        skia_surface
            .gr_context
            .flush_and_submit_surface(&mut skia_surface.surface, skia::gpu::SyncCpu::No);
        renderer.flush_planes_for_scanout();

        // Read the plane buffer back through an import.
        let copied = renderer
            .import_dmabuf(&plane, None)
            .expect("import plane dmabuf");
        let mut readback = renderer
            .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
            .expect("offscreen target");
        let sync = {
            let mut frame = renderer
                .render(&mut readback, (SIZE, SIZE).into(), Transform::Normal)
                .expect("frame");
            frame
                .clear(Color32F::new(0.0, 0.0, 1.0, 1.0), &[full()])
                .expect("clear");
            frame
                .render_texture_from_to(
                    &copied,
                    Rectangle::from_size((SIZE as f64, SIZE as f64).into()),
                    full(),
                    &[full()],
                    &[],
                    Transform::Normal,
                    1.0,
                )
                .expect("render texture");
            frame.finish().expect("finish")
        };
        sync.wait().expect("sync point signals");
        let mapping = renderer
            .copy_framebuffer(
                &readback,
                Rectangle::from_size((SIZE, SIZE).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        let data = renderer.map_texture(&mapping).expect("map").to_vec();
        for (x, y) in [(SIZE / 2, SIZE / 2), (1, 1), (SIZE - 2, SIZE - 2)] {
            assert_eq!(
                pixel(&data, x, y),
                [0, 255, 0, 255],
                "round {round}: the plane buffer shows the client at {x},{y}"
            );
        }
    }

    drop(surface);
    drop(release);
    renderer.flush_planes_for_scanout();
}

/// After a submit, a sampled client dmabuf belongs to the foreign queue
/// family again, so the client and KMS can use it without an acquire.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_sampled_dmabuf_returns_to_foreign_queue() {
    use layers::skia::gpu::vk as skvk;
    use smithay::reexports::ash::vk;

    let phd = first_physical_device();
    let mut renderer = SkiaVkRenderer::new(&phd).expect("vulkan renderer");
    let texture = xrgb_dmabuf_alpha_zero(&mut renderer);
    assert_eq!(sample(&mut renderer, &texture)[1], 255, "green is drawn");

    let (_, backend) = renderer.sampled.last().cloned().expect("tracked texture");
    let state = skvk::mutable_texture_states::new_vulkan(
        skvk::ImageLayout::GENERAL,
        vk::QUEUE_FAMILY_FOREIGN_EXT,
    );
    let previous = renderer
        .ctx()
        .set_backend_texture_state_and_return_previous(&backend, &state)
        .expect("texture state");
    assert_eq!(
        skvk::mutable_texture_states::get_vk_queue_family_index(&previous),
        vk::QUEUE_FAMILY_FOREIGN_EXT
    );

    // Sampling again acquires it anew and releases it after the submit.
    assert_eq!(
        sample(&mut renderer, &texture)[1],
        255,
        "green is drawn again"
    );
}

/// A sync file waited on by the GPU holds back the frames submitted after
/// it, and the frame still renders.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_gpu_wait_orders_later_frames() {
    use smithay::backend::renderer::sync::Fence;

    let phd = first_physical_device();
    let mut renderer = SkiaVkRenderer::new(&phd).expect("vulkan renderer");
    assert!(
        renderer.sync_fd.import && renderer.sync_fd.export,
        "SYNC_FD semaphores"
    );
    let texture = xrgb_dmabuf_alpha_zero(&mut renderer);
    for _ in 0..3 {
        let fence = renderer
            .sync_pool
            .signal(&renderer.device, renderer.sync_fd)
            .expect("signal")
            .expect("sync file");
        renderer
            .sync_pool
            .wait(&renderer.device, fence.export().expect("dup sync file"))
            .expect("GPU wait");
        assert_eq!(sample(&mut renderer, &texture)[1], 255, "green is drawn");
    }
}

/// An NV12 buffer is offered, imports, and samples as RGB through the
/// YCbCr conversion (BT.601, limited range).
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_nv12_dmabuf_samples_as_rgb() {
    use std::os::fd::AsRawFd;

    let phd = first_physical_device();
    let mut renderer = SkiaVkRenderer::new(&phd).expect("vulkan renderer");
    assert!(
        renderer
            .dmabuf_formats()
            .iter()
            .any(|format| format.code == Fourcc::Nv12 && format.modifier == Modifier::Linear),
        "linear NV12 is offered"
    );

    let node = phd.render_node().ok().flatten().expect("render node");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(node.dev_path().expect("render node path"))
        .expect("open render node");
    let gbm =
        GbmDevice::new(DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)))).expect("gbm device");
    // GBM will not allocate NV12 everywhere; an R8 buffer 1.5 times as tall
    // holds both planes, one buffer object like a decoder's. 64 rows keep
    // the chroma plane on a tile boundary for tiled modifiers.
    let modifier = std::env::var("OTTO_NV12_MODIFIER")
        .ok()
        .and_then(|m| u64::from_str_radix(m.trim_start_matches("0x"), 16).ok())
        .map(Modifier::from)
        .unwrap_or(Modifier::Linear);
    eprintln!("modifier {modifier:?}");
    let mut allocator = GbmAllocator::new(gbm, GbmBufferFlags::empty());
    let buffer = allocator
        .create_buffer(64, 96, Fourcc::R8, &[modifier])
        .expect("gbm R8 buffer");
    let r8 = buffer.export().expect("export dmabuf");
    let stride = r8.strides().next().expect("stride");
    let chroma_offset = stride * 64;
    let mut builder = Dmabuf::builder((64, 64), Fourcc::Nv12, modifier, DmabufFlags::empty());
    for offset in [0, chroma_offset] {
        let fd = r8.handles().next().expect("plane fd");
        builder.add_plane(fd.try_clone_to_owned().expect("dup fd"), offset, stride);
    }
    let dmabuf = builder.build().expect("NV12 view of the buffer");

    // Y = 81, Cb = 90, Cr = 240 is pure red in BT.601 limited range; the
    // planes are filled whole, so the tiling does not matter.
    let len = (stride * 96) as usize;
    let fd = dmabuf.handles().next().expect("plane fd").as_raw_fd();
    // SAFETY: a shared mapping of the whole buffer; unmapped below.
    unsafe {
        let map = libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        assert_ne!(map, libc::MAP_FAILED, "map the NV12 buffer");
        let bytes = std::slice::from_raw_parts_mut(map.cast::<u8>(), len);
        let (luma, chroma) = bytes.split_at_mut(chroma_offset as usize);
        luma.fill(81);
        for pair in chroma.chunks_exact_mut(2) {
            pair.copy_from_slice(&[90, 240]);
        }
        libc::munmap(map, len);
    }

    let texture = renderer.import_dmabuf(&dmabuf, None).expect("import NV12");
    let [r, g, b, a] = sample(&mut renderer, &texture);
    assert!(
        r > 230 && g < 25 && b < 25 && a == 255,
        "red, got {:?}",
        [r, g, b, a]
    );

    // Windows draw their surfaces scaled, with the filter the mapping needs.
    use layers::skia;
    let samplings = [
        ("nearest", skia::SamplingOptions::default()),
        (
            "linear",
            skia::SamplingOptions::new(skia::FilterMode::Linear, skia::MipmapMode::None),
        ),
        (
            "cubic",
            skia::SamplingOptions::from(skia::CubicResampler::catmull_rom()),
        ),
    ];
    for (name, sampling) in samplings {
        let target = renderer
            .create_buffer(Fourcc::Abgr8888, (SIZE, SIZE).into())
            .expect("offscreen target");
        let mut surface = target.skia_surface.surface.clone();
        let canvas = surface.canvas();
        canvas.clear(skia::Color::TRANSPARENT);
        canvas.draw_image_rect_with_sampling_options(
            &texture.image,
            None,
            skia::Rect::from_wh(12.0, 12.0),
            sampling,
            &skia::Paint::default(),
        );
        renderer
            .submit_target(&target)
            .expect("submit")
            .wait()
            .expect("sync");
        let mapping = renderer
            .copy_framebuffer(
                &target,
                Rectangle::from_size((SIZE, SIZE).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        let data = renderer.map_texture(&mapping).expect("map").to_vec();
        let [r, g, b, a] = pixel(&data, 6, 6);
        eprintln!("{name}: {:?}", [r, g, b, a]);
        assert!(
            r > 230 && g < 25 && b < 25 && a == 255,
            "{name}: red, got {:?}",
            [r, g, b, a]
        );
    }
}

/// nongio/otto#361: the wallpaper is painted as an image shader scaled to
/// cover the output. While a window is dragged only the damage rect around it
/// is repainted, and Skia crops the wallpaper's fill to that rect, so the GPU
/// rasterises a different pair of triangles than it did for the whole screen.
/// Along their shared diagonal the interpolated texture coordinates differ by
/// rounding; with nearest sampling at a fractional scale that lands on the
/// other side of a texel boundary, and the repainted band tears along a
/// diagonal starting at the rect's corner.
///
/// Paints the same wallpaper whole and then inside a window-sized clip, and
/// returns `(mismatches inside the clip, of which near a diagonal, mismatches
/// outside the clip)` between the two framebuffers.
fn wallpaper_partial_repaint_mismatches(
    sampling: layers::skia::SamplingOptions,
) -> (usize, usize, usize) {
    use layers::skia;

    const W: i32 = 1280;
    const H: i32 = 800;
    // Neither dimension divides the output: a fractional cover scale and a
    // fractional centring offset, the common case for a photo wallpaper.
    const IW: i32 = 999;
    const IH: i32 = 701;

    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (W, H).into())
        .expect("offscreen target");

    // Every texel differs from its neighbours, so picking the wrong one
    // changes the pixel.
    let mut pixels = Vec::with_capacity((IW * IH * 4) as usize);
    let mut seed: u32 = 0x9E37_79B9;
    for _ in 0..IW * IH {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        pixels.extend_from_slice(&[
            (seed & 0xff) as u8,
            ((seed >> 8) & 0xff) as u8,
            ((seed >> 16) & 0xff) as u8,
            255,
        ]);
    }
    let info = skia::ImageInfo::new(
        (IW, IH),
        skia::ColorType::RGBA8888,
        skia::AlphaType::Premul,
        None,
    );
    let image =
        skia::images::raster_from_data(&info, skia::Data::new_copy(&pixels), (IW * 4) as usize)
            .expect("raster image");

    // The cover transform exactly as `view_background` computes it.
    let paint_wallpaper = |canvas: &skia::Canvas| {
        let (w, h) = (W as f32, H as f32);
        let scale = (w / IW as f32).max(h / IH as f32);
        let offset_x = (w - IW as f32 * scale) / 2.0;
        let offset_y = (h - IH as f32 * scale) / 2.0;
        let mut matrix = skia::Matrix::new_identity();
        matrix.set_scale_translate((scale, scale), (offset_x, offset_y));
        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_shader(image.to_shader(
            (skia::TileMode::Repeat, skia::TileMode::Repeat),
            sampling,
            &matrix,
        ));
        canvas.draw_rect(skia::Rect::from_wh(w, h), &paint);
    };
    let read = |renderer: &mut SkiaVkRenderer, target: &_| -> Vec<u8> {
        let mapping = renderer
            .copy_framebuffer(
                target,
                Rectangle::from_size((W, H).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        renderer.map_texture(&mapping).expect("map").to_vec()
    };

    // Frame 1: the whole wallpaper, as the first frame after a wallpaper
    // change paints it.
    let sync = {
        let mut frame = renderer
            .render(&mut target, (W, H).into(), Transform::Normal)
            .expect("frame");
        frame
            .clear(
                Color32F::new(0.0, 0.0, 0.0, 1.0),
                &[Rectangle::from_size((W, H).into())],
            )
            .expect("clear");
        paint_wallpaper(frame.skia_surface.canvas());
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    let whole = read(&mut renderer, &target);

    // Frame 2: a drag frame. A 300x260 window at (511, 333) plus the 100px
    // shadow safe area on every side is the damage rect, and only it is
    // repainted — `SceneElement::draw` clips to each damage rect and runs
    // the tree again under it.
    let clip = skia::IRect::from_xywh(411, 233, 500, 460);
    let sync = {
        let mut frame = renderer
            .render(&mut target, (W, H).into(), Transform::Normal)
            .expect("frame");
        let canvas = frame.skia_surface.canvas();
        let save = canvas.save();
        canvas.clip_irect(clip, Some(skia::ClipOp::Intersect));
        paint_wallpaper(canvas);
        canvas.restore_to_count(save);
        frame.finish().expect("finish")
    };
    sync.wait().expect("sync point signals");
    let partial = read(&mut renderer, &target);

    let (mut inside, mut near_diagonal, mut outside) = (0usize, 0usize, 0usize);
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            if whole[i..i + 4] == partial[i..i + 4] {
                continue;
            }
            let in_clip = x >= clip.left && x < clip.right && y >= clip.top && y < clip.bottom;
            if !in_clip {
                outside += 1;
                continue;
            }
            inside += 1;
            let fx = (x - clip.left) as f32 / clip.width() as f32;
            let fy = (y - clip.top) as f32 / clip.height() as f32;
            if (fx - fy).abs() < 0.03 || (fx + fy - 1.0).abs() < 0.03 {
                near_diagonal += 1;
            }
        }
    }
    (inside, near_diagonal, outside)
}

/// Diagnostic for nongio/otto#361, meant to be run on the affected GPU with
/// `--nocapture`: prints how the band repainted with the production sampling
/// (`SamplingOptions::default()` is nearest, no mipmaps) differs from the
/// whole-screen paint. On Intel (anv) it is identical; the report from an AMD
/// iGPU decides whether radv rasterises the two passes differently.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_nearest_partial_repaint_report() {
    let (inside, near_diagonal, outside) =
        wallpaper_partial_repaint_mismatches(layers::skia::SamplingOptions::default());
    eprintln!(
        "nearest: {inside} pixels differ inside the damage rect, {near_diagonal} of them within 3% of a diagonal; {outside} outside"
    );
    assert_eq!(outside, 0, "pixels outside the damage rect are untouched");
}

/// The same diagnostic with linear sampling, the candidate fix. Bilinear
/// weights round differently between the two passes, so a handful of pixels
/// one shade apart is expected; the number to compare is the nearest one.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_linear_partial_repaint_report() {
    let (inside, near_diagonal, outside) =
        wallpaper_partial_repaint_mismatches(layers::skia::SamplingOptions::new(
            layers::skia::FilterMode::Linear,
            layers::skia::MipmapMode::None,
        ));
    eprintln!(
        "linear: {inside} pixels differ inside the damage rect, {near_diagonal} of them within 3% of a diagonal; {outside} outside"
    );
    assert_eq!(outside, 0, "pixels outside the damage rect are untouched");
}

/// nongio/otto#361 at scene level: the wallpaper layer as `BackgroundView`
/// builds it (picture-cached content, 24px rounded corners, black backing,
/// opaque hint) under an image-cached window with a blurred shadow child, as
/// `WindowView` builds one. The window is dragged and each frame repaints
/// only the engine's damage rects, as `SceneElement::draw` does; the result
/// has to equal a whole-screen repaint of the same scene.
///
/// Returns, per drag frame, `(pixels differing, of which inside a damage
/// rect, max channel delta)`.
fn wallpaper_scene_drag_mismatches(
    sampling: layers::skia::SamplingOptions,
) -> Vec<(usize, usize, u8)> {
    wallpaper_scene_drag_mismatches_at(sampling, 1280, 800, 999, 701, 1.0)
}

/// [`wallpaper_scene_drag_mismatches`] on a `W`x`H` physical output at
/// output `scale`, with an `IW`x`IH` wallpaper. Window geometry is logical
/// and reaches the layers multiplied by `scale`, as Otto does: positions
/// through `snap_position_px` (rounded), sizes left fractional.
#[allow(non_snake_case)]
fn wallpaper_scene_drag_mismatches_at(
    sampling: layers::skia::SamplingOptions,
    W: i32,
    H: i32,
    IW: i32,
    IH: i32,
    scale: f32,
) -> Vec<(usize, usize, u8)> {
    use layers::{
        drawing::render_node_tree,
        engine::Engine,
        prelude::taffy,
        skia,
        types::{BorderRadius, Color, Size},
    };

    let snap = |x: f32, y: f32| (x.round(), y.round());
    let safe_area = 100.0 * scale;

    let mut renderer = SkiaVkRenderer::new(&first_physical_device()).expect("renderer");
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, (W, H).into())
        .expect("offscreen target");

    let mut pixels = Vec::with_capacity((IW * IH * 4) as usize);
    let mut seed: u32 = 0x9E37_79B9;
    for _ in 0..IW * IH {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        pixels.extend_from_slice(&[
            (seed & 0xff) as u8,
            ((seed >> 8) & 0xff) as u8,
            ((seed >> 16) & 0xff) as u8,
            255,
        ]);
    }
    let info = skia::ImageInfo::new(
        (IW, IH),
        skia::ColorType::RGBA8888,
        skia::AlphaType::Premul,
        None,
    );
    let image =
        skia::images::raster_from_data(&info, skia::Data::new_copy(&pixels), (IW * 4) as usize)
            .expect("raster image");

    let absolute = || taffy::Style {
        position: taffy::Position::Absolute,
        ..Default::default()
    };

    let engine = Engine::create(W as f32, H as f32);
    let root = engine.new_layer();
    root.set_layout_style(absolute());
    root.set_size(Size::points(W as f32, H as f32), None);
    engine.add_layer(&root).unwrap();

    // `view_background` + `BackgroundView::new`.
    let background = engine.new_layer();
    background.set_key("background_view");
    background.set_layout_style(absolute());
    background.set_size(Size::points(W as f32, H as f32), None);
    background.set_border_corner_radius(BorderRadius::new_single(24.0 * scale), None);
    background.set_background_color(Color::new_rgba(0.0, 0.0, 0.0, 1.0), None);
    background.set_content_opaque(true);
    let wallpaper = image.clone();
    background.set_draw_content(move |canvas: &skia::Canvas, w: f32, h: f32| {
        let iw = wallpaper.width() as f32;
        let ih = wallpaper.height() as f32;
        let scale = (w / iw).max(h / ih);
        let offset_x = (w - iw * scale) / 2.0;
        let offset_y = (h - ih * scale) / 2.0;
        let mut matrix = skia::Matrix::new_identity();
        matrix.set_scale_translate((scale, scale), (offset_x, offset_y));
        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_shader(wallpaper.to_shader(
            (skia::TileMode::Repeat, skia::TileMode::Repeat),
            sampling,
            &matrix,
        ));
        canvas.draw_rect(skia::Rect::from_xywh(0.0, 0.0, w, h), &paint);
        skia::Rect::from_xywh(0.0, 0.0, w, h)
    });
    engine.append_layer(&background, root.id).unwrap();

    // `WindowView`: an image-cached window layer holding the shadow (itself
    // image-cached, drawn `SAFE_AREA` larger on every side) and the content.
    let (win_w, win_h) = (300.0 * scale, 260.0 * scale);
    let window = engine.new_layer();
    window.set_key("window");
    window.set_layout_style(absolute());
    window.set_size(Size::points(win_w, win_h), None);
    window.set_position(snap(383.0 * scale, 250.0 * scale), None);
    window.set_image_cached(true);
    engine.append_layer(&window, root.id).unwrap();

    let shadow = engine.new_layer();
    shadow.set_key("window_shadow");
    shadow.set_layout_style(absolute());
    shadow.set_position((-safe_area, -safe_area), None);
    shadow.set_size(
        Size::points(win_w + safe_area * 2.0, win_h + safe_area * 2.0),
        None,
    );
    shadow.set_image_cached(true);
    shadow.set_draw_content(move |canvas: &skia::Canvas, w: f32, h: f32| {
        let rrect = skia::RRect::new_rect_xy(
            skia::Rect::from_xywh(
                safe_area,
                safe_area,
                w - safe_area * 2.0,
                h - safe_area * 2.0,
            ),
            12.0 * scale,
            12.0 * scale,
        );
        let mut paint = skia::Paint::new(skia::Color4f::new(0.0, 0.0, 0.0, 0.35), None);
        paint.set_mask_filter(skia::MaskFilter::blur(
            skia::BlurStyle::Normal,
            24.0 * scale,
            false,
        ));
        canvas.draw_rrect(rrect, &paint);
        skia::Rect::from_xywh(0.0, 0.0, w, h)
    });
    engine.append_layer(&shadow, window.id).unwrap();

    let content = engine.new_layer();
    content.set_key("window_content");
    content.set_layout_style(absolute());
    content.set_size(Size::points(win_w, win_h), None);
    content.set_background_color(Color::new_rgba(0.2, 0.2, 0.25, 1.0), None);
    content.set_border_corner_radius(BorderRadius::new_single(12.0 * scale), None);
    engine.append_layer(&content, window.id).unwrap();

    let read = |renderer: &mut SkiaVkRenderer, target: &_| -> Vec<u8> {
        let mapping = renderer
            .copy_framebuffer(
                target,
                Rectangle::from_size((W, H).into()),
                Fourcc::Abgr8888,
            )
            .expect("copy framebuffer");
        renderer.map_texture(&mapping).expect("map").to_vec()
    };
    // `SceneElement::draw`: one pass per damage rect under a rect clip, the
    // rect forwarded as the region so untouched subtrees are culled.
    let render =
        |renderer: &mut SkiaVkRenderer, target: &mut _, rects: &[skia::IRect], clear: bool| {
            let sync = {
                let mut frame = renderer
                    .render(target, (W, H).into(), Transform::Normal)
                    .expect("frame");
                if clear {
                    frame
                        .clear(
                            Color32F::new(0.0, 0.0, 0.0, 1.0),
                            &[Rectangle::from_size((W, H).into())],
                        )
                        .expect("clear");
                }
                let canvas = frame.skia_surface.canvas();
                let scene = engine.scene();
                scene.with_arena(|arena| {
                    scene.with_renderable_arena(|renderables| {
                        for rect in rects {
                            let save = canvas.save();
                            canvas.clip_irect(*rect, Some(skia::ClipOp::Intersect));
                            let mut region = skia::Region::new();
                            region.set_rect(*rect);
                            render_node_tree(
                                root.id,
                                arena,
                                renderables,
                                canvas,
                                1.0,
                                None,
                                Some(&region),
                                None,
                            );
                            canvas.restore_to_count(save);
                        }
                    })
                });
                frame.finish().expect("finish")
            };
            sync.wait().expect("sync point signals");
        };
    let full = [skia::IRect::from_wh(W, H)];

    // Frame 0: the scene settles and paints whole.
    engine.update(0.016);
    render(&mut renderer, &mut target, &full, true);
    engine.clear_damage();

    let mut report = Vec::new();
    for step in 1..=6 {
        // A drag in logical points; `snap_position_px` rounds the physical
        // origin to whole pixels.
        let (lx, ly) = (383.0 + 7.0 * step as f32, 250.0 - 5.0 * step as f32);
        window.set_position(snap(lx * scale, ly * scale), None);
        engine.update(0.016);
        let rects: Vec<skia::IRect> = engine
            .damage_rects()
            .into_iter()
            .filter(|r| !r.is_empty())
            .map(|r| {
                let ir: skia::IRect = skia::RoundOut::round_out(&r);
                skia::IRect::intersect(&ir, &full[0]).unwrap_or(skia::IRect::new_empty())
            })
            .filter(|r| !r.is_empty())
            .collect();
        assert!(!rects.is_empty(), "step {step}: the move damaged something");
        render(&mut renderer, &mut target, &rects, false);
        engine.clear_damage();
        let partial = read(&mut renderer, &target);

        // The reference: the same scene painted whole.
        render(&mut renderer, &mut target, &full, true);
        let whole = read(&mut renderer, &target);

        let (mut differing, mut inside, mut max_delta) = (0usize, 0usize, 0u8);
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 4) as usize;
                let delta = (0..3)
                    .map(|c| whole[i + c].abs_diff(partial[i + c]))
                    .max()
                    .unwrap();
                if delta == 0 {
                    continue;
                }
                differing += 1;
                max_delta = max_delta.max(delta);
                if rects
                    .iter()
                    .any(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                {
                    inside += 1;
                }
            }
        }
        eprintln!(
            "step {step}: damage {rects:?}: {differing} pixels differ ({inside} inside the damage), max channel delta {max_delta}"
        );
        report.push((differing, inside, max_delta));
    }
    report
}

/// The production wallpaper sampling: nearest, no mipmaps. A drag frame
/// repainted per damage rect has to equal the whole repaint exactly.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_scene_drag_nearest() {
    let report = wallpaper_scene_drag_mismatches(layers::skia::SamplingOptions::default());
    let worst = report.iter().map(|(d, _, _)| *d).max().unwrap_or(0);
    assert_eq!(
        worst, 0,
        "a drag frame repainted per damage rect equals the whole repaint"
    );
}

/// Linear sampling, the candidate fix: the two passes may differ by one
/// shade of bilinear rounding, never more.
#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_scene_drag_linear() {
    let report = wallpaper_scene_drag_mismatches(layers::skia::SamplingOptions::new(
        layers::skia::FilterMode::Linear,
        layers::skia::MipmapMode::None,
    ));
    let worst_delta = report.iter().map(|(_, _, d)| *d).max().unwrap_or(0);
    assert!(
        worst_delta <= 1,
        "linear repaints differ by at most one shade, got {worst_delta}"
    );
}

/// The scene drag at a fractional output scale, as the #361 reporter may
/// run: a 2560x1600 panel, a 1920x1080 wallpaper (cover scale 1.48) and
/// window sizes that land between pixels. Prints per-step mismatches and
/// asserts nothing, so it reports on any GPU. A second renderer in the same
/// process crashes, so each case is its own test; run them one at a time.
fn fractional_scale_report(scale: f32, linear: bool) {
    let sampling = if linear {
        layers::skia::SamplingOptions::new(
            layers::skia::FilterMode::Linear,
            layers::skia::MipmapMode::None,
        )
    } else {
        layers::skia::SamplingOptions::default()
    };
    let report = wallpaper_scene_drag_mismatches_at(sampling, 2560, 1600, 1920, 1080, scale);
    let worst = report.iter().map(|(d, _, _)| *d).max().unwrap_or(0);
    let delta = report.iter().map(|(_, _, d)| *d).max().unwrap_or(0);
    let name = if linear { "linear" } else { "nearest" };
    eprintln!("scale {scale} {name}: worst frame {worst} pixels differ, max channel delta {delta}");
}

#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_scale_1_25_nearest() {
    fractional_scale_report(1.25, false);
}

#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_scale_1_33_nearest() {
    fractional_scale_report(1.33, false);
}

#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_scale_1_33_linear() {
    fractional_scale_report(1.33, true);
}

#[test]
#[ignore = "needs a Vulkan GPU"]
fn vulkan_wallpaper_scale_1_5_nearest() {
    fractional_scale_report(1.5, false);
}
