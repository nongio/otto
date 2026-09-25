# Skia on Vulkan: plan

Status: phases 1 and 2 done on 2026-09-25, on the `feat/vulkan-device` branch of the smithay fork and `try/smithay-latest` in Otto. Phase 3 onwards is not started.

Otto draws with Skia's Ganesh backend on OpenGL ES through EGL. This plan
moves the compositor's rendering to Ganesh on Vulkan while keeping Smithay's
DRM compositor, the plane scanout machinery and the lay-rs scene graph as they
are. Client-side rendering in otto-kit is out of scope.

## Why

- Explicit sync end to end. Vulkan timeline semaphores and sync files replace
  the EGL fence plus `glFlush` dance in `renderer/frame.rs`, and the CPU wait
  in `flush_planes_for_scanout` that exists because Mesa attaches no implicit
  fence to EGLImage render targets.
- One import path for client buffers. A dmabuf becomes a `VkImage` with an
  explicit DRM format modifier; no EGLImage, no external-texture blit
  fallback, no `wl_drm`.
- Formats. fp16 and 10-bit render targets, needed by the HDR branch, are
  ordinary Vulkan formats; the GL path depends on EGL config luck.
- Smithay's own renderer work is heading the same way (PR #2165), so a Vulkan
  device layer will exist upstream to share.

## What exists today

| Piece | State |
|---|---|
| skia-safe 0.93.1 | Vulkan backend compiled in already: lay-rs enables `all-linux`, which includes `vulkan`, and skia-binaries ships that combination prebuilt. No `ash` dependency; handles are raw pointers plus a `get_proc` closure. |
| Smithay master | `backend/vulkan` (instance, physical device, driver info) and `allocator/vulkan` (`VulkanAllocator`, dmabuf export with modifiers). No renderer. |
| Smithay PR #2165 | Draft Vulkan renderer, compute-shader based, `todo!()` in `wait`, filters, `update_memory`. Not usable as a renderer. Its `backend/vulkan/{device,image,format}.rs` layer (device + queue, dmabuf import with explicit modifier layouts, exportable images, timeline semaphore as drm syncobj) is the reusable part. |
| lay-rs | Backend agnostic. The engine takes a `DirectContext` from the host or from the target canvas. GL-specific code is ~80 lines: `renderer/skia_fbo.rs`, the C API's FBO entry point, one dead import in `drawing/scene.rs`, and the GL setup in 7 examples/tests. |
| Otto | ~2.6k lines fully GL-bound (`skia_renderer.rs`, `renderer/*`), plus typing coupling to `GlesRenderer` through Smithay's `GbmGlesBackend` / `MultiRenderer` in `udev/`. |

Gaps that shape the design:

- skia-safe exposes no semaphore API. `FlushInfo`'s signal semaphores are
  private and there is no `DirectContext::wait`. Sync has to happen around
  Skia's submits, not inside them.
- Skia treats a `VkImage` created with `DRM_FORMAT_MODIFIER_EXT` tiling as an
  external texture: sample only, no mipmaps, no render-to. Wrapped render
  targets are not tiling-checked. So client buffers import as textures and
  plane slots wrap as render targets, which matches how Otto uses them.
- Smithay's multi-GPU layer is GLES only. A Vulkan renderer needs its own
  `GraphicsApi` or a single-GPU first phase.

## Design

### Device layer

Take Smithay master's `backend/vulkan` plus the device/image/format layer
from PR #2165 into the fork branch (`nongio/smithay`), independent of that
PR's renderer. That gives Otto:

- `Instance` with `VK_KHR_external_memory_capabilities`,
  `VK_KHR_external_semaphore_capabilities`.
- `Device` on the graphics queue of the primary render node, with
  `VK_KHR_external_memory_fd`, `VK_EXT_external_memory_dma_buf`,
  `VK_EXT_image_drm_format_modifier`, `VK_EXT_queue_family_foreign`,
  `VK_KHR_external_semaphore_fd`, `VK_KHR_timeline_semaphore`.
- `VulkanImage::new_from_dmabuf` (explicit plane layouts) and
  `new_exportable` (modifier list, dedicated allocation, fd export).
- Format tables fourcc ↔ `vk::Format`, modifier queries per usage.

PR #2165 requires the compute queue and `STORAGE` usage; the port changes
that to the graphics queue and `SAMPLED` / `COLOR_ATTACHMENT`. This is the one
place we diverge from upstream, and it should go back as review feedback on
the PR rather than live in the fork for long.

### `SkiaVkRenderer` in Otto

A new renderer next to the existing one, implementing the same Smithay traits
`SkiaRenderer` implements today, without wrapping a `GlesRenderer`:

- `DirectContext` from `gpu::vk::BackendContext::new_with_extensions` on the
  shared device, passing the device extension list so Skia enables DRM
  modifier support.
- `RendererSuper`: `TextureId = SkiaVkTexture` (a `VulkanImage` plus the Skia
  `Image` wrapping it), `Framebuffer = SkiaVkTarget` (a `VulkanImage` plus the
  Skia `Surface`), `Frame = SkiaFrame` reused as is, since it only draws
  through Skia.
- `ImportDma`: dmabuf → `VulkanImage::new_from_dmabuf` → `ImageInfo` with
  `Alloc::from_device_memory`, `QUEUE_FAMILY_FOREIGN_EXT` →
  `backend_textures::make_vk` → `Image::from_texture`. Cached per
  `WeakDmabuf` like today; Skia handles the foreign-queue acquire barrier.
- `ImportMem` / SHM: raster `Image` → `new_texture_image(ctx)`. No
  `VK_EXT_host_image_copy` requirement.
- `Bind<Dmabuf>`: slot dmabuf → `VulkanImage::new_from_dmabuf` with
  `COLOR_ATTACHMENT | TRANSFER_SRC | TRANSFER_DST` →
  `backend_render_targets::make_vk` → `wrap_backend_render_target`,
  `SurfaceOrigin::TopLeft`. Replaces `create_surface_from_dmabuf` and
  `PlaneTextureRelease`.
- `Offscreen`: `gpu::surfaces::render_target`, already what backdrops use.
- `ExportMem`: `Surface::read_pixels`, unchanged.
- `Blit`: `Surface::draw` of a snapshot, or `vkCmdBlitImage` in an empty
  submit; the former is simpler and Skia-ordered.
- `BlitCurrentFrame` for screenshare: snapshot + draw.

### Sync

Vulkan orders semaphore operations by submission order on one queue, so
fences can wrap Skia's submits without touching its internals:

- Acquire: for each client buffer with a drm syncobj point, export a sync
  file (`DrmSyncPoint::export_sync_file`), import it as a binary semaphore
  with `SYNC_FD` (temporary import), and wait on it in an empty
  `vkQueueSubmit` before Skia's `flush_and_submit`. Buffers without explicit
  sync use `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` the same way.
- Release: after `flush_and_submit`, an empty submit signals a `SYNC_FD`
  exportable semaphore. The fd becomes the Smithay `SyncPoint` for the frame
  (KMS `IN_FENCE_FD`, syncobj release points, screencast fences). This
  replaces `SkiaSync(EGLFence)`, the trailing `glFlush`, and the CPU sync in
  `flush_planes_for_scanout`.
- Fallback for drivers without `SYNC_FD` semaphores:
  `vkQueueWaitIdle` behind a capability flag, so the path still works on
  odd hardware.

### Allocation for scanout

Keep `GbmAllocator` for the primary swapchain and the plane swapchains in
`scene_dmabuf_element.rs`, but pick modifiers from the intersection of the
plane's `IN_FORMATS` and the modifiers Vulkan reports for
`COLOR_ATTACHMENT`. Everything else in the plane machinery (slot bind,
damage, backdrop composite, `UnderlyingStorage::Dmabuf`) stays. Switching to
`VulkanAllocator` is possible later since `DrmCompositor` is generic over the
allocator, but it is not needed for the first cut.

### lay-rs

- Delete the unused `gl::direct_contexts` import in `drawing/scene.rs`.
- Move `renderer/skia_fbo.rs` and the `make_gl` fallback behind a `gl`
  feature; move `gl-rs` to dev-dependencies.
- Add `renderer/skia_vk.rs`: wrap a host `VkImage` as a render target with
  `SurfaceOrigin::TopLeft`. Used by examples and a headless Vulkan variant of
  `tests/hdr_intermediates.rs` (the only test exercising the GPU image-cache
  path).
- No engine changes. Subtree buffers, backdrop images, picture and image
  caches already go through backend-agnostic `surfaces::render_target` and
  snapshots.

### Backends

- udev: `renderer = "vulkan"` in config selects `SkiaVkRenderer`, single
  GPU, no `MultiRenderer`. `UdevRenderer` becomes generic over the two
  renderers, or the GL path keeps `MultiRenderer` and the Vulkan path uses
  the renderer directly. Multi-GPU on Vulkan is a later phase with its own
  Smithay `GraphicsApi`.
- winit: stays on GL. Vulkan development happens on `--tty-udev` and on the
  headless feature. A Vulkan winit path (Wayland WSI swapchain) is optional
  later work.
- x11: unchanged, low priority.
- headless: no GPU, unchanged.

## Phases

1. **Device layer in the fork.** Rebase Smithay PR #2165's
   `backend/vulkan` commits onto the fork, switch to the graphics queue, add
   the `SYNC_FD` semaphore helpers. Unit test: import a GBM dmabuf, export an
   image, round-trip the modifier.
2. **Standalone probe.** A small binary under `examples/` that creates the
   Skia Vulkan context on the device, wraps a GBM buffer as a render target,
   draws with lay-rs, and reads pixels back. Proves skia-safe's Vulkan
   wrapping and the fence wrapping without touching the compositor.
3. **`SkiaVkRenderer` behind a config flag.** Implement the trait table above;
   run the full udev path with planes disabled first (`planes_enabled` false).
   Verify with `/tmp/otto-dump-planes` and the perf toggles. Screenshare and
   screencopy through `BlitCurrentFrame`.
4. **Planes and explicit sync.** Slot binding as render targets, modifier
   intersection for plane swapchains, sync points from semaphores, syncobj
   release points. Verify zero-copy counts match the GL path on the
   Framework laptop.
5. **lay-rs cleanup and Vulkan test path.**
6. **Decide on default.** A/B GPU residency (the `gpu_bench.py` RC6 method),
   stability over a multi-day session, then flip the default or keep GL.

Phases 1 and 2 are independent of the Smithay master port that is in
progress in `otto-smithay-latest`; phase 3 onwards should land on top of it
so the renderer is written once against the current traits (`Frame::
output_size`, `FrameContext`, `draw` with `cache`).

## What the probe established

`examples/vulkan_probe.rs` (Otto, `try/smithay-latest`) runs the whole
chain on the Framework laptop's Iris Xe with ANV:

- Smithay's `Device` on the graphics queue, Skia `DirectContext` on it, a
  lay-rs scene rendered into an exportable `VkImage`, exported as a linear
  dmabuf, re-imported as a sampled texture and drawn again: pixel-exact,
  under 100 ms end to end in a debug build including PNG encoding.
- `Alloc::default()` is fine for wrapped images; Skia never touches the
  memory of an image it did not allocate.
- The empty-submit fence works: a `SYNC_FD` semaphore signalled after
  `flush_and_submit` becomes readable once the scene has rendered.
- Skia must be told `set_max_api_version(1.3)`. The device reports 1.4,
  the instance is capped at 1.3, and without the pin Skia asks for 1.4 core
  entry points that the loader does not hand out and context creation
  fails silently.
- Three fixes to PR #2165's device layer, now on the fork branch: dmabuf
  imports use `DRM_FORMAT_MODIFIER_EXT` tiling (the draft left them
  `OPTIMAL`, which produced Y-tiled reads of linear data and a memory
  requirement larger than the buffer), the memory type test used
  `bits & i` instead of `bits & (1 << i)`, and imports now intersect with
  `vkGetMemoryFdPropertiesKHR`. The `Texture` impl for `VulkanImage` moved
  into the device layer so `backend_vulkan` builds without
  `renderer_vulkan`, and the syncobj timeline types moved to
  `backend::drm::sync` with re-exports at the old path.

## Risks

- **Driver coverage.** ANV and RADV have the full extension set. NVK and
  the proprietary NVIDIA driver need checking for `SYNC_FD` semaphore
  export and `DRM_FORMAT_MODIFIER_EXT` on imported images. Keep GL as the
  fallback until both are verified.
- **Compressed modifiers.** Intel CCS and AMD DCC modifiers may be
  importable as textures but not usable as colour attachments. The modifier
  intersection handles plane slots; client buffers are only sampled, so this
  affects nothing else.
- **Disjoint multi-plane dmabufs.** PR #2165 imports single-memory-object
  buffers only. Video clients (NV12 from VAAPI) commonly produce disjoint
  planes; support needs `BindImagePlaneMemoryInfo`. Track as a follow-up,
  keep those clients composited until then.
- **Skia layout tracking.** Skia tracks `VkImageLayout` per wrapped image;
  after every external use (scanout, screencopy) the layout must be
  reported back with `set_vk_image_layout`, or Skia issues wrong barriers.
- **Two contexts in one process.** GL and Vulkan Skia contexts can coexist
  (the binary already links both), but images do not cross between them.
  No mixed mode; the config flag selects one renderer per session.
