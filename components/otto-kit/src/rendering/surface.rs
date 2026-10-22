use skia_safe::Canvas;
use std::cell::RefCell;
use std::rc::Rc;
use wayland_client::{
    backend::ObjectId,
    protocol::{wl_output, wl_surface},
    Proxy,
};

use crate::app_runner::AppContext;

/// EGL resources stored separately in global HashMap
/// Only accessed during commit, resize, and cleanup
pub struct EglSurfaceResources {
    pub wl_surface: wl_surface::WlSurface,
    pub egl_surface: khronos_egl::Surface,
    pub wl_egl_surface: wayland_egl::WlEglSurface,
    pub width: i32,
    pub height: i32,
    /// `width`/`height` have not reached the `wl_egl_window` yet. See
    /// [`SkiaSurface::resize`].
    pub resize_pending: bool,
}

impl Drop for EglSurfaceResources {
    fn drop(&mut self) {
        // Destroy EGL surface before wl_egl_surface
        let _ = crate::app_runner::context::EGL_DISPLAY.try_with(|display_cell| {
            if let Some(display) = *display_cell.borrow() {
                unsafe {
                    let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()
                        .ok()?;
                    let _res = egl.destroy_surface(display, self.egl_surface);
                }
            }
            Some(())
        });
        // wl_egl_surface and wl_surface will be dropped automatically
    }
}

/// Individual renderable surface with its own EGL surface
/// Can be used for main windows, subsurfaces, or any other surface type
pub struct SkiaSurface {
    surface_id: ObjectId,
    // Cached Skia surface for zero-overhead drawing (hot path)
    cached_surface: Rc<RefCell<Option<skia_safe::Surface>>>,
    // The subpixel layout text is rendered for; see `set_pixel_geometry`.
    pixel_geometry: std::cell::Cell<skia_safe::PixelGeometry>,
}

/// The subpixel layout to render text for, in a buffer's own orientation, on
/// an output whose panel reports `subpixel` and which shows its content with
/// `transform`.
///
/// `wl_output.subpixel` describes the panel; a rotated or flipped output turns
/// the buffer relative to it. At 90° the buffer's rows run along the panel's
/// columns, so a horizontal RGB panel is a vertical one to the buffer; at
/// 180° the order reverses; the flipped transforms mirror the buffer's x axis
/// first. A layout nothing can be said about is `Unknown`, which renders
/// greyscale text — never wrong, only less sharp.
pub(crate) fn pixel_geometry(
    subpixel: wl_output::Subpixel,
    transform: wl_output::Transform,
) -> skia_safe::PixelGeometry {
    use skia_safe::PixelGeometry;
    use wl_output::{Subpixel, Transform};

    // The panel's layout: along its x axis or its y axis, and whether red
    // comes first (left, or top).
    let (panel_horizontal, panel_rgb) = match subpixel {
        Subpixel::HorizontalRgb => (true, true),
        Subpixel::HorizontalBgr => (true, false),
        Subpixel::VerticalRgb => (false, true),
        Subpixel::VerticalBgr => (false, false),
        _ => return PixelGeometry::Unknown,
    };
    // Which of the buffer's axes runs along the panel's x axis and along its
    // y axis, and whether each runs backwards.
    let (along_x, along_y) = match transform {
        Transform::Normal => ((true, false), (false, false)),
        Transform::_90 => ((false, false), (true, true)),
        Transform::_180 => ((true, true), (false, true)),
        Transform::_270 => ((false, true), (true, false)),
        Transform::Flipped => ((true, true), (false, false)),
        Transform::Flipped90 => ((false, false), (true, false)),
        Transform::Flipped180 => ((true, false), (false, true)),
        Transform::Flipped270 => ((false, true), (true, true)),
        _ => return PixelGeometry::Unknown,
    };
    let (buffer_horizontal, reversed) = if panel_horizontal { along_x } else { along_y };
    match (buffer_horizontal, panel_rgb != reversed) {
        (true, true) => PixelGeometry::RGBH,
        (true, false) => PixelGeometry::BGRH,
        (false, true) => PixelGeometry::RGBV,
        (false, false) => PixelGeometry::BGRV,
    }
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
            resize_pending: false,
        };
        AppContext::insert_egl_resources(surface_id.clone(), resources);

        Self {
            surface_id,
            cached_surface: Rc::new(RefCell::new(None)),
            pixel_geometry: std::cell::Cell::new(skia_safe::PixelGeometry::Unknown),
        }
    }

    /// Render text for `geometry`, the subpixel layout the buffer's pixels
    /// land on — `Unknown` for greyscale text. Skia fixes it when a surface is
    /// wrapped, so a change wraps the render target again on the next draw.
    pub fn set_pixel_geometry(&self, geometry: skia_safe::PixelGeometry) {
        if self.pixel_geometry.replace(geometry) != geometry {
            *self.cached_surface.borrow_mut() = None;
        }
    }

    /// Resize the surface
    ///
    /// The `wl_egl_window` is resized on the next draw, around making this
    /// surface current — see `initialize_skia_surface`.
    pub fn resize(&self, width: i32, height: i32) {
        AppContext::with_egl_resources(&self.surface_id, |res| {
            res.width = width;
            res.height = height;
            res.resize_pending = true;
        });
        // Invalidate cached surface so it gets recreated with new size
        *self.cached_surface.borrow_mut() = None;
    }

    /// Draw on this surface using the provided context and drawing function.
    ///
    /// `scale` is buffer pixels per logical point: the canvas is scaled by it
    /// before `draw_fn` runs, so `draw_fn` works in points.
    /// Zero-overhead hot path - only accesses the cached Skia surface
    pub fn draw<F>(&self, ctx: &mut super::SkiaContext, scale: f32, draw_fn: F)
    where
        F: FnOnce(&Canvas),
    {
        // Initialize cached surface if needed (cold path - only on first draw)
        if self.cached_surface.borrow().is_none() {
            self.initialize_skia_surface(ctx);
        }

        // Make this surface's EGL context current before each draw
        AppContext::with_egl_resources(&self.surface_id, |res| unsafe {
            let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required().unwrap();
            egl.make_current(
                ctx.egl_display(),
                Some(res.egl_surface),
                Some(res.egl_surface),
                Some(ctx.egl_context()),
            )
            .ok();
        });

        // Hot path: direct access to cached surface, zero lookup overhead
        if let Some(ref mut skia_surface) = *self.cached_surface.borrow_mut() {
            let canvas = skia_surface.canvas();

            // The buffer is the surface's logical size times its scale; the
            // caller draws in points.
            canvas.save();
            canvas.scale((scale, scale));

            // Clear to transparent to prevent glitches from old buffer content
            canvas.clear(skia_safe::Color::TRANSPARENT);

            // Call user's drawing function
            draw_fn(canvas);

            // Restore canvas state to prevent scale accumulation
            canvas.restore();
        }
    }

    /// Initialize the Skia surface (cold path - called once)
    fn initialize_skia_surface(&self, ctx: &mut super::SkiaContext) {
        AppContext::with_egl_resources(&self.surface_id, |res| {
            unsafe {
                // Load EGL - this is cached internally after first load, so cheap
                let egl =
                    khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required().unwrap();

                // A pending resize goes to the `wl_egl_window` both before and
                // after the surface is made current, because the two EGL
                // implementations only honour it on opposite sides:
                // - Mesa takes the back buffer when the surface is made current
                //   and ignores a resize while it holds one, so the size has to
                //   land first. The second call is then a no-op.
                // - NVIDIA's egl-wayland reallocates straight away only for a
                //   current surface. Any other surface is reallocated inside its
                //   next `eglSwapBuffers`, after that frame has been drawn into
                //   the old buffer, which the compositor then stretches over
                //   the new size.
                if res.resize_pending {
                    res.wl_egl_surface.resize(res.width, res.height, 0, 0);
                }

                // Make this surface's EGL surface current
                egl.make_current(
                    ctx.egl_display(),
                    Some(res.egl_surface),
                    Some(res.egl_surface),
                    Some(ctx.egl_context()),
                )
                .ok();

                if res.resize_pending {
                    res.wl_egl_surface.resize(res.width, res.height, 0, 0);
                    res.resize_pending = false;
                }

                // CRITICAL: Disable vsync for THIS surface to prevent blocking
                // Must be called AFTER make_current
                egl.swap_interval(ctx.egl_display(), 0).ok();

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

                // The subpixel layout is the surface's, so Skia can render
                // LCD text for it; `Unknown` keeps text greyscale.
                let props = skia_safe::SurfaceProps::new(
                    skia_safe::SurfacePropsFlags::default(),
                    self.pixel_geometry.get(),
                );
                *self.cached_surface.borrow_mut() =
                    skia_safe::gpu::surfaces::wrap_backend_render_target(
                        ctx.skia_context(),
                        &backend_render_target,
                        skia_safe::gpu::SurfaceOrigin::BottomLeft,
                        skia_safe::ColorType::RGBA8888,
                        None,
                        Some(&props),
                    );
            }
        });
    }

    /// Swap buffers after drawing (cold path)
    pub fn swap_buffers(&self, ctx: &mut super::SkiaContext) {
        AppContext::with_egl_resources(&self.surface_id, |res| {
            unsafe {
                // Flush to GPU
                ctx.skia_context().flush_and_submit();

                let egl =
                    khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required().unwrap();
                egl.swap_buffers(ctx.egl_display(), res.egl_surface).ok();
            }
        });
    }

    /// Swap buffers, telling the compositor only `rects` changed.
    ///
    /// `rects` are in buffer pixels with a top-left origin; EGL wants a
    /// bottom-left one, so they are flipped here. Every pixel of the buffer must
    /// still be correct — this narrows what the compositor recomposites, not
    /// what the client paints. Falls back to a whole-buffer swap on a driver
    /// without `EGL_KHR_swap_buffers_with_damage`, or with nothing to report.
    pub fn swap_buffers_with_damage(
        &self,
        ctx: &mut super::SkiaContext,
        rects: &[skia_safe::IRect],
    ) {
        type SwapWithDamage = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            *const i32,
            i32,
        ) -> i32;

        let swapped = AppContext::with_egl_resources(&self.surface_id, |res| unsafe {
            if rects.is_empty() {
                return false;
            }
            let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required().unwrap();
            let Some(function) = egl.get_proc_address("eglSwapBuffersWithDamageKHR") else {
                return false;
            };
            let function: SwapWithDamage = std::mem::transmute(function);

            let height = res.height;
            let flat: Vec<i32> = rects
                .iter()
                .flat_map(|r| [r.left, height - r.bottom, r.width(), r.height()])
                .collect();

            ctx.skia_context().flush_and_submit();
            function(
                ctx.egl_display().as_ptr() as *mut _,
                res.egl_surface.as_ptr() as *mut _,
                flat.as_ptr(),
                rects.len() as i32,
            ) != 0
        })
        .unwrap_or(false);

        if !swapped {
            self.swap_buffers(ctx);
        }
    }

    /// Commit the surface and mark damage
    pub fn commit(&self) {
        AppContext::with_egl_resources(&self.surface_id, |res| {
            // Damage the entire buffer
            res.wl_surface.damage_buffer(0, 0, res.width, res.height);
            // Commit to make changes visible
            res.wl_surface.commit();
        });
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> wl_surface::WlSurface {
        AppContext::with_egl_resources(&self.surface_id, |res| res.wl_surface.clone()).unwrap()
    }

    /// Get current width
    pub fn width(&self) -> i32 {
        AppContext::with_egl_resources(&self.surface_id, |res| res.width).unwrap_or(0)
    }

    /// Get current height
    pub fn height(&self) -> i32 {
        AppContext::with_egl_resources(&self.surface_id, |res| res.height).unwrap_or(0)
    }

    /// Get surface ID
    pub fn surface_id(&self) -> &ObjectId {
        &self.surface_id
    }
}

impl Drop for SkiaSurface {
    fn drop(&mut self) {
        // Clean up EGL resources from HashMap
        AppContext::remove_egl_resources(&self.surface_id);
        AppContext::forget_focus_ring(&self.surface_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::PixelGeometry;
    use wl_output::{Subpixel, Transform};

    /// Text rendered for the wrong layout fringes in colour, so each
    /// transform has to land a panel's stripes where they really are in the
    /// buffer: a portrait output turns horizontal stripes vertical, and the
    /// direction it turns decides which colour comes first.
    #[test]
    fn subpixel_layout_follows_the_output_transform() {
        let cases = [
            (
                Subpixel::HorizontalRgb,
                Transform::Normal,
                PixelGeometry::RGBH,
            ),
            (
                Subpixel::HorizontalBgr,
                Transform::Normal,
                PixelGeometry::BGRH,
            ),
            (
                Subpixel::VerticalRgb,
                Transform::Normal,
                PixelGeometry::RGBV,
            ),
            (Subpixel::HorizontalRgb, Transform::_90, PixelGeometry::RGBV),
            (
                Subpixel::HorizontalRgb,
                Transform::_180,
                PixelGeometry::BGRH,
            ),
            (
                Subpixel::HorizontalRgb,
                Transform::_270,
                PixelGeometry::BGRV,
            ),
            (Subpixel::VerticalRgb, Transform::_90, PixelGeometry::BGRH),
            (Subpixel::VerticalRgb, Transform::_270, PixelGeometry::RGBH),
            (
                Subpixel::HorizontalRgb,
                Transform::Flipped,
                PixelGeometry::BGRH,
            ),
            (
                Subpixel::HorizontalRgb,
                Transform::Flipped90,
                PixelGeometry::RGBV,
            ),
            (
                Subpixel::HorizontalRgb,
                Transform::Flipped180,
                PixelGeometry::RGBH,
            ),
            (
                Subpixel::HorizontalRgb,
                Transform::Flipped270,
                PixelGeometry::BGRV,
            ),
            (
                Subpixel::VerticalRgb,
                Transform::Flipped90,
                PixelGeometry::RGBH,
            ),
            (Subpixel::Unknown, Transform::Normal, PixelGeometry::Unknown),
            (Subpixel::None, Transform::_90, PixelGeometry::Unknown),
        ];
        for (subpixel, transform, expected) in cases {
            assert_eq!(
                pixel_geometry(subpixel, transform),
                expected,
                "{subpixel:?} under {transform:?}"
            );
        }
    }
}
