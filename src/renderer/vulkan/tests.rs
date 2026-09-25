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

/// Samples `texture` onto a black frame and returns the pixel at (8, 8).
fn sample(renderer: &mut SkiaVkRenderer, texture: &super::SkiaVkTexture) -> [u8; 4] {
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

    // The same through a dmabuf. The renderer cannot draw into Xrgb8888, so
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
    let texture = renderer.import_dmabuf(&xrgb, None).expect("import dmabuf");
    assert_eq!(
        sample(&mut renderer, &texture),
        [0, 255, 0, 255],
        "dmabuf texture with a zero X byte"
    );
}
