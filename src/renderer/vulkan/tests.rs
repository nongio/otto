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
