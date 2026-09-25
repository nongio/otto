//! Skia on Vulkan, standalone.
//!
//! Proves the pieces the Vulkan renderer plan rests on, without the
//! compositor: Smithay's Vulkan device layer creates the device on the
//! graphics queue, Skia's Ganesh Vulkan backend runs on that device, a
//! lay-rs scene renders into an exportable `VkImage`, the image leaves as a
//! dmabuf and comes back as a sampled texture, and the GPU work is fenced
//! with a sync-file semaphore signalled by an empty queue submit after
//! Skia's own submit.
//!
//! Writes `vulkan_probe.png` (the scene) and `vulkan_probe_import.png`
//! (the scene drawn through the re-imported dmabuf) into the current
//! directory.

use std::{
    ffi::CStr,
    os::fd::{AsRawFd, FromRawFd},
    ptr,
    time::Instant,
};

use layers::{
    prelude::*,
    skia::{
        self,
        gpu::{
            self, backend_render_targets, backend_textures, direct_contexts, surfaces, vk as skvk,
        },
    },
    types::Size,
};
use smithay::{
    backend::{
        allocator::{dmabuf::AsDmabuf, Buffer, Fourcc, Modifier},
        vulkan::{
            device::{Device, QueueType},
            image::VulkanImage,
            version::Version,
            Instance, PhysicalDevice,
        },
    },
    reexports::ash::{self, vk, vk::Handle},
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;

/// The Skia binding names Vulkan formats as its own enum; only the formats
/// this probe can produce are mapped.
fn skia_format(format: vk::Format) -> skvk::Format {
    match format {
        vk::Format::B8G8R8A8_UNORM => skvk::Format::B8G8R8A8_UNORM,
        vk::Format::R8G8B8A8_UNORM => skvk::Format::R8G8B8A8_UNORM,
        vk::Format::B8G8R8A8_SRGB => skvk::Format::B8G8R8A8_SRGB,
        vk::Format::R8G8B8A8_SRGB => skvk::Format::R8G8B8A8_SRGB,
        other => panic!("unmapped vulkan format {other:?}"),
    }
}

fn main() {
    let t0 = Instant::now();
    let stamp = |what: &str| println!("[{:>7.1} ms] {what}", t0.elapsed().as_secs_f64() * 1e3);

    // --- device layer -------------------------------------------------------
    let instance = Instance::new(Version::VERSION_1_3, None).expect("vulkan instance");
    let phd = PhysicalDevice::enumerate(&instance)
        .expect("enumerate")
        .find(|p| matches!(p.render_node(), Ok(Some(_))))
        .expect("a physical device with a render node");
    println!(
        "device: {} ({:?}), render node {:?}",
        phd.name(),
        phd.driver().map(|d| d.name.clone()),
        phd.render_node().ok().flatten().map(|n| n.dev_path())
    );

    let extensions: [&CStr; 6] = [
        ash::khr::external_memory_fd::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::ext::image_drm_format_modifier::NAME,
        ash::khr::external_semaphore_fd::NAME,
        ash::ext::queue_family_foreign::NAME,
        ash::khr::image_format_list::NAME,
    ];
    let mut features = vk::PhysicalDeviceFeatures2::default();
    let device =
        Device::new(&phd, &extensions, &mut features, QueueType::Graphics, false).expect("device");
    let qfi = device.queue_family_idx();
    stamp("device on the graphics queue");

    // --- Skia context on that device -----------------------------------------
    let entry = unsafe { ash::Entry::load() }.expect("libvulkan");
    let get_instance_proc_addr = entry.static_fn().get_instance_proc_addr;
    let get_device_proc_addr = instance.handle().fp_v1_0().get_device_proc_addr;
    let get_proc = |of: skvk::GetProcOf| -> skvk::GetProcResult {
        let f = match of {
            skvk::GetProcOf::Instance(inst, name) => unsafe {
                get_instance_proc_addr(vk::Instance::from_raw(inst as _), name)
            },
            skvk::GetProcOf::Device(dev, name) => unsafe {
                get_device_proc_addr(vk::Device::from_raw(dev as _), name)
            },
        };
        match f {
            Some(f) => f as *const std::ffi::c_void,
            None => {
                eprintln!("get_proc: unresolved {:?}", unsafe { of.name() });
                ptr::null()
            }
        }
    };
    let device_extension_names: Vec<String> = extensions
        .iter()
        .map(|e| e.to_str().unwrap().to_owned())
        .collect();
    let device_extension_refs: Vec<&str> =
        device_extension_names.iter().map(String::as_str).collect();
    let mut backend = unsafe {
        skvk::BackendContext::new_with_extensions(
            instance.handle().handle().as_raw() as _,
            phd.handle().as_raw() as _,
            device.vk().handle().as_raw() as _,
            (device.queue().as_raw() as _, qfi as usize),
            &get_proc,
            &[],
            &device_extension_refs,
        )
    };
    // The instance is capped at 1.3 while the device advertises 1.4; without
    // this Skia asks the driver for 1.4 core entry points that the loader
    // does not hand out under a 1.3 instance.
    backend.set_max_api_version(skvk::Version::from(vk::API_VERSION_1_3));
    let mut ctx = direct_contexts::make_vulkan(&backend, None).expect("skia vulkan context");
    stamp("skia DirectContext (vulkan)");

    // --- exportable render target ---------------------------------------------
    let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
        | vk::ImageUsageFlags::TRANSFER_SRC
        | vk::ImageUsageFlags::TRANSFER_DST
        | vk::ImageUsageFlags::SAMPLED;
    let target = VulkanImage::new_exportable(
        &device,
        WIDTH,
        HEIGHT,
        Fourcc::Argb8888,
        [Modifier::Linear].into_iter(),
        usage,
    )
    .expect("exportable image");
    let target_info = unsafe {
        skvk::ImageInfo::new(
            target.vk().as_raw() as _,
            skvk::Alloc::default(),
            if target.is_linear() {
                skvk::ImageTiling::LINEAR
            } else {
                skvk::ImageTiling::OPTIMAL
            },
            skvk::ImageLayout::UNDEFINED,
            skia_format(target.format()),
            1,
            qfi,
            None,
            None,
            None,
        )
    };
    let rt = backend_render_targets::make_vk((WIDTH as i32, HEIGHT as i32), &target_info);
    let mut surface = surfaces::wrap_backend_render_target(
        &mut ctx,
        &rt,
        gpu::SurfaceOrigin::TopLeft,
        skia::ColorType::BGRA8888,
        None,
        None,
    )
    .expect("wrap render target");
    stamp("VkImage wrapped as a Skia surface");

    // --- a lay-rs scene ----------------------------------------------------------
    let engine = Engine::create(WIDTH as f32, HEIGHT as f32);
    let root = engine.new_layer();
    root.set_size(Size::points(WIDTH as f32, HEIGHT as f32), None);
    root.set_background_color(Color::new_hex("#1e2430"), None);
    let _ = engine.add_layer(&root);
    for (i, hex) in ["#ff6b6b", "#ffd93d", "#6bcB77", "#4d96ff"]
        .iter()
        .enumerate()
    {
        let card = engine.new_layer();
        card.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        card.set_size(Size::points(120.0, 160.0), None);
        card.set_position(Point::new(40.0 + i as f32 * 150.0, 120.0), None);
        card.set_background_color(Color::new_hex(hex), None);
        card.set_border_corner_radius(BorderRadius::new_single(18.0), None);
        let _ = root.add_sublayer(&card);
    }
    engine.update(0.0);
    stamp("lay-rs scene laid out");

    {
        let scene = engine.scene();
        let root_id = engine.scene_root().expect("scene root");
        let canvas = surface.canvas();
        canvas.clear(skia::Color::BLACK);
        scene.with_arena(|arena| {
            scene.with_renderable_arena(|renderables| {
                render_node_tree(root_id, arena, renderables, canvas, 1.0, None, None, None);
            });
        });
    }
    ctx.flush_and_submit();
    stamp("scene recorded and submitted");

    // --- fence the work: a sync-file semaphore signalled after Skia's submit ---
    let sem_fd_ext = device
        .vk_khr_external_semaphore_fd()
        .expect("VK_KHR_external_semaphore_fd");
    let mut export_info = vk::ExportSemaphoreCreateInfo::default()
        .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let sem_info = vk::SemaphoreCreateInfo::default().push_next(&mut export_info);
    let semaphore = unsafe { device.vk().create_semaphore(&sem_info, None) }.expect("semaphore");
    let signal = [semaphore];
    let submit = vk::SubmitInfo::default().signal_semaphores(&signal);
    unsafe {
        device
            .vk()
            .queue_submit(*device.queue(), &[submit], vk::Fence::null())
    }
    .expect("empty submit");
    let fd_info = vk::SemaphoreGetFdInfoKHR::default()
        .semaphore(semaphore)
        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let sync_fd = unsafe { sem_fd_ext.get_semaphore_fd(&fd_info) }.expect("export sync fd");
    let sync_fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(sync_fd) };
    let t_wait = Instant::now();
    let mut pfd = libc::pollfd {
        fd: sync_fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let polled = unsafe { libc::poll(&mut pfd, 1, 5000) };
    assert!(polled == 1, "sync file did not signal within 5 s");
    println!(
        "sync file signalled after {:.2} ms",
        t_wait.elapsed().as_secs_f64() * 1e3
    );
    stamp("GPU work fenced through a sync file");

    // --- read back ---------------------------------------------------------------
    let snapshot = surface.image_snapshot();
    let png = snapshot
        .encode(Some(&mut ctx), skia::EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write("vulkan_probe.png", png.as_bytes()).expect("write png");
    stamp("vulkan_probe.png written");

    // --- export as dmabuf, import back as a texture ----------------------------------
    let dmabuf = target.export().expect("export dmabuf");
    println!(
        "dmabuf: {}x{} {:?} modifier {:?}, {} plane(s)",
        dmabuf.width(),
        dmabuf.height(),
        dmabuf.format().code,
        dmabuf.format().modifier,
        dmabuf.num_planes()
    );
    let imported = VulkanImage::new_from_dmabuf(
        &device,
        &dmabuf,
        vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC
            | vk::ImageUsageFlags::TRANSFER_DST,
    )
    .expect("import dmabuf");
    let imported_info = unsafe {
        skvk::ImageInfo::new(
            imported.vk().as_raw() as _,
            skvk::Alloc::default(),
            skvk::ImageTiling::DRM_FORMAT_MODIFIER_EXT,
            skvk::ImageLayout::UNDEFINED,
            skia_format(imported.format()),
            1,
            vk::QUEUE_FAMILY_FOREIGN_EXT,
            None,
            None,
            None,
        )
    };
    let tex =
        unsafe { backend_textures::make_vk((WIDTH as i32, HEIGHT as i32), &imported_info, "") };
    let tex_image = skia::Image::from_texture(
        &mut ctx,
        &tex,
        gpu::SurfaceOrigin::TopLeft,
        skia::ColorType::BGRA8888,
        skia::AlphaType::Premul,
        None,
    )
    .expect("image from imported texture");
    stamp("dmabuf re-imported as a Skia texture");

    let mut second = surfaces::render_target(
        &mut ctx,
        gpu::Budgeted::Yes,
        &skia::ImageInfo::new_n32_premul((WIDTH as i32, HEIGHT as i32), None),
        None,
        gpu::SurfaceOrigin::TopLeft,
        None,
        false,
        None,
    )
    .expect("offscreen surface");
    {
        let canvas = second.canvas();
        canvas.clear(skia::Color::WHITE);
        let dst = skia::Rect::from_xywh(40.0, 40.0, WIDTH as f32 - 80.0, HEIGHT as f32 - 80.0);
        canvas.draw_image_rect(&tex_image, None, dst, &skia::Paint::default());
    }
    ctx.flush_and_submit();
    let png = second
        .image_snapshot()
        .encode(Some(&mut ctx), skia::EncodedImageFormat::PNG, 100)
        .expect("encode png");
    std::fs::write("vulkan_probe_import.png", png.as_bytes()).expect("write png");
    stamp("vulkan_probe_import.png written");

    drop(tex_image);
    drop(second);
    drop(surface);
    ctx.flush_submit_and_sync_cpu();
    unsafe { device.vk().destroy_semaphore(semaphore, None) };
    stamp("done");
}
