use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::HashMap,
    ffi::{c_char, CStr},
    rc::Rc,
    sync::atomic::Ordering,
};

use layers::{sb, skia};

use smithay::{
    backend::{
        allocator::{
            dmabuf::{Dmabuf, WeakDmabuf},
            format::{has_alpha, FormatSet},
            Buffer as DmaBuffer, Fourcc,
        },
        egl::{self, display::EGLBufferReader, fence::EGLFence, EGLContext, EGLSurface},
        renderer::{
            gles::{
                ffi::{
                    self,
                    types::{GLint, GLuint},
                },
                format::{fourcc_to_gl_formats, gl_internal_format_to_fourcc},
                Capability, GlesError, GlesRenderbuffer, GlesRenderer, GlesTexture,
            },
            sync::SyncPoint,
            Bind, Blit, Color32F, DebugFlags, ExportMem, Frame, ImportDma, ImportDmaWl, ImportEgl,
            ImportMem, ImportMemWl, Offscreen, Renderer, Texture, TextureFilter, Unbind,
        },
    },
    reexports::wayland_server::{protocol::wl_buffer::WlBuffer, DisplayHandle},
    utils::{Buffer, Physical, Rectangle, Size, Transform},
    wayland::compositor::SurfaceData,
};

use crate::renderer::{
    egl_context::EGLSurfaceWrapper,
    skia_surface::SkiaSurface,
    sync::{finished_proc, FlushInfo2, SkiaSync, FINISHED_PROC_STATE},
    textures::{SkiaFrame, SkiaTexture, SkiaTextureMapping},
};

// Re-export types that are part of the public API
pub use crate::renderer::textures::{SkiaGLesFbo, SkiaTextureImage};

pub struct SkiaRenderer {
    gl_renderer: GlesRenderer,
    gl: ffi::Gles2,

    target_renderer: HashMap<SkiaTarget, SkiaSurface>,
    current_target: Option<SkiaTarget>,
    pub buffers: HashMap<SkiaTarget, SkiaGLesFbo>,
    pub context: Option<skia::gpu::DirectContext>,

    dmabuf_cache: std::collections::HashMap<WeakDmabuf, SkiaTexture>,
}

impl From<GlesRenderer> for SkiaRenderer {
    fn from(gl_renderer: GlesRenderer) -> SkiaRenderer {
        let egl = gl_renderer.egl_context();

        let gl = unsafe {
            let res = egl.make_current();
            if res.is_err() {
                panic!("Failed to make current");
            }
            SkiaRenderer::load_gl()
        };

        let mut options = skia::gpu::ContextOptions::default();
        options.skip_gl_error_checks = skia::gpu::ganesh::context_options::Enable::Yes;
        // options.
        let interface = skia::gpu::gl::Interface::new_native().unwrap();
        let mut context = skia::gpu::direct_contexts::make_gl(interface, &options);

        let ctx = context.as_mut().unwrap();
        ctx.reset(None);

        SkiaRenderer {
            gl,
            gl_renderer,
            target_renderer: HashMap::new(),
            buffers: HashMap::new(),
            current_target: None,
            context,
            dmabuf_cache: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkiaTarget {
    // EGLSurface(smithay::backend::egl::ffi::egl::types::EGLSurface),
    #[allow(private_interfaces)]
    EGLSurface(EGLSurfaceWrapper),
    Texture(ffi::types::GLuint),
    Renderbuffer(*const GlesRenderbuffer),
    Dmabuf(Dmabuf),
    Fbo(SkiaGLesFbo),
}
impl std::fmt::Debug for SkiaRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkiaRenderer").finish()
    }
}

impl SkiaRenderer {
    /// # Safety
    ///
    /// This operation will cause undefined behavior if the given EGLContext is active in another thread.
    pub unsafe fn supported_capabilities(
        context: &EGLContext,
    ) -> Result<Vec<Capability>, GlesError> {
        GlesRenderer::supported_capabilities(context)
    }

    /// # Safety
    ///
    /// This operation will cause undefined behavior if the given EGLContext is active in another thread.
    pub unsafe fn load_gl() -> ffi::Gles2 {
        ffi::Gles2::load_with(|s| smithay::backend::egl::get_proc_address(s) as *const _)
    }
    /// # Safety
    ///
    /// This operation will cause undefined behavior if the given EGLContext is active in another thread.
    pub unsafe fn with_capabilities(
        egl: EGLContext,
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Result<SkiaRenderer, GlesError> {
        egl.make_current()?;
        let gl_renderer = GlesRenderer::with_capabilities(egl, capabilities)?;
        // let egl = gl_renderer.egl_context();
        let interface = skia::gpu::gl::Interface::new_native().unwrap();
        let context = skia::gpu::direct_contexts::make_gl(interface, None);

        let (gl, _exts) = {
            let gl =
                ffi::Gles2::load_with(|s| smithay::backend::egl::get_proc_address(s) as *const _);
            let ext_ptr = gl.GetString(ffi::EXTENSIONS) as *const c_char;
            if ext_ptr.is_null() {
                return Err(GlesError::GLFunctionLoaderError);
            }

            let exts = {
                let p = CStr::from_ptr(ext_ptr);
                let list =
                    String::from_utf8(p.to_bytes().to_vec()).unwrap_or_else(|_| String::new());
                list.split(' ').map(|e| e.to_string()).collect::<Vec<_>>()
            };

            tracing::info!("Initializing OpenGL ES Renderer");
            tracing::info!(
                "GL Version: {:?}",
                CStr::from_ptr(gl.GetString(ffi::VERSION) as *const c_char)
            );
            tracing::info!(
                "GL Vendor: {:?}",
                CStr::from_ptr(gl.GetString(ffi::VENDOR) as *const c_char)
            );
            tracing::info!(
                "GL Renderer: {:?}",
                CStr::from_ptr(gl.GetString(ffi::RENDERER) as *const c_char)
            );
            tracing::info!("Supported GL Extensions: {:?}", exts);

            // let gl_version = smithay::backend::renderer::gles::version::GlVersion::try_from(&gl).unwrap_or_else(|_| {
            //     tracing::warn!("Failed to detect GLES version, defaulting to 2.0");
            //     version::GLES_2_0
            // });

            // required for the manditory wl_shm formats
            if !exts
                .iter()
                .any(|ext| ext == "GL_EXT_texture_format_BGRA8888")
            {
                return Err(GlesError::GLExtensionNotSupported(&[
                    "GL_EXT_texture_format_BGRA8888",
                ]));
            }

            // required for buffers without linear memory layout
            // if gl_version < version::GLES_3_0
            //     && !exts.iter().any(|ext| ext == "GL_EXT_unpack_subimage")
            // {
            //     return Err(GlesError::GLExtensionNotSupported(&[
            //         "GL_EXT_unpack_subimage",
            //     ]));
            // }

            // let gl_debug_span = if requested_capabilities.contains(&Capability::Debug) {
            //     gl.Enable(ffi::DEBUG_OUTPUT);
            //     gl.Enable(ffi::DEBUG_OUTPUT_SYNCHRONOUS);
            //     let span = Box::into_raw(Box::new(span.clone()));
            //     gl.DebugMessageCallback(Some(gl_debug_log), span as *mut _);
            //     Some(span)
            // } else {
            //     None
            // };

            // (gl, gl_version, exts, requested_capabilities, gl_debug_span)
            (gl, exts)
        };

        Ok(SkiaRenderer {
            gl_renderer,
            gl,
            target_renderer: HashMap::new(),
            buffers: HashMap::new(),
            current_target: None,
            context,
            dmabuf_cache: std::collections::HashMap::new(),
        })
    }

    /// # Safety
    ///
    /// This operation will cause undefined behavior if the given EGLContext is active in another thread.
    pub unsafe fn new(context: EGLContext) -> Result<SkiaRenderer, GlesError> {
        let supported_capabilities = Self::supported_capabilities(&context)?;
        Self::with_capabilities(context, supported_capabilities)
    }
    pub fn egl_context(&self) -> &EGLContext {
        self.gl_renderer.egl_context()
    }
    pub fn current_skia_renderer(&mut self) -> Option<&SkiaSurface> {
        let renderer = self
            .current_target
            .as_ref()
            .and_then(|current_target| self.target_renderer.get(current_target));
        renderer
    }
    #[profiling::function]
    fn create_texture_and_framebuffer(
        &self,
        width: i32,
        height: i32,
        format: Fourcc,
    ) -> SkiaGLesFbo {
        let mut texture: GLuint = 0;
        let mut framebuffer: GLuint = 0;

        let (internal_format, read_format, read_type) = fourcc_to_gl_formats(format).unwrap();
        unsafe {
            // Generate and bind the texture
            self.gl.GenTextures(1, &mut texture);
            self.gl.BindTexture(ffi::TEXTURE_2D, texture);

            // Set the texture parameters
            self.gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_MIN_FILTER,
                ffi::LINEAR as GLint,
            );
            self.gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_MAG_FILTER,
                ffi::LINEAR as GLint,
            );
            self.gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_S,
                ffi::MIRRORED_REPEAT as GLint,
            );
            self.gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_T,
                ffi::MIRRORED_REPEAT as GLint,
            );

            // Allocate the texture storage
            self.gl.TexImage2D(
                ffi::TEXTURE_2D,
                0,
                internal_format as GLint,
                width,
                height,
                0,
                read_format,
                read_type,
                std::ptr::null(),
            );

            // Generate and bind the framebuffer
            self.gl.GenFramebuffers(1, &mut framebuffer);
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, framebuffer);

            // Attach the texture to the framebuffer
            self.gl.FramebufferTexture2D(
                ffi::FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                texture,
                0,
            );

            // Check that the framebuffer is complete
            if self.gl.CheckFramebufferStatus(ffi::FRAMEBUFFER) != ffi::FRAMEBUFFER_COMPLETE {
                panic!("Failed to create complete framebuffer");
            }

            // Unbind the framebuffer
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
        }

        SkiaGLesFbo {
            fbo: framebuffer,
            tex_id: texture,
            format,
            origin: skia::gpu::SurfaceOrigin::TopLeft,
        }
    }

    fn import_dmabuf_internal(
        &mut self,
        dmabuf: &Dmabuf,
        damage: Option<&[Rectangle<i32, Buffer>]>,
    ) -> Result<<Self as Renderer>::TextureId, <Self as Renderer>::Error> {
        use smithay::backend::allocator::Buffer;
        let damage = damage.map(|damage| damage.to_vec());

        // self.make_current()?;

        let texture = self
            .existing_dmabuf_texture(dmabuf)?
            .map(Ok)
            .unwrap_or_else(|| {
                let is_external = !self
                    .egl_context()
                    .dmabuf_render_formats()
                    .contains(&dmabuf.format());

                let egl_image = self
                    .egl_context()
                    .display()
                    .create_image_from_dmabuf(dmabuf)
                    .map_err(GlesError::BindBufferEGLError)?;

                let format = fourcc_to_gl_formats(dmabuf.format().code)
                    .map(|(internal, _, _)| internal)
                    .unwrap_or(ffi::RGBA8);
                let has_alpha = has_alpha(dmabuf.format().code);

                // If external, resolve/blit into a TEXTURE_2D so Skia can sample it reliably.
                let (tex_id, skia_external_flag) = if is_external {
                    let dst = self.create_texture_and_framebuffer(
                        dmabuf.size().w,
                        dmabuf.size().h,
                        dmabuf.format().code,
                    );
                    self.blit_eglimage_to_2d_texture(egl_image, dst.tex_id, dmabuf.size())?;
                    (dst.tex_id, false)
                } else {
                    let tex = self.import_egl_image(egl_image, is_external, None)?;
                    (tex, false)
                };

                let gles_texture = unsafe {
                    GlesTexture::from_raw(
                        &self.gl_renderer,
                        Some(format),
                        !has_alpha,
                        tex_id,
                        dmabuf.size(),
                    )
                };
                let image = self
                    .import_skia_image_from_texture(&gles_texture, skia_external_flag)
                    .ok_or("")
                    .map_err(|_| GlesError::MappingError)?;

                let texture = SkiaTexture {
                    texture: gles_texture,
                    image,
                    has_alpha,
                    format: Some(dmabuf.format().code),
                    egl_images: Some(vec![egl_image]),
                    // Preserve original is_external to drive update path in reuse
                    is_external,
                    damage: damage.clone(),
                };

                self.dmabuf_cache.insert(dmabuf.weak(), texture.clone());
                Ok(texture)
            });
        texture.map(|mut tex| {
            tex.image = self
                .import_skia_image_from_texture(&tex.texture, false)
                .unwrap();
            tex.damage = damage.clone();
            // println!("SkiaRenderer: import_dmabuf_internal END");
            tex
        })
    }
    #[profiling::function]
    fn existing_dmabuf_texture(&self, buffer: &Dmabuf) -> Result<Option<SkiaTexture>, GlesError> {
        let existing_texture = self
            .dmabuf_cache
            .iter()
            .find(|(weak, _)| {
                weak.upgrade()
                    .map(|entry| &entry == buffer)
                    .unwrap_or(false)
            })
            .map(|(_, tex)| tex.clone());

        if let Some(texture) = existing_texture {
            if let Some(egl_images) = texture.egl_images.as_ref() {
                if egl_images[0] == smithay::backend::egl::ffi::egl::NO_IMAGE_KHR {
                    return Ok(None);
                }
                if texture.is_external {
                    // For external sources, refresh the 2D texture by blitting from the EGLImage.
                    let size =
                        Size::<i32, Buffer>::from((texture.image.width(), texture.image.height()));
                    self.blit_eglimage_to_2d_texture(
                        egl_images[0],
                        texture.texture.tex_id(),
                        size,
                    )?;
                } else {
                    // For non-external, rebind the EGLImage to the existing 2D texture.
                    let tex = Some(texture.texture.tex_id());
                    self.import_egl_image(egl_images[0], texture.is_external, tex)?;
                }
            }
            Ok(Some(texture))
        } else {
            Ok(None)
        }
    }

    #[profiling::function]
    fn import_egl_image(
        &self,
        image: smithay::backend::egl::ffi::egl::types::EGLImage,
        is_external: bool,
        tex: Option<u32>,
    ) -> Result<u32, GlesError> {
        let tex = tex.unwrap_or_else(|| unsafe {
            let mut tex = 0;
            self.gl.GenTextures(1, &mut tex);
            tex
        });
        let target = if is_external {
            ffi::TEXTURE_EXTERNAL_OES
        } else {
            ffi::TEXTURE_2D
        };
        unsafe {
            self.gl.BindTexture(target, tex);
            // External textures only support a subset of params; ensure valid defaults.
            // Use linear filtering and clamp-to-edge, and restrict to base level 0.
            self.gl
                .TexParameteri(target, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as GLint);
            self.gl
                .TexParameteri(target, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as GLint);
            self.gl
                .TexParameteri(target, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as GLint);
            self.gl
                .TexParameteri(target, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as GLint);
            self.gl.TexParameteri(target, ffi::TEXTURE_BASE_LEVEL, 0);
            self.gl.TexParameteri(target, ffi::TEXTURE_MAX_LEVEL, 0);

            self.gl.EGLImageTargetTexture2DOES(target, image);
            self.gl.BindTexture(target, 0);
        }

        Ok(tex)
    }
}

impl Frame for SkiaFrame<'_> {
    type Error = GlesError;
    type TextureId = SkiaTexture;

    fn id(&self) -> usize {
        // self.renderer.id()
        self.id
    }
    fn clear(
        &mut self,
        color: Color32F,
        at: &[Rectangle<i32, Physical>],
    ) -> Result<(), Self::Error> {
        self.draw_solid(Rectangle::from_loc_and_size((0, 0), self.size), at, color)?;
        Ok(())
    }
    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), Self::Error> {
        if damage.is_empty() {
            return Ok(());
        }

        let dest_rect = skia::Rect::from_xywh(
            dst.loc.x as f32,
            dst.loc.y as f32,
            dst.size.w as f32,
            dst.size.h as f32,
        );

        let color = skia::Color4f::new(color.r(), color.g(), color.b(), color.a());
        let mut paint = skia::Paint::new(color, None);
        paint.set_blend_mode(skia::BlendMode::Src);

        let mut surface = self.skia_surface.clone();
        let canvas = surface.canvas();

        // Render each damage rect with clipping for true partial rendering
        for rect in damage.iter() {
            let rect_constrained_loc = rect
                .loc
                .constrain(Rectangle::from_extemities((0, 0), dst.size.to_point()));
            let rect_clamped_size = rect.size.clamp(
                (0, 0),
                (dst.size.to_point() - rect_constrained_loc).to_size(),
            );

            if rect_clamped_size.w <= 0 || rect_clamped_size.h <= 0 {
                continue;
            }

            let clip_rect = skia::Rect::from_xywh(
                (dst.loc.x + rect_constrained_loc.x) as f32,
                (dst.loc.y + rect_constrained_loc.y) as f32,
                rect_clamped_size.w as f32,
                rect_clamped_size.h as f32,
            );

            canvas.save();
            canvas.clip_rect(clip_rect, None, None);
            canvas.draw_rect(dest_rect, &paint);
            canvas.restore();
        }

        Ok(())
    }
    #[profiling::function]
    fn render_texture_from_to(
        &mut self,
        texture: &Self::TextureId,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        src_transform: Transform,
        alpha: f32,
    ) -> Result<(), Self::Error> {
        if damage.is_empty() {
            return Ok(());
        }

        let image = &texture.image;

        let mut paint = skia::Paint::new(skia::Color4f::new(1.0, 1.0, 1.0, alpha), None);
        paint.set_blend_mode(skia::BlendMode::SrcOver);

        let mut matrix = skia::Matrix::new_identity();

        let mut surface = self.skia_surface.clone();
        let canvas = surface.canvas();

        let scale_x = dst.size.w as f32 / src.size.w as f32;
        let scale_y = dst.size.h as f32 / src.size.h as f32;

        match src_transform {
            Transform::Normal => {
                matrix.pre_scale((scale_x, scale_y), None);
                matrix.pre_translate((
                    dst.loc.x as f32 / scale_x - (src.loc.x as f32),
                    dst.loc.y as f32 / scale_y - (src.loc.y as f32),
                ));
            }
            Transform::Flipped180 => {
                matrix.pre_scale((scale_x, -scale_y), None);
                matrix.pre_translate((
                    dst.loc.x as f32 / scale_x - src.loc.x as f32,
                    -dst.loc.y as f32 / scale_y + src.loc.y as f32,
                ));
            }
            Transform::Flipped90 => {
                panic!("unhandled transform {:?}", src_transform);
            }
            Transform::Flipped270 => {
                panic!("unhandled transform {:?}", src_transform);
            }
            _ => {
                panic!("unhandled transform {:?}", src_transform);
            }
        }

        // Setup shader once outside loop
        paint.set_shader(image.to_shader(
            (skia::TileMode::Repeat, skia::TileMode::Repeat),
            skia::SamplingOptions::default(),
            &matrix,
        ));

        let draw_rect = skia::Rect::from_xywh(
            dst.loc.x as f32,
            dst.loc.y as f32,
            dst.size.w as f32,
            dst.size.h as f32,
        );

        // Render only damaged regions with per-rect clipping
        for rect in damage.iter() {
            let rect_constrained_loc = rect
                .loc
                .constrain(Rectangle::from_extemities((0, 0), dst.size.to_point()));
            let rect_clamped_size = rect.size.clamp(
                (0, 0),
                (dst.size.to_point() - rect_constrained_loc).to_size(),
            );

            if rect_clamped_size.w <= 0 || rect_clamped_size.h <= 0 {
                continue;
            }

            let clip_rect = skia::Rect::from_xywh(
                (dst.loc.x + rect_constrained_loc.x) as f32,
                (dst.loc.y + rect_constrained_loc.y) as f32,
                rect_clamped_size.w as f32,
                rect_clamped_size.h as f32,
            );

            canvas.save();
            canvas.clip_rect(clip_rect, None, None);
            canvas.draw_rect(draw_rect, &paint);
            canvas.restore();
        }

        Ok(())
    }
    fn transformation(&self) -> Transform {
        // self.frame.transformation()
        Transform::Normal
    }
    #[profiling::function]
    fn finish(self) -> Result<SyncPoint, Self::Error> {
        let mut surface = self.skia_surface;

        let info = FlushInfo2 {
            fNumSemaphores: 0,
            fGpuStatsFlags: 0,
            fSignalSemaphores: std::ptr::null_mut(),
            fFinishedProc: Some(finished_proc),
            fFinishedWithStatsProc: None,
            fFinishedContext: std::ptr::null_mut(),
            fSubmittedProc: None,
            fSubmittedContext: std::ptr::null_mut(),
        };

        // Transmute flushinfo2 into flushinfo
        let info = unsafe {
            let native = &*(&info as *const FlushInfo2 as *const sb::GrFlushInfo);
            &*(native as *const sb::GrFlushInfo as *const layers::skia::gpu::FlushInfo)
        };

        FINISHED_PROC_STATE.store(false, Ordering::SeqCst);

        let semaphores = surface.gr_context.flush(info);

        // FIXME review sync logic
        let syncpoint = if semaphores == skia::gpu::SemaphoresSubmitted::Yes {
            profiling::scope!("FINISHED_PROC_STATE");
            let skia_sync = SkiaSync::create(self.renderer.egl_context().display())
                .map_err(|_err| GlesError::FramebufferBindingError)?;
            SyncPoint::from(skia_sync)
        } else {
            SyncPoint::signaled()
        };

        {
            profiling::scope!("context_submit");
            surface.gr_context.submit(None);
            // surface
            //     .gr_context
            //     .flush_and_submit_surface(&mut surface.surface, GrSyncCpu::Yes);
        }

        Ok(syncpoint)
    }

    fn wait(
        &mut self,
        sync: &smithay::backend::renderer::sync::SyncPoint,
    ) -> Result<(), Self::Error> {
        sync.wait()
            .map_err(|_| GlesError::FramebufferBindingError)?;
        Ok(())
    }
}

impl Renderer for SkiaRenderer {
    type Error = GlesError;
    type TextureId = SkiaTexture;
    type Frame<'a> = SkiaFrame<'a>;

    fn id(&self) -> usize {
        99
    }
    fn downscale_filter(&mut self, filter: TextureFilter) -> Result<(), Self::Error> {
        self.gl_renderer.downscale_filter(filter)
    }
    fn upscale_filter(&mut self, filter: TextureFilter) -> Result<(), Self::Error> {
        self.gl_renderer.upscale_filter(filter)
    }
    fn set_debug_flags(&mut self, flags: DebugFlags) {
        self.gl_renderer.set_debug_flags(flags)
    }
    fn debug_flags(&self) -> DebugFlags {
        self.gl_renderer.debug_flags()
    }
    #[profiling::function]
    fn render(
        &mut self,
        output_size: Size<i32, Physical>,
        _dst_transform: Transform,
    ) -> Result<Self::Frame<'_>, Self::Error> {
        let id = self.id();
        let current_target = self.current_target.as_ref().unwrap();
        let buffer = self.buffers.get(current_target).unwrap();

        self.target_renderer
            .entry(current_target.clone())
            .or_insert_with(|| {
                {
                    let ctx = self.context.as_mut().unwrap();
                    ctx.reset(None);
                }
                let context = self.context.as_ref();
                let color_type = match buffer.format {
                    Fourcc::Argb8888 => skia::ColorType::RGBA8888,
                    Fourcc::Abgr2101010 => skia::ColorType::RGBA1010102,
                    _ => skia::ColorType::RGBA8888,
                };
                let gl_internal = fourcc_to_gl_formats(buffer.format)
                    .map(|(internal, _, _)| internal)
                    .unwrap_or(ffi::RGBA8);
                SkiaSurface::new_with_fbo(
                    output_size.w,
                    output_size.h,
                    0_usize,
                    8_usize,
                    buffer.fbo,
                    color_type,
                    context,
                    buffer.origin,
                    gl_internal as u32,
                )
                // SkiaSurface::new_with_texture(
                //     output_size.w,
                //     output_size.h,
                //     0_usize,
                //     // 8_usize,
                //     buffer.tex_id,
                //     color_type,
                //     context,
                //     buffer.origin,
                // )
            });

        unsafe {
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, buffer.fbo);

            let status = self.gl.CheckFramebufferStatus(ffi::FRAMEBUFFER);
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);

            if status != ffi::FRAMEBUFFER_COMPLETE {
                println!("framebuffer incomplete");
                return Err(GlesError::FramebufferBindingError);
            }
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
        }
        let surface = self
            .target_renderer
            .get_mut(self.current_target.as_ref().unwrap())
            .unwrap();

        Ok(SkiaFrame {
            skia_surface: surface.clone(),
            size: output_size,
            renderer: self,
            id,
        })
    }

    fn wait(
        &mut self,
        sync: &smithay::backend::renderer::sync::SyncPoint,
    ) -> Result<(), Self::Error> {
        let display = self.egl_context().display();

        // if the sync point holds a EGLFence we can try
        // to directly insert it in our context
        if let Some(fence) = sync.get::<EGLFence>() {
            if fence.wait(display).is_ok() {
                return Ok(());
            }
        }

        // alternative we try to create a temporary fence
        // out of the native fence if available and try
        // to insert it in our context
        if let Some(native) = EGLFence::supports_importing(display)
            .then(|| sync.export())
            .flatten()
        {
            if let Ok(fence) = EGLFence::import(display, native) {
                if fence.wait(display).is_ok() {
                    return Ok(());
                }
            }
        }

        // if everything above failed we can only
        // block until the sync point has been reached
        sync.wait().map_err(|_| GlesError::SyncInterrupted)
    }
}

impl SkiaRenderer {
    #[profiling::function]
    fn import_skia_image_from_texture(
        &mut self,
        texture: &GlesTexture,
        is_external: bool,
    ) -> Option<skia::Image> {
        #[cfg(feature = "profile-with-puffin")]
        profiling::scope!("import_skia_image_from_texture");
        let context = self.context.as_mut().unwrap();

        let target = if is_external {
            ffi::TEXTURE_EXTERNAL_OES
        } else {
            ffi::TEXTURE_2D
        };

        let size = skia::Point {
            x: texture.width() as f32,
            y: texture.height() as f32,
        };
        unsafe {
            let gl_format = texture.format().map_or(ffi::RGBA8, |fourcc| {
                fourcc_to_gl_formats(fourcc).map_or(ffi::RGBA8, |(internal, _, _)| internal)
            });
            let skia_format = match gl_format {
                ffi::RGBA | ffi::RGBA8 => Some(skia::gpu::gl::Format::RGBA8),
                ffi::BGRA_EXT => Some(skia::gpu::gl::Format::BGRA8),
                ffi::RGB8 => Some(skia::gpu::gl::Format::RGB8),
                ffi::RGB10_A2 => Some(skia::gpu::gl::Format::RGB10_A2),
                ffi::RGBA16F => Some(skia::gpu::gl::Format::RGBA16F),
                _ => None,
            };
            let skia_color = match gl_format {
                ffi::RGBA | ffi::RGBA8 => Some(skia::ColorType::RGBA8888),
                ffi::BGRA_EXT => Some(skia::ColorType::BGRA8888),
                ffi::RGB8 => Some(skia::ColorType::RGB888x),
                ffi::RGB10_A2 => Some(skia::ColorType::RGBA1010102),
                ffi::RGBA16F => Some(skia::ColorType::RGBAF16),
                _ => None,
            };
            let texture_info = skia::gpu::gl::TextureInfo {
                target,
                id: texture.tex_id(),
                format: skia_format.unwrap().into(),
                ..Default::default()
            };

            let texture = skia::gpu::backend_textures::make_gl(
                (size.x as i32, size.y as i32),
                skia::gpu::Mipmapped::No,
                texture_info,
                "",
            );

            let image = skia::Image::from_texture(
                context,
                &texture,
                skia::gpu::SurfaceOrigin::TopLeft,
                skia_color.unwrap(),
                skia::AlphaType::Premul,
                None,
            )
            .unwrap();
            if let Some(surface) = self.current_skia_renderer() {
                let mut ctx = surface.gr_context.clone();
                ctx.flush_and_submit_image(&image);
                // ctx.flush_submit_and_sync_cpu();
                // println!("flush image");
            }
            Some(image)
        }
    }
}

impl SkiaRenderer {
    #[profiling::function]
    fn blit_eglimage_to_2d_texture(
        &self,
        image: smithay::backend::egl::ffi::egl::types::EGLImage,
        dst_tex: u32,
        size: smithay::utils::Size<i32, Buffer>,
    ) -> Result<(), GlesError> {
        unsafe {
            // Create source renderbuffer and FBO from EGLImage
            let mut src_rbo = 0;
            self.gl.GenRenderbuffers(1, &mut src_rbo);
            self.gl.BindRenderbuffer(ffi::RENDERBUFFER, src_rbo);
            self.gl
                .EGLImageTargetRenderbufferStorageOES(ffi::RENDERBUFFER, image);
            let mut src_fbo = 0;
            self.gl.GenFramebuffers(1, &mut src_fbo);
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, src_fbo);
            self.gl.FramebufferRenderbuffer(
                ffi::READ_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::RENDERBUFFER,
                src_rbo,
            );

            // Create destination FBO and attach the 2D destination texture
            let mut dst_fbo = 0;
            self.gl.GenFramebuffers(1, &mut dst_fbo);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, dst_fbo);
            self.gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                dst_tex,
                0,
            );

            // Blit from src (READ) to dst (DRAW)
            self.gl.BlitFramebuffer(
                0,
                0,
                size.w,
                size.h,
                0,
                0,
                size.w,
                size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::NEAREST,
            );

            // Cleanup bindings and temp objects
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, 0);
            self.gl.DeleteFramebuffers(1, &src_fbo);
            self.gl.DeleteFramebuffers(1, &dst_fbo);
            self.gl.BindRenderbuffer(ffi::RENDERBUFFER, 0);
            self.gl.DeleteRenderbuffers(1, &src_rbo);
        }
        Ok(())
    }

    /// Get the FBO info for the current render target
    pub fn get_current_fbo(&self) -> Result<&SkiaGLesFbo, GlesError> {
        self.current_target
            .as_ref()
            .and_then(|target| self.buffers.get(target))
            .ok_or(GlesError::FramebufferBindingError)
    }

    /// Blit between two framebuffers
    #[profiling::function]
    pub fn blit_fbo_to_fbo(
        &self,
        src_fbo: u32,
        dst_fbo: u32,
        size: Size<i32, Buffer>,
    ) -> Result<(), GlesError> {
        unsafe {
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, src_fbo);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, dst_fbo);

            self.gl.BlitFramebuffer(
                0,
                0,
                size.w,
                size.h,
                0,
                0,
                size.w,
                size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );

            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, 0);
        }
        Ok(())
    }
}
impl ImportMemWl for SkiaRenderer {
    #[profiling::function]
    fn import_shm_buffer(
        &mut self,
        buffer: &WlBuffer,
        surface: Option<&SurfaceData>,
        damage: &[Rectangle<i32, Buffer>],
    ) -> Result<<Self as Renderer>::TextureId, <Self as Renderer>::Error> {
        let texture = self
            .gl_renderer
            .import_shm_buffer(buffer, surface, damage)?;
        let has_alpha = texture
            .format()
            .is_some_and(|fourcc: Fourcc| has_alpha(fourcc));
        let image = self
            .import_skia_image_from_texture(&texture, false)
            .ok_or("")
            .map_err(|_| GlesError::MappingError)?;

        let format = texture.format();
        Ok(SkiaTexture {
            texture,
            image,
            has_alpha,
            format,
            egl_images: None,
            is_external: false,
            damage: Some(damage.to_vec()),
        })
    }
    fn shm_formats(
        &self,
    ) -> Box<dyn Iterator<Item = smithay::reexports::wayland_server::protocol::wl_shm::Format>>
    {
        self.gl_renderer.shm_formats()
    }
}

impl ImportEgl for SkiaRenderer {
    fn bind_wl_display(&mut self, display: &DisplayHandle) -> Result<(), egl::Error> {
        self.gl_renderer.bind_wl_display(display)
    }

    fn unbind_wl_display(&mut self) {
        self.gl_renderer.unbind_wl_display()
    }

    fn egl_reader(&self) -> Option<&EGLBufferReader> {
        self.gl_renderer.egl_reader()
    }
    #[profiling::function]
    fn import_egl_buffer(
        &mut self,
        buffer: &WlBuffer,
        surface: Option<&SurfaceData>,
        damage: &[Rectangle<i32, Buffer>],
    ) -> Result<<Self as Renderer>::TextureId, <Self as Renderer>::Error> {
        let texture = self
            .gl_renderer
            .import_egl_buffer(buffer, surface, damage)?;
        let has_alpha = texture
            .format()
            .is_some_and(|fourcc: Fourcc| has_alpha(fourcc));
        let image = self
            .import_skia_image_from_texture(&texture, false)
            .ok_or("")
            .map_err(|_| GlesError::MappingError)?;

        let format = texture.format();
        if let SkiaTarget::EGLSurface(EGLSurfaceWrapper(surface)) =
            self.current_target.as_ref().unwrap()
        {
            unsafe {
                self.egl_context().make_current_with_surface(surface)?;
                self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
            }
        }

        Ok(SkiaTexture {
            texture,
            image,
            has_alpha,
            format,
            egl_images: None,
            is_external: false,
            damage: Some(damage.to_vec()),
        })
    }
}

impl ImportDma for SkiaRenderer {
    #[profiling::function]
    fn import_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        damage: Option<&[Rectangle<i32, Buffer>]>,
    ) -> Result<<Self as Renderer>::TextureId, <Self as Renderer>::Error> {
        // self.gl_renderer.import_dmabuf(dmabuf, damage)
        let texture = self.import_dmabuf_internal(dmabuf, damage)?;
        Ok(texture)
    }
    fn dmabuf_formats(&self) -> FormatSet {
        self.gl_renderer.dmabuf_formats()
    }
}

impl ImportDmaWl for SkiaRenderer {
    fn import_dma_buffer(
        &mut self,
        buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
        _surface: Option<&smithay::wayland::compositor::SurfaceData>,
        damage: &[Rectangle<i32, Buffer>],
    ) -> Result<<Self as Renderer>::TextureId, <Self as Renderer>::Error> {
        let dmabuf = smithay::wayland::dmabuf::get_dmabuf(buffer)
            .expect("import_dma_buffer without checking buffer type?");
        self.import_dmabuf(dmabuf, Some(damage))
    }
}

impl ImportMem for SkiaRenderer {
    #[profiling::function]
    fn import_memory(
        &mut self,
        data: &[u8],
        format: Fourcc,
        size: Size<i32, Buffer>,
        flipped: bool,
    ) -> Result<<Self as Renderer>::TextureId, <Self as Renderer>::Error> {
        let texture = self
            .gl_renderer
            .import_memory(data, format, size, flipped)?;
        let image = self
            .import_skia_image_from_texture(&texture, false)
            .ok_or("")
            .map_err(|_| GlesError::MappingError)?;
        let has_alpha = has_alpha(format);

        if let Some(SkiaTarget::EGLSurface(EGLSurfaceWrapper(surface))) =
            self.current_target.as_ref()
        {
            unsafe {
                self.egl_context().make_current_with_surface(surface)?;
                self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
            }
        }
        let format = texture.format();
        Ok(SkiaTexture {
            texture,
            image,
            has_alpha,
            format,
            egl_images: None,
            is_external: false,
            damage: None,
        })
    }
    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        self.gl_renderer.mem_formats()
    }
    fn update_memory(
        &mut self,
        texture: &<Self as Renderer>::TextureId,
        data: &[u8],
        region: Rectangle<i32, Buffer>,
    ) -> Result<(), <Self as Renderer>::Error> {
        self.gl_renderer
            .update_memory(&texture.texture, data, region)
    }
}

impl Borrow<GlesRenderer> for SkiaRenderer {
    fn borrow(&self) -> &GlesRenderer {
        &self.gl_renderer
    }
}

impl BorrowMut<GlesRenderer> for SkiaRenderer {
    fn borrow_mut(&mut self) -> &mut GlesRenderer {
        &mut self.gl_renderer
    }
}

impl ExportMem for SkiaRenderer {
    type TextureMapping = SkiaTextureMapping;

    // Copies a region of the framebuffer into a texture and returns a TextureMapping
    fn copy_framebuffer(
        &mut self,
        region: Rectangle<i32, Buffer>,
        fourcc: Fourcc,
    ) -> Result<Self::TextureMapping, <Self as Renderer>::Error> {
        // Just store the FBO info - don't create image or read pixels yet
        let fbo_info = self.get_current_fbo()?.clone();

        Ok(Self::TextureMapping {
            fourcc_format: fourcc,
            flipped: false,
            width: region.size.w as u32,
            height: region.size.h as u32,
            fbo_info,
            region,
            image: RefCell::new(None),
            data: RefCell::new(None),
        })
    }
    fn copy_texture(
        &mut self,
        _texture: &Self::TextureId,
        _region: Rectangle<i32, Buffer>,
        _fourcc: Fourcc,
    ) -> Result<Self::TextureMapping, Self::Error> {
        unimplemented!("copy_texture")
    }

    fn map_texture<'a>(
        &mut self,
        texture_mapping: &'a Self::TextureMapping,
    ) -> Result<&'a [u8], <Self as Renderer>::Error> {
        // Lazy-load the pixel data if not already loaded
        let mut data_opt = texture_mapping.data.borrow_mut();

        if data_opt.is_none() {
            // Need to read pixels from the FBO now
            let region = texture_mapping.region;
            let fourcc = texture_mapping.fourcc_format;

            // Get the current surface and read pixels
            let renderer = self
                .current_skia_renderer()
                .ok_or(GlesError::FramebufferBindingError)?;
            let mut surface = renderer.surface();

            let (_, read_format, _) =
                fourcc_to_gl_formats(fourcc).ok_or(GlesError::UnknownPixelFormat)?;
            let color_type = match read_format {
                ffi::BGRA_EXT => skia::ColorType::BGRA8888,
                _ => skia::ColorType::RGBA8888,
            };
            let info = skia::ImageInfo::new(
                skia::ISize::new(region.size.w, region.size.h),
                color_type,
                skia::AlphaType::Premul,
                None,
            );

            let len = region.size.w * region.size.h * 4;
            let mut data_vec = vec![0; len as usize];
            let byte_row = info.min_row_bytes();

            if !surface.read_pixels(&info, &mut data_vec, byte_row, (region.loc.x, region.loc.y)) {
                return Err(GlesError::MappingError);
            }

            *data_opt = Some(data_vec);
        }

        let data_ref = data_opt.as_ref().unwrap();
        let len = data_ref.len();
        let ptr = data_ref.as_ptr();

        unsafe { Ok(std::slice::from_raw_parts(ptr, len)) }
    }

    fn can_read_texture(&mut self, texture: &Self::TextureId) -> Result<bool, Self::Error> {
        self.gl_renderer.can_read_texture(&texture.texture)
    }
}

impl Bind<Rc<EGLSurface>> for SkiaRenderer {
    fn bind(&mut self, surface: Rc<EGLSurface>) -> Result<(), <Self as Renderer>::Error> {
        unsafe {
            self.egl_context().make_current_with_surface(&surface)?;
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
        }
        let egl_surface = EGLSurfaceWrapper(surface.clone());
        let render_target = SkiaTarget::EGLSurface(egl_surface);

        if !self.buffers.contains_key(&render_target) {
            let format = surface.pixel_format();
            let format = match (format.color_bits, format.alpha_bits) {
                (24, 8) => ffi::RGB8,
                (30, 2) => ffi::RGB10_A2,
                (48, 16) => ffi::RGB16F,
                _ => ffi::RGB8,
            };
            let sfbo = SkiaGLesFbo {
                fbo: 0,
                tex_id: 0,
                format: gl_internal_format_to_fourcc(format).unwrap(),
                origin: skia::gpu::SurfaceOrigin::BottomLeft,
            };
            self.buffers.insert(render_target.clone(), sfbo);
        }
        self.current_target = Some(render_target);

        Ok(())
    }
}

impl Bind<SkiaGLesFbo> for SkiaRenderer {
    fn bind(&mut self, texture: SkiaGLesFbo) -> Result<(), <Self as Renderer>::Error> {
        self.current_target = Some(SkiaTarget::Fbo(texture.clone()));
        self.buffers
            .insert(SkiaTarget::Fbo(texture.clone()), texture);

        Ok(())
    }
}

impl Bind<GlesTexture> for SkiaRenderer {
    fn bind(&mut self, texture: GlesTexture) -> Result<(), <Self as Renderer>::Error> {
        self.current_target = Some(SkiaTarget::Texture(texture.tex_id()));
        // let res = self.gl_renderer.bind(texture);
        unimplemented!("bind GlesTexture")

        // Ok(())
    }
}

impl Bind<GlesRenderbuffer> for SkiaRenderer {
    fn bind(&mut self, target: GlesRenderbuffer) -> Result<(), <Self as Renderer>::Error> {
        self.current_target = Some(SkiaTarget::Renderbuffer(&target));
        // let res = self.gl_renderer.bind(target);
        unimplemented!("bind GlesRenderbuffer")
    }
}

impl Bind<Dmabuf> for SkiaRenderer {
    fn bind(&mut self, dmabuf: Dmabuf) -> Result<(), <Self as Renderer>::Error> {
        let target = SkiaTarget::Dmabuf(dmabuf.clone());
        self.current_target = Some(target);
        let egl_display = self.egl_context().display().clone();
        #[allow(clippy::mutable_key_type)]
        let buffers = self.buffers.borrow_mut();
        buffers
            .entry(SkiaTarget::Dmabuf(dmabuf.clone()))
            .or_insert_with(|| {
                let image = egl_display.create_image_from_dmabuf(&dmabuf).unwrap();
                let mut texture = 0;
                // .map_err(GlesError::BindBufferEGLError)?;
                let size = dmabuf.size();
                let _width = size.w;
                let _height = size.h;

                unsafe {
                    self.gl.GenTextures(1, &mut texture);
                    self.gl.BindTexture(ffi::TEXTURE_2D, texture);
                    self.gl.EGLImageTargetTexture2DOES(ffi::TEXTURE_2D, image);

                    let mut rbo = 0;
                    self.gl.GenRenderbuffers(1, &mut rbo as *mut _);
                    self.gl.BindRenderbuffer(ffi::RENDERBUFFER, rbo);
                    // self.gl.RenderbufferStorageMultisample(ffi::RENDERBUFFER, 2, ffi::RGBA8, width, height);
                    self.gl
                        .EGLImageTargetRenderbufferStorageOES(ffi::RENDERBUFFER, image);
                    self.gl.BindRenderbuffer(ffi::RENDERBUFFER, 0);

                    let mut fbo = 0;
                    self.gl.GenFramebuffers(1, &mut fbo as *mut _);
                    self.gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo);

                    self.gl.FramebufferRenderbuffer(
                        ffi::FRAMEBUFFER,
                        ffi::COLOR_ATTACHMENT0,
                        ffi::RENDERBUFFER,
                        rbo,
                    );

                    let status = self.gl.CheckFramebufferStatus(ffi::FRAMEBUFFER);
                    self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);

                    if status != ffi::FRAMEBUFFER_COMPLETE {
                        //TODO wrap image and drop here
                        println!("framebuffer incomplete");
                        // return Err(GlesError::FramebufferBindingError);
                    }
                    SkiaGLesFbo {
                        fbo,
                        tex_id: texture,
                        format: Fourcc::Abgr8888,
                        origin: skia::gpu::SurfaceOrigin::TopLeft,
                    }
                }
            });
        Ok(())
    }
}

impl Unbind for SkiaRenderer {
    fn unbind(&mut self) -> Result<(), <Self as Renderer>::Error> {
        self.current_target = None;
        self.egl_context().unbind()?;
        Ok(())
    }
}

impl Offscreen<SkiaGLesFbo> for SkiaRenderer {
    #[profiling::function]
    fn create_buffer(
        &mut self,
        format: Fourcc,
        size: Size<i32, Buffer>,
    ) -> Result<SkiaGLesFbo, GlesError> {
        let lfbo = self.create_texture_and_framebuffer(size.w, size.h, format);
        Ok(lfbo)
    }
}

impl<'a> AsRef<SkiaFrame<'a>> for SkiaFrame<'a> {
    fn as_ref(&self) -> &SkiaFrame<'a> {
        self
    }
}

impl<'a> AsMut<SkiaFrame<'a>> for SkiaFrame<'a> {
    fn as_mut(&mut self) -> &mut SkiaFrame<'a> {
        self
    }
}

impl Blit<Dmabuf> for SkiaRenderer {
    #[profiling::function]
    fn blit_to(
        &mut self,
        to: Dmabuf,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
        _filter: TextureFilter,
    ) -> Result<(), <Self as Renderer>::Error> {
        // Get source FBO from current render target
        let src_fbo = self.get_current_fbo()?.fbo;

        // Bind destination dmabuf to get its FBO
        self.bind(to.clone())?;
        let dst_fbo = self
            .buffers
            .get(&SkiaTarget::Dmabuf(to))
            .ok_or(GlesError::FramebufferBindingError)?
            .fbo;

        // Direct FBO-to-FBO blit (GPU only)
        unsafe {
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, src_fbo);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, dst_fbo);

            self.gl.BlitFramebuffer(
                src.loc.x,
                src.loc.y,
                src.loc.x + src.size.w,
                src.loc.y + src.size.h,
                dst.loc.x,
                dst.loc.y,
                dst.loc.x + dst.size.w,
                dst.loc.y + dst.size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );

            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, 0);
        }

        Ok(())
    }

    #[profiling::function]
    fn blit_from(
        &mut self,
        from: Dmabuf,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
        _filter: TextureFilter,
    ) -> Result<(), <Self as Renderer>::Error> {
        // Get destination FBO from current render target
        let dst_fbo = self.get_current_fbo()?.fbo;

        // Bind source dmabuf to get its FBO
        self.bind(from.clone())?;
        let src_fbo = self
            .buffers
            .get(&SkiaTarget::Dmabuf(from))
            .ok_or(GlesError::FramebufferBindingError)?
            .fbo;

        // Direct FBO-to-FBO blit (GPU only)
        unsafe {
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, src_fbo);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, dst_fbo);

            self.gl.BlitFramebuffer(
                src.loc.x,
                src.loc.y,
                src.loc.x + src.size.w,
                src.loc.y + src.size.h,
                dst.loc.x,
                dst.loc.y,
                dst.loc.x + dst.size.w,
                dst.loc.y + dst.size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );

            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, 0);
        }

        Ok(())
    }
}
