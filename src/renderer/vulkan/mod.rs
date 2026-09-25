//! Skia on Vulkan: the udev renderer of a `vulkan` build.
//!
//! [`SkiaVkRenderer`] implements the Smithay renderer traits the GL
//! [`SkiaRenderer`](crate::skia_renderer::SkiaRenderer) implements, on a
//! Smithay Vulkan [`Device`] with a Skia Ganesh Vulkan context on its
//! graphics queue:
//!
//! - client dmabufs import as `VkImage`s with their explicit DRM format
//!   modifier and are sampled from the foreign queue family;
//! - shared-memory buffers and `import_memory` upload into Skia textures;
//! - dmabufs bind as wrapped render targets and are handed back to the
//!   foreign queue family in `GENERAL` layout after each frame;
//! - a frame's completion is a sync file from an empty submit after Skia's
//!   (see [`sync`]).
//!
//! winit and x11 stay on GL in every build.

// Rust guideline compliant 2026-02-21

mod format;
mod frame;
mod sync;
mod texture;

#[cfg(test)]
mod tests;

pub use frame::SkiaVkFrame;
pub use sync::SkiaVkSync;
pub use texture::{SkiaVkMapping, SkiaVkTarget, SkiaVkTexture};

use std::{
    collections::{HashMap, HashSet},
    ffi::CStr,
    fmt, ptr,
    sync::{Arc, Mutex},
};

use layers::skia::{
    self,
    gpu::{
        self, backend_render_targets, backend_textures, direct_contexts, surfaces, vk as skvk,
        DirectContext,
    },
};
use smithay::{
    backend::{
        allocator::{
            dmabuf::{Dmabuf, WeakDmabuf},
            format::FormatSet,
            Buffer as _, Fourcc,
        },
        renderer::{
            sync::SyncPoint, Bind, Blit, ContextId, DebugFlags, ExportMem, ImportDma, ImportDmaWl,
            ImportMem, ImportMemWl, Offscreen, Renderer, RendererSuper, TextureFilter,
        },
        vulkan::{
            device::{Device, DeviceError, QueueType},
            image::VulkanImage,
            version::Version,
            PhysicalDevice,
        },
        SwapBuffersError,
    },
    reexports::{
        ash::{self, vk, vk::Handle},
        wayland_server::protocol::wl_buffer::WlBuffer,
    },
    utils::{Buffer, Physical, Rectangle, Size, Transform},
    wayland::{compositor::SurfaceData, shm},
};

use self::{
    format::{skia_format, SkiaFormat, FOURCCS},
    sync::{SyncFdSupport, SyncPool},
    texture::{TargetKind, TextureBacking},
};
use crate::renderer::SkiaSurface;

/// Device extensions the renderer needs, all of them passed on to Skia.
const DEVICE_EXTENSIONS: [&CStr; 6] = [
    ash::khr::external_memory_fd::NAME,
    ash::ext::external_memory_dma_buf::NAME,
    ash::ext::image_drm_format_modifier::NAME,
    ash::khr::external_semaphore_fd::NAME,
    ash::ext::queue_family_foreign::NAME,
    ash::khr::image_format_list::NAME,
];

/// Usage of a client dmabuf imported for sampling.
const TEXTURE_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::SAMPLED.as_raw()
        | vk::ImageUsageFlags::TRANSFER_SRC.as_raw()
        | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);

/// Usage of a dmabuf bound as a render target.
const TARGET_USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw()
        | vk::ImageUsageFlags::SAMPLED.as_raw()
        | vk::ImageUsageFlags::TRANSFER_SRC.as_raw()
        | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);

/// Errors of the Vulkan renderer.
#[derive(Debug, thiserror::Error)]
pub enum SkiaVkError {
    /// The physical device lacks a required extension.
    #[error("the Vulkan device lacks {0}")]
    MissingExtension(&'static str),
    /// Creating the logical device failed.
    #[error("creating the Vulkan device failed: {0}")]
    Device(#[from] DeviceError),
    /// `libvulkan` could not be loaded.
    #[error("loading libvulkan failed: {0}")]
    Loader(String),
    /// Skia refused to create a context on the device.
    #[error("Skia could not create a Vulkan context")]
    ContextCreation,
    /// A Vulkan call failed.
    #[error("Vulkan call failed: {0}")]
    Vk(#[from] vk::Result),
    /// Creating or importing a `VkImage` failed.
    #[error("Vulkan image: {0}")]
    Image(#[from] smithay::backend::vulkan::image::Error),
    /// The pixel format has no Skia mapping.
    #[error("unsupported pixel format {0:?}")]
    UnsupportedFormat(Fourcc),
    /// The dmabuf was refused as a render target before.
    #[error("the dmabuf cannot be rendered into")]
    RefusedTarget,
    /// Skia refused to wrap an image or render target.
    #[error("Skia could not wrap the image")]
    Wrap,
    /// Skia could not allocate a surface.
    #[error("Skia could not create a surface")]
    SurfaceCreation,
    /// A memory buffer is smaller than its size and format imply.
    #[error("buffer holds {got} bytes, {expected} expected")]
    BufferSize {
        /// Bytes the buffer should hold.
        expected: usize,
        /// Bytes it holds.
        got: usize,
    },
    /// A shared-memory buffer could not be accessed.
    #[error("shm buffer access failed: {0}")]
    Shm(#[from] shm::BufferAccessError),
    /// Reading pixels back failed.
    #[error("reading pixels back failed")]
    Readback,
    /// Nothing has been rendered or bound yet.
    #[error("no render target is bound")]
    NoTarget,
    /// The texture does not support the operation.
    #[error("the operation is not supported for this texture")]
    Unsupported,
    /// Waiting for a sync point was interrupted.
    #[error("waiting for a sync point was interrupted")]
    SyncInterrupted,
}

impl From<SkiaVkError> for SwapBuffersError {
    fn from(err: SkiaVkError) -> Self {
        match err {
            SkiaVkError::MissingExtension(_)
            | SkiaVkError::Device(_)
            | SkiaVkError::Loader(_)
            | SkiaVkError::ContextCreation
            | SkiaVkError::Vk(vk::Result::ERROR_DEVICE_LOST) => {
                SwapBuffersError::ContextLost(Box::new(err))
            }
            err => SwapBuffersError::TemporaryFailure(Box::new(err)),
        }
    }
}

type PlaneImageQueue = Arc<Mutex<Vec<VulkanImage>>>;

/// Keeps a plane slot's `VkImage` alive while its Skia surface renders into it.
///
/// Dropping it queues the image; the renderer destroys it on the next
/// [`SkiaVkRenderer::flush_planes_for_scanout`], after the GPU has finished
/// every use of it.
pub struct PlaneTextureRelease {
    image: Option<VulkanImage>,
    queue: PlaneImageQueue,
}

impl Drop for PlaneTextureRelease {
    fn drop(&mut self) {
        if let (Some(image), Ok(mut queue)) = (self.image.take(), self.queue.lock()) {
            queue.push(image);
        }
    }
}

/// Per-surface cache of the texture a shared-memory buffer uploads into.
#[derive(Default)]
struct ShmCache(Mutex<Option<(ContextId<SkiaVkTexture>, SkiaVkTexture)>>);

/// Skia Ganesh on a Smithay Vulkan device.
pub struct SkiaVkRenderer {
    /// The Skia context every surface and texture of this renderer lives in.
    ///
    /// Named like the GL renderer's field, which lay-rs is handed. Always
    /// `Some` until the renderer drops.
    pub context: Option<DirectContext>,
    device: Device,
    physical_device: PhysicalDevice,
    context_id: ContextId<SkiaVkTexture>,
    sync_fd: SyncFdSupport,
    sync_pool: SyncPool,
    texture_formats: FormatSet,
    render_formats: FormatSet,
    /// Client dmabufs imported for sampling, keyed weakly so the cache never
    /// keeps a dropped buffer alive.
    dmabuf_cache: HashMap<WeakDmabuf, VulkanImage>,
    /// Dmabufs wrapped as render targets.
    target_cache: HashMap<WeakDmabuf, SkiaVkTarget>,
    /// Dmabufs the device cannot render into, so they are not retried every frame.
    refused_targets: HashSet<WeakDmabuf>,
    /// The target last bound or rendered to; the screenshare blit reads it.
    current_target: Option<SkiaVkTarget>,
    plane_releases: PlaneImageQueue,
    upscale_filter: TextureFilter,
    downscale_filter: TextureFilter,
    debug_flags: DebugFlags,
}

impl fmt::Debug for SkiaVkRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkiaVkRenderer")
            .field("device", &self.physical_device.name())
            .field("sync_fd", &self.sync_fd)
            .finish_non_exhaustive()
    }
}

impl SkiaVkRenderer {
    /// Creates the renderer on `phd`'s graphics queue.
    ///
    /// # Errors
    ///
    /// Fails when the device lacks one of the dmabuf, modifier or sync-file
    /// extensions, when the logical device cannot be created, or when Skia
    /// refuses the device.
    pub fn new(phd: &PhysicalDevice) -> Result<Self, SkiaVkError> {
        for ext in DEVICE_EXTENSIONS {
            if !phd.has_device_extension(ext) {
                return Err(SkiaVkError::MissingExtension(
                    ext.to_str().unwrap_or("a device extension"),
                ));
            }
        }

        let mut features = vk::PhysicalDeviceFeatures2::default();
        let device = Device::new(
            phd,
            &DEVICE_EXTENSIONS,
            &mut features,
            QueueType::Graphics,
            false,
        )?;
        let context = Self::create_context(phd, &device)?;

        let formats_for = |usage: vk::ImageUsageFlags| -> FormatSet {
            device
                .formats()
                .map(|entry| entry.format)
                .filter(|format| skia_format(format.code).is_some())
                .filter(|format| matches!(phd.drm_format_info(*format, usage), Ok(Some(_))))
                .collect()
        };
        let texture_formats = formats_for(TEXTURE_USAGE);
        let render_formats = formats_for(TARGET_USAGE);
        let sync_fd = sync::sync_fd_support(phd);

        tracing::info!(
            "Rendering with Skia on Vulkan: {} ({}), render node {:?}, Vulkan {}, \
             {} texture and {} render formats, sync files: export {} import {}",
            phd.name(),
            phd.driver()
                .map(|d| d.name.as_str())
                .unwrap_or("unknown driver"),
            device.node().and_then(|node| node.dev_path()),
            phd.api_version(),
            texture_formats.iter().count(),
            render_formats.iter().count(),
            sync_fd.export,
            sync_fd.import,
        );

        Ok(Self {
            context: Some(context),
            device,
            physical_device: phd.clone(),
            context_id: ContextId::new(),
            sync_fd,
            sync_pool: SyncPool::default(),
            texture_formats,
            render_formats,
            dmabuf_cache: HashMap::new(),
            target_cache: HashMap::new(),
            refused_targets: HashSet::new(),
            current_target: None,
            plane_releases: PlaneImageQueue::default(),
            upscale_filter: TextureFilter::Linear,
            downscale_filter: TextureFilter::Linear,
            debug_flags: DebugFlags::empty(),
        })
    }

    /// Creates the Skia context on `device`'s queue.
    fn create_context(phd: &PhysicalDevice, device: &Device) -> Result<DirectContext, SkiaVkError> {
        // SAFETY: loading the system Vulkan loader has no preconditions; it
        // is the library Smithay's instance already loaded.
        let entry =
            unsafe { ash::Entry::load() }.map_err(|e| SkiaVkError::Loader(e.to_string()))?;
        let get_instance_proc_addr = entry.static_fn().get_instance_proc_addr;
        let get_device_proc_addr = phd.instance().handle().fp_v1_0().get_device_proc_addr;
        let get_proc = move |of: skvk::GetProcOf| -> skvk::GetProcResult {
            // SAFETY: Skia hands valid instance/device handles and
            // NUL-terminated names.
            let f = unsafe {
                match of {
                    skvk::GetProcOf::Instance(instance, name) => {
                        get_instance_proc_addr(vk::Instance::from_raw(instance as _), name)
                    }
                    skvk::GetProcOf::Device(device, name) => {
                        get_device_proc_addr(vk::Device::from_raw(device as _), name)
                    }
                }
            };
            f.map_or(ptr::null(), |f| f as *const std::ffi::c_void)
        };

        let extensions: Vec<&str> = DEVICE_EXTENSIONS
            .iter()
            .filter_map(|ext| ext.to_str().ok())
            .collect();
        // Skia asks for core entry points of the version it is told; the
        // instance is capped at 1.3, and entry points of a newer device
        // version are not handed out under it.
        let api_version = phd.api_version().min(Version::VERSION_1_3).to_raw();

        // SAFETY: the instance, physical device, device and queue outlive the
        // backend context, which is dropped right after the Skia context is
        // made; the context keeps what it needs.
        let context = unsafe {
            let mut backend = skvk::BackendContext::new_with_extensions(
                phd.instance().handle().handle().as_raw() as _,
                phd.handle().as_raw() as _,
                device.vk().handle().as_raw() as _,
                (
                    device.queue().as_raw() as _,
                    device.queue_family_idx() as usize,
                ),
                &get_proc,
                &[],
                &extensions,
            );
            backend.set_max_api_version(skvk::Version::from(api_version));
            direct_contexts::make_vulkan(&backend, None)
        };
        context.ok_or(SkiaVkError::ContextCreation)
    }

    /// The Skia context.
    fn ctx(&mut self) -> &mut DirectContext {
        self.context
            .as_mut()
            .expect("the Skia context lives as long as the renderer")
    }

    /// Formats a dmabuf can have to be bound as a render target.
    pub fn dmabuf_render_formats(&self) -> FormatSet {
        self.render_formats.clone()
    }

    /// The surface of the target last bound or rendered to.
    pub fn current_skia_renderer(&mut self) -> Option<&SkiaSurface> {
        self.current_target
            .as_ref()
            .map(|target| &target.skia_surface)
    }

    /// Waits for `sync` on the GPU if it can be imported, on the CPU otherwise.
    pub(crate) fn wait_sync(&mut self, sync: &SyncPoint) -> Result<(), SkiaVkError> {
        if sync.is_reached() {
            return Ok(());
        }
        if self.sync_fd.import {
            if let Some(fd) = sync.export() {
                match self.sync_pool.wait(&self.device, fd) {
                    Ok(()) => return Ok(()),
                    Err(err) => tracing::debug!(?err, "GPU wait failed, waiting on the CPU"),
                }
            }
        }
        sync.wait().map_err(|_| SkiaVkError::SyncInterrupted)
    }

    /// Submits everything drawn into `target` and returns when it will be done.
    ///
    /// A dmabuf target is released to the foreign queue family in `GENERAL`
    /// layout, so KMS or another device can read it, and so Skia acquires it
    /// again on the next frame.
    pub(crate) fn submit_target(
        &mut self,
        target: &SkiaVkTarget,
    ) -> Result<SyncPoint, SkiaVkError> {
        let mut surface = target.skia_surface.surface.clone();
        let ctx = self.ctx();
        if target.is_dmabuf() {
            let state = skvk::mutable_texture_states::new_vulkan(
                skvk::ImageLayout::GENERAL,
                vk::QUEUE_FAMILY_FOREIGN_EXT,
            );
            ctx.flush_surface_with_texture_state(
                &mut surface,
                &gpu::FlushInfo::default(),
                Some(&state),
            );
        } else {
            ctx.flush_surface(&mut surface);
        }
        ctx.flush_and_submit();
        Ok(match self.sync_pool.signal(&self.device, self.sync_fd)? {
            Some(sync) => SyncPoint::from(sync),
            None => SyncPoint::signaled(),
        })
    }

    /// Blocks until every plane surface rendered this frame is written, and
    /// frees the images of plane slots dropped since the last call.
    ///
    /// Call once per frame, after the last plane render and before handing
    /// the buffers to the DRM compositor.
    pub fn flush_planes_for_scanout(&mut self) {
        self.ctx().flush_submit_and_sync_cpu();
        self.release_plane_textures();
        self.sync_pool.reclaim(&self.device);
    }

    /// Destroys the images of plane slots dropped since the last call.
    ///
    /// Only call once the GPU is done with them, as
    /// [`Self::flush_planes_for_scanout`] makes sure.
    fn release_plane_textures(&mut self) {
        let released = std::mem::take(
            &mut *self
                .plane_releases
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        drop(released);
    }

    /// Wraps `dmabuf` as a Skia render target.
    fn wrap_dmabuf_target(&mut self, dmabuf: &Dmabuf) -> Result<SkiaVkTarget, SkiaVkError> {
        let code = dmabuf.format().code;
        let fmt = skia_format(code).ok_or(SkiaVkError::UnsupportedFormat(code))?;
        let image = VulkanImage::new_from_dmabuf(&self.device, dmabuf, TARGET_USAGE)?;
        if image.format() != fmt.vk {
            return Err(SkiaVkError::UnsupportedFormat(code));
        }
        let size = dmabuf.size();
        // GENERAL, not UNDEFINED: a swapchain slot rendered with partial
        // damage keeps the content of its last frame.
        let info = image_info(&image, fmt, skvk::ImageLayout::GENERAL);
        let render_target = backend_render_targets::make_vk((size.w, size.h), &info);
        let ctx = self.ctx();
        let surface = surfaces::wrap_backend_render_target(
            ctx,
            &render_target,
            gpu::SurfaceOrigin::TopLeft,
            fmt.color_type,
            None,
            Some(&surface_props()),
        )
        .ok_or(SkiaVkError::Wrap)?;
        Ok(SkiaVkTarget {
            skia_surface: SkiaSurface {
                gr_context: ctx.clone(),
                surface,
            },
            format: code,
            kind: TargetKind::Dmabuf(image),
        })
    }

    /// Returns the cached render target of `dmabuf`, wrapping it on first use.
    fn dmabuf_target(&mut self, dmabuf: &Dmabuf) -> Result<SkiaVkTarget, SkiaVkError> {
        // Evict first: a new buffer can reuse a dead buffer's address, which
        // would otherwise alias its stale entry.
        self.target_cache.retain(|weak, _| !weak.is_gone());
        self.refused_targets.retain(|weak| !weak.is_gone());

        let key = dmabuf.weak();
        if let Some(target) = self.target_cache.get(&key) {
            return Ok(target.clone());
        }
        if self.refused_targets.contains(&key) {
            return Err(SkiaVkError::RefusedTarget);
        }
        match self.wrap_dmabuf_target(dmabuf) {
            Ok(target) => {
                self.target_cache.insert(key, target.clone());
                Ok(target)
            }
            Err(err) => {
                let format = dmabuf.format();
                tracing::warn!(
                    "cannot render into dmabuf {:?} ({:x?}): {err}",
                    format.code,
                    format.modifier
                );
                self.refused_targets.insert(key);
                Err(err)
            }
        }
    }

    /// Imports `dmabuf` for sampling.
    ///
    /// The `VkImage` is cached per buffer, but the Skia image is wrapped anew
    /// on every import: a re-import follows a client commit, after which the
    /// buffer is back in the foreign queue family and Skia must not trust the
    /// layout it last saw.
    fn import_dmabuf_texture(&mut self, dmabuf: &Dmabuf) -> Result<SkiaVkTexture, SkiaVkError> {
        self.dmabuf_cache.retain(|weak, _| !weak.is_gone());
        let code = dmabuf.format().code;
        let fmt = skia_format(code).ok_or(SkiaVkError::UnsupportedFormat(code))?;
        let image = match self.dmabuf_cache.get(&dmabuf.weak()) {
            Some(image) => image.clone(),
            None => {
                let image = VulkanImage::new_from_dmabuf(&self.device, dmabuf, TEXTURE_USAGE)?;
                self.dmabuf_cache.insert(dmabuf.weak(), image.clone());
                image
            }
        };
        if image.format() != fmt.vk {
            return Err(SkiaVkError::UnsupportedFormat(code));
        }
        let size = dmabuf.size();
        let info = image_info(&image, fmt, skvk::ImageLayout::GENERAL);
        // SAFETY: the image outlives the Skia image, which the texture's
        // backing keeps alongside it.
        let backend =
            unsafe { backend_textures::make_vk((size.w, size.h), &info, "client dmabuf") };
        let sk_image = skia::Image::from_texture(
            self.ctx(),
            &backend,
            gpu::SurfaceOrigin::TopLeft,
            fmt.color_type,
            fmt.alpha_type(),
            None,
        )
        .ok_or(SkiaVkError::Wrap)?;
        Ok(SkiaVkTexture {
            image: sk_image,
            has_alpha: !fmt.opaque,
            format: Some(code),
            backing: Arc::new(TextureBacking::Dmabuf { _image: image }),
        })
    }

    /// Uploads `data` (rows of `stride` bytes) into a new Skia texture.
    fn upload(
        &mut self,
        data: &[u8],
        stride: usize,
        format: Fourcc,
        size: Size<i32, Buffer>,
        flipped: bool,
    ) -> Result<SkiaVkTexture, SkiaVkError> {
        let fmt = skia_format(format).ok_or(SkiaVkError::UnsupportedFormat(format))?;
        let info = fmt.image_info(size.w, size.h);
        let ctx = self.ctx();
        let mut surface = surfaces::render_target(
            ctx,
            gpu::Budgeted::Yes,
            &info,
            None,
            gpu::SurfaceOrigin::TopLeft,
            None,
            false,
            None,
        )
        .ok_or(SkiaVkError::SurfaceCreation)?;
        write_rows(
            &mut surface,
            &info,
            data,
            stride,
            Rectangle::from_size(size),
            flipped,
        )?;
        let backend = surfaces::get_backend_texture(
            &mut surface,
            skia::surface::BackendHandleAccess::FlushRead,
        )
        .ok_or(SkiaVkError::Wrap)?;
        let image = skia::Image::from_texture(
            ctx,
            &backend,
            gpu::SurfaceOrigin::TopLeft,
            fmt.color_type,
            fmt.alpha_type(),
            None,
        )
        .ok_or(SkiaVkError::Wrap)?;
        ctx.flush_and_submit_surface(&mut surface, None);
        Ok(SkiaVkTexture {
            image,
            has_alpha: !fmt.opaque,
            format: Some(format),
            backing: Arc::new(TextureBacking::Memory {
                surface: Mutex::new(surface),
                flipped,
            }),
        })
    }

    /// Writes `region` of `data` (rows of `stride` bytes) into a memory texture.
    fn write_memory(
        &mut self,
        texture: &SkiaVkTexture,
        data: &[u8],
        stride: usize,
        region: Rectangle<i32, Buffer>,
    ) -> Result<(), SkiaVkError> {
        let TextureBacking::Memory { surface, flipped } = &*texture.backing else {
            return Err(SkiaVkError::Unsupported);
        };
        let mut surface = surface.lock().unwrap_or_else(|e| e.into_inner());
        let info = surface.image_info();
        write_rows(&mut surface, &info, data, stride, region, *flipped)?;
        self.ctx().flush_and_submit_surface(&mut surface, None);
        Ok(())
    }

    /// Draws `src` of `from` into `dst` of `to`, replacing what is there.
    fn copy_between(
        &mut self,
        from: &SkiaVkTarget,
        to: &SkiaVkTarget,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
        filter: TextureFilter,
    ) -> Result<(), SkiaVkError> {
        let mut from_surface = from.skia_surface.surface.clone();
        let bounds = skia::IRect::from_xywh(src.loc.x, src.loc.y, src.size.w, src.size.h);
        let full = skia::IRect::from_wh(from_surface.width(), from_surface.height());
        let bounds = skia::IRect::intersect(&bounds, &full).ok_or(SkiaVkError::Readback)?;
        let image = from_surface
            .image_snapshot_with_bounds(bounds)
            .ok_or(SkiaVkError::Readback)?;
        // The snapshot starts at the clipped origin; shift the destination
        // by as much as the source was clipped.
        let scale_x = dst.size.w as f32 / src.size.w.max(1) as f32;
        let scale_y = dst.size.h as f32 / src.size.h.max(1) as f32;
        let dst_rect = skia::Rect::from_xywh(
            dst.loc.x as f32 + (bounds.left - src.loc.x) as f32 * scale_x,
            dst.loc.y as f32 + (bounds.top - src.loc.y) as f32 * scale_y,
            bounds.width() as f32 * scale_x,
            bounds.height() as f32 * scale_y,
        );
        let sampling = match filter {
            TextureFilter::Nearest => {
                skia::SamplingOptions::new(skia::FilterMode::Nearest, skia::MipmapMode::None)
            }
            TextureFilter::Linear => {
                skia::SamplingOptions::new(skia::FilterMode::Linear, skia::MipmapMode::None)
            }
        };
        let mut paint = skia::Paint::default();
        paint.set_blend_mode(skia::BlendMode::Src);
        let mut to_surface = to.skia_surface.surface.clone();
        to_surface
            .canvas()
            .draw_image_rect_with_sampling_options(&image, None, dst_rect, sampling, &paint);
        Ok(())
    }

    /// Copies part of the current frame into `dst_dmabuf`.
    ///
    /// The copy is fenced into the dmabuf itself (an implicit write fence),
    /// so a consumer that reads it with implicit sync waits for the GPU; if
    /// the kernel cannot take the fence, this blocks until the copy is done.
    /// The current target stays what it was.
    ///
    /// # Errors
    ///
    /// Fails when nothing has been rendered yet or the dmabuf cannot be
    /// rendered into.
    pub fn blit_current_frame(
        &mut self,
        dst_dmabuf: &Dmabuf,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), SkiaVkError> {
        let source = self.current_target.clone().ok_or(SkiaVkError::NoTarget)?;
        let target = self.dmabuf_target(dst_dmabuf)?;
        self.copy_between(&source, &target, src, dst, TextureFilter::Linear)?;
        let sync = self.submit_target(&target)?;
        let attached = sync
            .get::<SkiaVkSync>()
            .map(|fence| sync::attach_write_fence(dst_dmabuf, fence.fd()));
        if let Some(Err(err)) = attached {
            tracing::debug!(?err, "cannot attach the blit fence to the dmabuf, waiting");
            sync.wait().map_err(|_| SkiaVkError::SyncInterrupted)?;
        }
        Ok(())
    }

    /// Wraps `dmabuf` as a render target for a plane slot.
    ///
    /// Returns the surface and a token that keeps the `VkImage` alive; drop
    /// the token after the surface.
    ///
    /// # Errors
    ///
    /// Fails when the format cannot be rendered into.
    pub fn create_surface_from_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
    ) -> Result<(SkiaSurface, PlaneTextureRelease), SkiaVkError> {
        let target = self.wrap_dmabuf_target(dmabuf)?;
        let TargetKind::Dmabuf(image) = target.kind else {
            return Err(SkiaVkError::Wrap);
        };
        Ok((
            target.skia_surface,
            PlaneTextureRelease {
                image: Some(image),
                queue: self.plane_releases.clone(),
            },
        ))
    }

    /// Imports a shared-memory buffer, reusing the surface's texture when
    /// its size and format still match.
    fn import_shm(
        &mut self,
        buffer: &WlBuffer,
        surface: Option<&SurfaceData>,
        damage: &[Rectangle<i32, Buffer>],
    ) -> Result<SkiaVkTexture, SkiaVkError> {
        let cache = surface.map(|surface| {
            surface
                .data_map
                .insert_if_missing_threadsafe(ShmCache::default);
            surface.data_map.get::<ShmCache>().expect("inserted above")
        });
        let context_id = self.context_id.clone();

        shm::with_buffer_contents(buffer, |ptr, len, data| {
            let format = shm::shm_format_to_fourcc(data.format)
                .ok_or(SkiaVkError::UnsupportedFormat(Fourcc::Argb8888))?;
            let stride = data.stride as usize;
            let size = Size::<i32, Buffer>::from((data.width, data.height));
            let offset = data.offset as usize;
            let expected = stride * data.height as usize;
            if len < offset + expected {
                return Err(SkiaVkError::BufferSize {
                    expected: offset + expected,
                    got: len,
                });
            }
            // SAFETY: the pool mapping holds `len` bytes at `ptr` for the
            // duration of this closure, and `offset + expected <= len`.
            let bytes = unsafe { std::slice::from_raw_parts(ptr.add(offset), expected) };

            let mut cached = cache.map(|cache| cache.0.lock().unwrap_or_else(|e| e.into_inner()));
            let reusable = cached.as_ref().and_then(|cached| {
                cached.as_ref().filter(|(id, texture)| {
                    *id == context_id
                        && texture.format == Some(format)
                        && texture.image.width() == size.w
                        && texture.image.height() == size.h
                })
            });
            if let Some((_, texture)) = reusable {
                let texture = texture.clone();
                for rect in damage {
                    let region = rect.intersection(Rectangle::from_size(size));
                    if let Some(region) = region {
                        self.write_memory(&texture, bytes, stride, region)?;
                    }
                }
                return Ok(texture);
            }

            let texture = self.upload(bytes, stride, format, size, false)?;
            if let Some(cached) = cached.as_mut() {
                **cached = Some((context_id.clone(), texture.clone()));
            }
            Ok(texture)
        })?
    }
}

impl Drop for SkiaVkRenderer {
    fn drop(&mut self) {
        self.current_target = None;
        self.target_cache.clear();
        self.dmabuf_cache.clear();
        if let Some(mut context) = self.context.take() {
            context.flush_submit_and_sync_cpu();
            // Skia objects that outlive the renderer (textures kept in
            // surface state) must not touch the device once it is gone.
            context.release_resources_and_abandon();
        }
        self.release_plane_textures();
        // SAFETY: the device is still alive; nothing else submits to it.
        if let Err(err) = unsafe { self.device.vk().device_wait_idle() } {
            tracing::warn!(?err, "waiting for the Vulkan device to go idle failed");
        }
        self.sync_pool.destroy(&self.device);
    }
}

/// Describes `image` to Skia as a foreign-owned image in `layout`.
fn image_info(image: &VulkanImage, fmt: SkiaFormat, layout: skvk::ImageLayout) -> skvk::ImageInfo {
    // SAFETY: the handle is a live image; Skia neither owns nor frees its
    // memory (`Alloc::default()`).
    unsafe {
        skvk::ImageInfo::new(
            image.vk().as_raw() as _,
            skvk::Alloc::default(),
            skvk::ImageTiling::DRM_FORMAT_MODIFIER_EXT,
            layout,
            fmt.skia_vk,
            1,
            vk::QUEUE_FAMILY_FOREIGN_EXT,
            None,
            None,
            None,
        )
    }
}

/// Surface properties matching the GL renderer's.
fn surface_props() -> skia::SurfaceProps {
    skia::SurfaceProps::new(Default::default(), skia::PixelGeometry::BGRH)
}

/// Writes `region` of `data` (rows of `stride` bytes) into `surface`.
///
/// With `flipped`, the rows of `data` run bottom to top and land mirrored.
fn write_rows(
    surface: &mut skia::Surface,
    info: &skia::ImageInfo,
    data: &[u8],
    stride: usize,
    region: Rectangle<i32, Buffer>,
    flipped: bool,
) -> Result<(), SkiaVkError> {
    let bpp = info.bytes_per_pixel();
    let (x, y) = (region.loc.x as usize, region.loc.y as usize);
    let (w, h) = (region.size.w as usize, region.size.h as usize);
    let row_len = w * bpp;
    let mut rows = Vec::with_capacity(row_len * h);
    for row in 0..h {
        let start = (y + row) * stride + x * bpp;
        let src = data
            .get(start..start + row_len)
            .ok_or(SkiaVkError::BufferSize {
                expected: start + row_len,
                got: data.len(),
            })?;
        rows.extend_from_slice(src);
    }
    let dst_y = if flipped {
        rows = rows
            .chunks_exact(row_len.max(1))
            .rev()
            .flatten()
            .copied()
            .collect();
        info.height() - region.loc.y - region.size.h
    } else {
        region.loc.y
    };
    let region_info = info.with_dimensions((region.size.w, region.size.h));
    let pixmap =
        skia::Pixmap::new(&region_info, &mut rows, row_len).ok_or(SkiaVkError::BufferSize {
            expected: row_len * h,
            got: 0,
        })?;
    surface.write_pixels_from_pixmap(&pixmap, (region.loc.x, dst_y));
    Ok(())
}

impl RendererSuper for SkiaVkRenderer {
    type Error = SkiaVkError;
    type TextureId = SkiaVkTexture;
    type Framebuffer<'buffer> = SkiaVkTarget;
    type Frame<'frame, 'buffer>
        = SkiaVkFrame<'frame>
    where
        'buffer: 'frame;
}

impl Renderer for SkiaVkRenderer {
    fn context_id(&self) -> ContextId<Self::TextureId> {
        self.context_id.clone()
    }

    fn downscale_filter(&mut self, filter: TextureFilter) -> Result<(), Self::Error> {
        self.downscale_filter = filter;
        Ok(())
    }

    fn upscale_filter(&mut self, filter: TextureFilter) -> Result<(), Self::Error> {
        self.upscale_filter = filter;
        Ok(())
    }

    fn set_debug_flags(&mut self, flags: DebugFlags) {
        self.debug_flags = flags;
    }

    fn debug_flags(&self) -> DebugFlags {
        self.debug_flags
    }

    fn render<'frame, 'buffer>(
        &'frame mut self,
        framebuffer: &'frame mut Self::Framebuffer<'buffer>,
        output_size: Size<i32, Physical>,
        _dst_transform: Transform,
    ) -> Result<Self::Frame<'frame, 'buffer>, Self::Error>
    where
        'buffer: 'frame,
    {
        self.current_target = Some(framebuffer.clone());
        Ok(SkiaVkFrame {
            size: output_size,
            skia_surface: framebuffer.skia_surface.clone(),
            target: framebuffer.clone(),
            renderer: self,
        })
    }

    fn wait(&mut self, sync: &SyncPoint) -> Result<(), Self::Error> {
        self.wait_sync(sync)
    }

    fn cleanup_texture_cache(&mut self) -> Result<(), Self::Error> {
        self.dmabuf_cache.retain(|weak, _| !weak.is_gone());
        self.target_cache.retain(|weak, _| !weak.is_gone());
        self.sync_pool.reclaim(&self.device);
        Ok(())
    }
}

impl ImportMem for SkiaVkRenderer {
    fn import_memory(
        &mut self,
        data: &[u8],
        format: Fourcc,
        size: Size<i32, Buffer>,
        flipped: bool,
    ) -> Result<Self::TextureId, Self::Error> {
        let fmt = skia_format(format).ok_or(SkiaVkError::UnsupportedFormat(format))?;
        let stride = fmt.image_info(size.w, size.h).min_row_bytes();
        self.upload(data, stride, format, size, flipped)
    }

    fn update_memory(
        &mut self,
        texture: &Self::TextureId,
        data: &[u8],
        region: Rectangle<i32, Buffer>,
    ) -> Result<(), Self::Error> {
        let stride = texture.image.image_info().min_row_bytes();
        self.write_memory(texture, data, stride, region)
    }

    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        Box::new(FOURCCS.iter().copied())
    }
}

impl ImportMemWl for SkiaVkRenderer {
    fn import_shm_buffer(
        &mut self,
        buffer: &WlBuffer,
        surface: Option<&SurfaceData>,
        damage: &[Rectangle<i32, Buffer>],
    ) -> Result<Self::TextureId, Self::Error> {
        self.import_shm(buffer, surface, damage)
    }
}

impl ImportDma for SkiaVkRenderer {
    fn dmabuf_formats(&self) -> FormatSet {
        self.texture_formats.clone()
    }

    fn import_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        _damage: Option<&[Rectangle<i32, Buffer>]>,
    ) -> Result<Self::TextureId, Self::Error> {
        self.import_dmabuf_texture(dmabuf)
    }
}

impl ImportDmaWl for SkiaVkRenderer {}

impl ExportMem for SkiaVkRenderer {
    type TextureMapping = SkiaVkMapping;

    fn copy_framebuffer(
        &mut self,
        target: &Self::Framebuffer<'_>,
        region: Rectangle<i32, Buffer>,
        format: Fourcc,
    ) -> Result<Self::TextureMapping, Self::Error> {
        let fmt = skia_format(format).ok_or(SkiaVkError::UnsupportedFormat(format))?;
        let info = skia::ImageInfo::new(
            (region.size.w, region.size.h),
            fmt.color_type,
            skia::AlphaType::Premul,
            None,
        );
        let row_bytes = info.min_row_bytes();
        let mut data = vec![0u8; row_bytes * region.size.h.max(0) as usize];
        let mut surface = target.skia_surface.surface.clone();
        if !surface.read_pixels(&info, &mut data, row_bytes, (region.loc.x, region.loc.y)) {
            return Err(SkiaVkError::Readback);
        }
        Ok(SkiaVkMapping {
            format,
            region,
            data,
        })
    }

    fn copy_texture(
        &mut self,
        texture: &Self::TextureId,
        region: Rectangle<i32, Buffer>,
        format: Fourcc,
    ) -> Result<Self::TextureMapping, Self::Error> {
        let fmt = skia_format(format).ok_or(SkiaVkError::UnsupportedFormat(format))?;
        let info = skia::ImageInfo::new(
            (region.size.w, region.size.h),
            fmt.color_type,
            skia::AlphaType::Premul,
            None,
        );
        let row_bytes = info.min_row_bytes();
        let mut data = vec![0u8; row_bytes * region.size.h.max(0) as usize];
        let read = texture.image.read_pixels_with_context(
            self.ctx(),
            &info,
            &mut data,
            row_bytes,
            (region.loc.x, region.loc.y),
            skia::image::CachingHint::Disallow,
        );
        if !read {
            return Err(SkiaVkError::Readback);
        }
        Ok(SkiaVkMapping {
            format,
            region,
            data,
        })
    }

    fn can_read_texture(&mut self, _texture: &Self::TextureId) -> Result<bool, Self::Error> {
        Ok(true)
    }

    fn map_texture<'a>(
        &mut self,
        texture_mapping: &'a Self::TextureMapping,
    ) -> Result<&'a [u8], Self::Error> {
        Ok(&texture_mapping.data)
    }
}

impl Bind<Dmabuf> for SkiaVkRenderer {
    fn bind(&mut self, dmabuf: &mut Dmabuf) -> Result<SkiaVkTarget, Self::Error> {
        let target = self.dmabuf_target(dmabuf)?;
        self.current_target = Some(target.clone());
        Ok(target)
    }
}

impl Bind<SkiaVkTarget> for SkiaVkRenderer {
    fn bind(&mut self, target: &mut SkiaVkTarget) -> Result<SkiaVkTarget, Self::Error> {
        self.current_target = Some(target.clone());
        Ok(target.clone())
    }
}

impl Offscreen<SkiaVkTarget> for SkiaVkRenderer {
    fn create_buffer(
        &mut self,
        format: Fourcc,
        size: Size<i32, Buffer>,
    ) -> Result<SkiaVkTarget, Self::Error> {
        let fmt = skia_format(format).ok_or(SkiaVkError::UnsupportedFormat(format))?;
        let ctx = self.ctx();
        let surface = surfaces::render_target(
            ctx,
            gpu::Budgeted::Yes,
            &fmt.image_info(size.w, size.h),
            None,
            gpu::SurfaceOrigin::TopLeft,
            Some(&surface_props()),
            false,
            None,
        )
        .ok_or(SkiaVkError::SurfaceCreation)?;
        Ok(SkiaVkTarget {
            skia_surface: SkiaSurface {
                gr_context: ctx.clone(),
                surface,
            },
            format,
            kind: TargetKind::Offscreen,
        })
    }
}

impl Blit for SkiaVkRenderer {
    fn blit(
        &mut self,
        from: &SkiaVkTarget,
        to: &mut SkiaVkTarget,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
        filter: TextureFilter,
    ) -> Result<SyncPoint, Self::Error> {
        self.copy_between(from, to, src, dst, filter)?;
        self.submit_target(to)
    }
}
