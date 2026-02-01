use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefCell,
    collections::HashMap,
    ffi::{c_char, CStr},
    rc::Rc,
};

use layers::skia;

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
            Bind, Blit, ContextId, DebugFlags, ExportMem, ImportDma, ImportDmaWl, ImportEgl,
            ImportMem, ImportMemWl, Offscreen, Renderer, RendererSuper, Texture, TextureFilter,
        },
    },
    reexports::wayland_server::{protocol::wl_buffer::WlBuffer, DisplayHandle},
    utils::{Buffer, Physical, Rectangle, Size, Transform},
    wayland::compositor::SurfaceData,
};

// Import and re-export types from renderer module
use crate::renderer::EGLSurfaceWrapper;

// Re-export public types from renderer module
pub use crate::renderer::{
    SkiaFrame, SkiaGLesFbo, SkiaSurface, SkiaTexture, SkiaTextureImage, SkiaTextureMapping,
};

pub struct SkiaRenderer {
    gl_renderer: GlesRenderer,
    pub(crate) gl: ffi::Gles2,

    target_renderer: HashMap<SkiaTarget, SkiaSurface>,
    current_target: Option<SkiaTarget>,
    pub buffers: HashMap<SkiaTarget, SkiaGLesFbo>,
    pub context: Option<skia::gpu::DirectContext>,

    dmabuf_cache: std::collections::HashMap<WeakDmabuf, SkiaTexture>,
    smithay_context_id: ContextId<SkiaTexture>,
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
            smithay_context_id: ContextId::new(),
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
            smithay_context_id: ContextId::new(),
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
            width,
            height,
        }
    }

    fn import_dmabuf_internal(
        &mut self,
        dmabuf: &Dmabuf,
        damage: Option<&[Rectangle<i32, Buffer>]>,
    ) -> Result<<Self as RendererSuper>::TextureId, <Self as RendererSuper>::Error> {
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

impl RendererSuper for SkiaRenderer {
    type Error = GlesError;
    type TextureId = SkiaTexture;
    type Framebuffer<'buffer> = SkiaGLesFbo;
    type Frame<'frame, 'buffer>
        = SkiaFrame<'frame>
    where
        'buffer: 'frame;
}

impl Renderer for SkiaRenderer {
    fn context_id(&self) -> ContextId<Self::TextureId> {
        self.smithay_context_id.clone()
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
    fn render<'frame, 'buffer>(
        &'frame mut self,
        _framebuffer: &'frame mut Self::Framebuffer<'buffer>,
        output_size: Size<i32, Physical>,
        _dst_transform: Transform,
    ) -> Result<Self::Frame<'frame, 'buffer>, Self::Error>
    where
        'buffer: 'frame,
    {
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
            // id,
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

        // Check if we have a current renderer before borrowing context
        let has_current_renderer = self.current_skia_renderer().is_some();
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
            let skia_format = skia_format?;
            let skia_color = skia_color?;

            let texture_info = skia::gpu::gl::TextureInfo {
                target,
                id: texture.tex_id(),
                format: skia_format.into(),
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
                skia_color,
                skia::AlphaType::Premul,
                None,
            )?;

            // Flush the image if we have a current rendering surface
            if has_current_renderer {
                context.flush_and_submit_image(&image);
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
    ) -> Result<<Self as RendererSuper>::TextureId, <Self as RendererSuper>::Error> {
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
    ) -> Result<<Self as RendererSuper>::TextureId, <Self as RendererSuper>::Error> {
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
    ) -> Result<<Self as RendererSuper>::TextureId, <Self as RendererSuper>::Error> {
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
    ) -> Result<<Self as RendererSuper>::TextureId, <Self as RendererSuper>::Error> {
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
    ) -> Result<<Self as RendererSuper>::TextureId, <Self as RendererSuper>::Error> {
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
        texture: &<Self as RendererSuper>::TextureId,
        data: &[u8],
        region: Rectangle<i32, Buffer>,
    ) -> Result<(), <Self as RendererSuper>::Error> {
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
        _target: &Self::Framebuffer<'_>,
        region: Rectangle<i32, Buffer>,
        fourcc_format: Fourcc,
    ) -> Result<Self::TextureMapping, Self::Error> {
        // Just store the FBO info - don't create image or read pixels yet
        let fbo_info = self.get_current_fbo()?.clone();

        Ok(Self::TextureMapping {
            fourcc_format,
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
    ) -> Result<&'a [u8], <Self as RendererSuper>::Error> {
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

impl Bind<EGLSurface> for SkiaRenderer {
    fn bind(
        &mut self,
        surface: &mut EGLSurface,
    ) -> Result<SkiaGLesFbo, <Self as RendererSuper>::Error> {
        // Make the surface current first
        unsafe {
            self.egl_context().make_current_with_surface(surface)?;
            self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
        }

        // Delegate to GlesRenderer to get framebuffer info
        let gles_framebuffer = self.gl_renderer.bind(surface)?;

        // Create the framebuffer descriptor
        let sfbo = SkiaGLesFbo {
            fbo: 0,
            tex_id: 0,
            format: gles_framebuffer.format().unwrap_or(Fourcc::Argb8888),
            origin: skia::gpu::SurfaceOrigin::BottomLeft,
            width: gles_framebuffer.width() as i32,
            height: gles_framebuffer.height() as i32,
        };

        // Set this as the current render target using the FBO as the key
        let render_target = SkiaTarget::Fbo(sfbo.clone());
        self.current_target = Some(render_target.clone());
        self.buffers.insert(render_target, sfbo.clone());

        Ok(sfbo)
    }
}

impl Bind<Rc<EGLSurface>> for SkiaRenderer {
    fn bind(
        &mut self,
        surface: &mut Rc<EGLSurface>,
    ) -> Result<SkiaGLesFbo, <Self as RendererSuper>::Error> {
        unsafe {
            self.egl_context().make_current_with_surface(surface)?;
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
                width: 0,
                height: 0,
            };
            self.buffers.insert(render_target.clone(), sfbo.clone());
        }
        self.current_target = Some(render_target.clone());

        Ok(self.buffers.get(&render_target).unwrap().clone())
    }
}

impl Bind<SkiaGLesFbo> for SkiaRenderer {
    fn bind(
        &mut self,
        texture: &mut SkiaGLesFbo,
    ) -> Result<SkiaGLesFbo, <Self as RendererSuper>::Error> {
        self.current_target = Some(SkiaTarget::Fbo(texture.clone()));
        self.buffers
            .insert(SkiaTarget::Fbo(texture.clone()), texture.clone());

        Ok(texture.clone())
    }
}

impl Bind<GlesTexture> for SkiaRenderer {
    fn bind(
        &mut self,
        texture: &mut GlesTexture,
    ) -> Result<SkiaGLesFbo, <Self as RendererSuper>::Error> {
        self.current_target = Some(SkiaTarget::Texture(texture.tex_id()));
        // let res = self.gl_renderer.bind(texture);
        unimplemented!("bind GlesTexture")

        // Ok(())
    }
}

impl Bind<GlesRenderbuffer> for SkiaRenderer {
    fn bind(
        &mut self,
        target: &mut GlesRenderbuffer,
    ) -> Result<SkiaGLesFbo, <Self as RendererSuper>::Error> {
        self.current_target = Some(SkiaTarget::Renderbuffer(target));
        // let res = self.gl_renderer.bind(target);
        unimplemented!("bind GlesRenderbuffer")
    }
}

impl Bind<Dmabuf> for SkiaRenderer {
    fn bind(&mut self, dmabuf: &mut Dmabuf) -> Result<SkiaGLesFbo, <Self as RendererSuper>::Error> {
        let target = SkiaTarget::Dmabuf(dmabuf.clone());
        self.current_target = Some(target.clone());
        let egl_display = self.egl_context().display().clone();
        #[allow(clippy::mutable_key_type)]
        let buffers = self.buffers.borrow_mut();
        let fbo = buffers
            .entry(SkiaTarget::Dmabuf(dmabuf.clone()))
            .or_insert_with(|| {
                let image = egl_display.create_image_from_dmabuf(dmabuf).unwrap();
                let mut texture = 0;
                // .map_err(GlesError::BindBufferEGLError)?;
                let size = dmabuf.size();
                let width = size.w;
                let height = size.h;

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

                    // Add a depth buffer to make the framebuffer complete
                    // (required for rendering, even if depth testing is disabled)
                    let mut depth_rbo = 0;
                    self.gl.GenRenderbuffers(1, &mut depth_rbo as *mut _);
                    self.gl.BindRenderbuffer(ffi::RENDERBUFFER, depth_rbo);
                    self.gl.RenderbufferStorage(
                        ffi::RENDERBUFFER,
                        ffi::DEPTH_COMPONENT16,
                        width,
                        height,
                    );
                    self.gl.FramebufferRenderbuffer(
                        ffi::FRAMEBUFFER,
                        ffi::DEPTH_ATTACHMENT,
                        ffi::RENDERBUFFER,
                        depth_rbo,
                    );
                    self.gl.BindRenderbuffer(ffi::RENDERBUFFER, 0);

                    let status = self.gl.CheckFramebufferStatus(ffi::FRAMEBUFFER);

                    if status != ffi::FRAMEBUFFER_COMPLETE {
                        panic!("Framebuffer incomplete for dmabuf: status 0x{:X}", status);
                    }

                    self.gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
                    SkiaGLesFbo {
                        fbo,
                        tex_id: texture,
                        format: Fourcc::Abgr8888,
                        origin: skia::gpu::SurfaceOrigin::TopLeft,
                        width,
                        height,
                    }
                }
            })
            .clone();
        Ok(fbo)
    }
}

impl Blit for SkiaRenderer {
    #[profiling::function]
    fn blit(
        &mut self,
        from: &SkiaGLesFbo,
        to: &mut SkiaGLesFbo,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
        filter: TextureFilter,
    ) -> Result<SyncPoint, <Self as RendererSuper>::Error> {
        // Direct FBO-to-FBO blit using OpenGL
        unsafe {
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, from.fbo);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, to.fbo);

            let gl_filter = match filter {
                TextureFilter::Nearest => ffi::NEAREST,
                TextureFilter::Linear => ffi::LINEAR,
            };

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
                gl_filter,
            );

            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, 0);
        }

        // Create a sync point (no actual sync needed for immediate blit)
        Ok(SyncPoint::signaled())
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
