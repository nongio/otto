use skia_safe::Canvas;
use std::cell::RefCell;
use std::rc::Rc;
use wayland_client::{backend::ObjectId, protocol::wl_surface, Proxy};

/// EGL resources stored separately in global HashMap
/// Only accessed during commit, resize, and cleanup
pub struct EglSurfaceResources {
    pub wl_surface: wl_surface::WlSurface,
    pub egl_surface: khronos_egl::Surface,
    pub wl_egl_surface: wayland_egl::WlEglSurface,
    pub width: i32,
    pub height: i32,
}

/// Individual renderable surface with its own EGL surface
/// Can be used for main windows, subsurfaces, or any other surface type

#[derive(Clone)]
pub struct SkiaSurface {
    surface_id: ObjectId,
    // Cached Skia surface for zero-overhead drawing (hot path)
    cached_surface: Rc<RefCell<Option<skia_safe::Surface>>>,
}

impl SkiaSurface {
    /// Create a SkiaSurface from already-created parts
    /// This is called internally by SkiaContext
    pub(crate) fn new_from_parts(
        wl_surface: wl_surface::WlSurface,
        egl_surface: khronos_egl::Surface,
        wl_egl_surface: wayland_egl::WlEglSurface,
        width: i32,
        height: i32,
    ) -> Self {
        let surface_id = wl_surface.id();

        // Store EGL resources in global HashMap
        let resources = EglSurfaceResources {
            wl_surface,
            egl_surface,
            wl_egl_surface,
            width,
            height,
        };
        crate::app_runner::AppContext::insert_egl_resources(surface_id.clone(), resources);

        Self {
            surface_id,
            cached_surface: Rc::new(RefCell::new(None)),
        }
    }

    /// Resize the surface
    pub fn resize(&mut self, width: i32, height: i32) {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| {
            res.width = width;
            res.height = height;
            res.wl_egl_surface.resize(width, height, 0, 0);
        });
        // Invalidate cached surface so it gets recreated with new size
        *self.cached_surface.borrow_mut() = None;
    }

    /// Draw on this surface using the provided context and drawing function
    /// Zero-overhead hot path - only accesses the cached Skia surface
    pub fn draw<F>(&self, ctx: &mut super::SkiaContext, draw_fn: F)
    where
        F: FnOnce(&Canvas),
    {
        // Initialize cached surface if needed (cold path - only on first draw)
        if self.cached_surface.borrow().is_none() {
            self.initialize_skia_surface(ctx);
        }

        // Hot path: direct access to cached surface, zero lookup overhead
        if let Some(ref mut skia_surface) = *self.cached_surface.borrow_mut() {
            let canvas = skia_surface.canvas();

            // Scale canvas by 2x for HiDPI rendering (buffers are 2x size)
            canvas.save();
            canvas.scale((2.0, 2.0));

            // Call user's drawing function
            draw_fn(canvas);

            // Restore canvas state to prevent scale accumulation
            canvas.restore();
        }
    }

    /// Initialize the Skia surface (cold path - called once)
    fn initialize_skia_surface(&self, ctx: &mut super::SkiaContext) {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| {
            unsafe {
                // Load EGL - this is cached internally after first load, so cheap
                let egl =
                    khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required().unwrap();

                // Make this surface's EGL surface current
                egl.make_current(
                    ctx.egl_display(),
                    Some(res.egl_surface),
                    Some(res.egl_surface),
                    Some(ctx.egl_context()),
                )
                .ok();

                // Query framebuffer info
                let mut fboid: gl::types::GLint = 0;
                gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid);

                let stencil = 8;

                // Create Skia backend render target
                let fb_info = skia_safe::gpu::gl::FramebufferInfo {
                    fboid: fboid as u32,
                    format: skia_safe::gpu::gl::Format::RGBA8.into(),
                    protected: skia_safe::gpu::Protected::No,
                };

                let backend_render_target = skia_safe::gpu::backend_render_targets::make_gl(
                    (res.width, res.height),
                    0,
                    stencil as usize,
                    fb_info,
                );

                *self.cached_surface.borrow_mut() =
                    skia_safe::gpu::surfaces::wrap_backend_render_target(
                        ctx.skia_context(),
                        &backend_render_target,
                        skia_safe::gpu::SurfaceOrigin::BottomLeft,
                        skia_safe::ColorType::RGBA8888,
                        None,
                        None,
                    );
            }
        });
    }

    /// Swap buffers after drawing (cold path)
    pub fn swap_buffers(&self, ctx: &mut super::SkiaContext) {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| {
            unsafe {
                // Flush to GPU
                ctx.skia_context().flush_and_submit();

                let egl =
                    khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required().unwrap();
                egl.swap_buffers(ctx.egl_display(), res.egl_surface).ok();
            }
        });
    }

    /// Commit the surface and mark damage
    pub fn commit(&self) {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| {
            res.wl_surface.damage_buffer(0, 0, res.width, res.height);
            res.wl_surface.commit();
        });
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> wl_surface::WlSurface {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| {
            res.wl_surface.clone()
        })
        .unwrap()
    }

    /// Get current width
    pub fn width(&self) -> i32 {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| res.width)
            .unwrap_or(0)
    }

    /// Get current height
    pub fn height(&self) -> i32 {
        crate::app_runner::AppContext::with_egl_resources(&self.surface_id, |res| res.height)
            .unwrap_or(0)
    }

    /// Get surface ID
    pub fn surface_id(&self) -> &ObjectId {
        &self.surface_id
    }
}

impl Drop for SkiaSurface {
    fn drop(&mut self) {
        // Clean up EGL resources from HashMap
        crate::app_runner::AppContext::remove_egl_resources(&self.surface_id);
    }
}
