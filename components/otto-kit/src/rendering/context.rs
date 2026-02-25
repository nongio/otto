use wayland_client::protocol::wl_surface;
use wayland_client::Proxy;

/// Shared rendering context for all surfaces
/// Holds the EGL display, context, and Skia DirectContext that are shared across all surfaces
pub struct SkiaContext {
    egl_display: khronos_egl::Display,
    egl_context: khronos_egl::Context,
    skia_context: skia_safe::gpu::DirectContext,
}

impl SkiaContext {
    /// Initialize a new SkiaContext from a Wayland display and surface
    ///
    /// This creates the EGL display, context, and Skia DirectContext that will be shared
    /// across all surfaces. The initial surface is used to set up the context but can be
    /// used for rendering afterwards.
    pub fn new(
        wl_display_ptr: *mut std::ffi::c_void,
        initial_surface: &wl_surface::WlSurface,
        width: i32,
        height: i32,
    ) -> Result<(Self, super::SkiaSurface), String> {
        unsafe {
            // Load EGL dynamically
            let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()
                .map_err(|e| format!("Failed to load EGL: {}", e))?;

            // Get EGL display from Wayland display
            let display = egl
                .get_display(wl_display_ptr as khronos_egl::NativeDisplayType)
                .ok_or("Failed to get EGL display")?;

            egl.initialize(display)
                .map_err(|e| format!("Failed to initialize EGL: {}", e))?;

            // Choose config
            let config_attribs = [
                khronos_egl::RED_SIZE,
                8,
                khronos_egl::GREEN_SIZE,
                8,
                khronos_egl::BLUE_SIZE,
                8,
                khronos_egl::ALPHA_SIZE,
                8,
                khronos_egl::RENDERABLE_TYPE,
                khronos_egl::OPENGL_ES2_BIT,
                khronos_egl::NONE,
            ];

            let config = egl
                .choose_first_config(display, &config_attribs)
                .map_err(|e| format!("Failed to choose config: {}", e))?
                .ok_or("No suitable EGL config found")?;

            // Bind OpenGL ES API
            egl.bind_api(khronos_egl::OPENGL_ES_API)
                .map_err(|e| format!("Failed to bind API: {}", e))?;

            // Create context
            let context_attribs = [khronos_egl::CONTEXT_CLIENT_VERSION, 2, khronos_egl::NONE];

            let context = egl
                .create_context(display, config, None, &context_attribs)
                .map_err(|e| format!("Failed to create context: {}", e))?;

            // Create WlEglSurface for initial surface
            let wl_egl_surface =
                wayland_egl::WlEglSurface::new(initial_surface.id(), width, height)
                    .map_err(|e| format!("Failed to create WlEglSurface: {:?}", e))?;

            // Create EGL surface
            let egl_surface = egl
                .create_window_surface(
                    display,
                    config,
                    wl_egl_surface.ptr() as khronos_egl::NativeWindowType,
                    None,
                )
                .map_err(|e| format!("Failed to create EGL surface: {}", e))?;

            // Make context current
            egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
                .map_err(|e| format!("Failed to make context current: {}", e))?;

            // Disable vsync - swap immediately without waiting for vblank
            egl.swap_interval(display, 0)
                .map_err(|e| format!("Failed to set swap interval: {}", e))?;

            // Load GL functions
            gl::load_with(|name| egl.get_proc_address(name).unwrap() as *const _);

            // Create Skia interface
            let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
                egl.get_proc_address(name).unwrap() as *const _
            })
            .ok_or("Failed to create Skia GL interface")?;

            // Create Skia DirectContext
            let skia_context = skia_safe::gpu::direct_contexts::make_gl(interface, None)
                .ok_or("Failed to create Skia DirectContext")?;

            let ctx = Self {
                egl_display: display,
                egl_context: context,
                skia_context,
            };

            let surface = super::SkiaSurface::new_from_parts(
                initial_surface.clone(),
                egl_surface,
                wl_egl_surface,
                width,
                height,
            );

            Ok((ctx, surface))
        }
    }

    pub fn egl_display(&self) -> khronos_egl::Display {
        self.egl_display
    }

    pub fn egl_context(&self) -> khronos_egl::Context {
        self.egl_context
    }

    pub fn skia_context(&mut self) -> &mut skia_safe::gpu::DirectContext {
        &mut self.skia_context
    }

    /// Create a new SkiaSurface from this context for a new Wayland surface
    ///
    /// This allows multiple surfaces to share the same EGL/Skia context
    pub fn create_surface(
        &self,
        wl_surface: &wl_surface::WlSurface,
        width: i32,
        height: i32,
    ) -> Result<super::SkiaSurface, String> {
        unsafe {
            let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()
                .map_err(|e| format!("Failed to load EGL: {}", e))?;

            // Get the same config used for the context
            let config_attribs = [
                khronos_egl::RED_SIZE,
                8,
                khronos_egl::GREEN_SIZE,
                8,
                khronos_egl::BLUE_SIZE,
                8,
                khronos_egl::ALPHA_SIZE,
                8,
                khronos_egl::RENDERABLE_TYPE,
                khronos_egl::OPENGL_ES2_BIT,
                khronos_egl::NONE,
            ];

            let config = egl
                .choose_first_config(self.egl_display, &config_attribs)
                .map_err(|e| format!("Failed to choose config: {}", e))?
                .ok_or("No suitable EGL config found")?;

            // Create WlEglSurface
            let wl_egl_surface = wayland_egl::WlEglSurface::new(wl_surface.id(), width, height)
                .map_err(|e| format!("Failed to create WlEglSurface: {:?}", e))?;

            // Create EGL surface
            let egl_surface = egl
                .create_window_surface(
                    self.egl_display,
                    config,
                    wl_egl_surface.ptr() as khronos_egl::NativeWindowType,
                    None,
                )
                .map_err(|e| format!("Failed to create EGL surface: {}", e))?;

            Ok(super::SkiaSurface::new_from_parts(
                wl_surface.clone(),
                egl_surface,
                wl_egl_surface,
                width,
                height,
            ))
        }
    }
}
