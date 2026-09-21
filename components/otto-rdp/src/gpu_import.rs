//! Reading frames the CPU cannot map.
//!
//! A tiled dmabuf has no meaningful bytes to `mmap`: its layout belongs to the
//! GPU. This imports each one into EGL on the render node, attaches it to a
//! framebuffer and reads it back with `glReadPixels`, which untiles it. It is
//! what lets the bridge take frames from a driver that cannot render into
//! LINEAR at all (NVIDIA), and so has nothing else to offer.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::os::fd::AsRawFd;

use anyhow::{anyhow, Context as _};
use khronos_egl as egl;

/// `DRM_FORMAT_ARGB8888`: PipeWire's `BGRA`.
pub const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241;
/// `DRM_FORMAT_XRGB8888`: PipeWire's `BGRx`.
pub const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;

const PLATFORM_GBM_KHR: egl::Enum = 0x31D7;
const LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
const LINUX_DRM_FOURCC_EXT: egl::Attrib = 0x3271;
const DMA_BUF_PLANE0_FD_EXT: egl::Attrib = 0x3272;
const DMA_BUF_PLANE0_OFFSET_EXT: egl::Attrib = 0x3273;
const DMA_BUF_PLANE0_PITCH_EXT: egl::Attrib = 0x3274;
const DMA_BUF_PLANE0_MODIFIER_LO_EXT: egl::Attrib = 0x3443;
const DMA_BUF_PLANE0_MODIFIER_HI_EXT: egl::Attrib = 0x3444;
const GL_BGRA_EXT: gl::types::GLenum = 0x80E1;

#[link(name = "gbm")]
extern "C" {
    fn gbm_create_device(fd: i32) -> *mut c_void;
    fn gbm_device_destroy(device: *mut c_void);
}

type ImageTargetTexture2D = unsafe extern "system" fn(gl::types::GLenum, *const c_void);
type QueryDmaBufModifiers =
    unsafe extern "system" fn(egl::EGLDisplay, i32, i32, *mut u64, *mut u32, *mut i32) -> u32;

/// One buffer of the stream's pool, imported and ready to read.
struct Imported {
    image: egl::Image,
    texture: gl::types::GLuint,
    fbo: gl::types::GLuint,
}

/// A surfaceless GLES context on the render node, current on the thread that
/// created it. PipeWire calls back on that same thread, so it never moves.
pub struct GpuReader {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    gbm: *mut c_void,
    image_target: ImageTargetTexture2D,
    query_modifiers: QueryDmaBufModifiers,
    /// Keyed by the buffer's fd, which PipeWire keeps for the pool's life.
    imported: HashMap<i64, Imported>,
    /// Buffers of this pool that could not be imported, so they are not
    /// retried, and warned about, on every frame.
    refused: HashSet<i64>,
    /// The render node the GBM device was made on, closed after it.
    _node: Option<std::fs::File>,
}

impl GpuReader {
    /// Open `OTTO_RDP_RENDER_NODE`, or the first render node, and make a
    /// context current on this thread.
    pub fn new() -> anyhow::Result<Self> {
        let path = match std::env::var_os("OTTO_RDP_RENDER_NODE") {
            Some(path) => path.into(),
            None => first_render_node()?,
        };
        let node = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let gbm = unsafe { gbm_create_device(node.as_raw_fd()) };
        if gbm.is_null() {
            anyhow::bail!("gbm_create_device failed on {}", path.display());
        }
        let reader = Self::on_device(gbm);
        if reader.is_err() {
            unsafe { gbm_device_destroy(gbm) };
        }
        let mut reader = reader?;
        reader._node = Some(node);
        tracing::info!("GPU frame import on {}", path.display());
        Ok(reader)
    }

    /// Everything above the GBM device, which the caller destroys when this
    /// fails.
    fn on_device(gbm: *mut c_void) -> anyhow::Result<Self> {
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|e| anyhow!("loading libEGL: {e}"))?;
        let display =
            unsafe { egl.get_platform_display(PLATFORM_GBM_KHR, gbm, &[egl::ATTRIB_NONE]) }?;
        egl.initialize(display)?;
        let (context, image_target, query_modifiers) = match Self::context_on(&egl, display) {
            Ok(context) => context,
            Err(e) => {
                // Terminating releases whatever was made on the display.
                let _ = egl.make_current(display, None, None, None);
                let _ = egl.terminate(display);
                return Err(e);
            }
        };
        Ok(Self {
            image_target,
            query_modifiers,
            egl,
            display,
            context,
            gbm,
            imported: HashMap::new(),
            refused: HashSet::new(),
            _node: None,
        })
    }

    /// Make a GLES context current on `display` and look up the entry points
    /// the reader needs.
    fn context_on(
        egl: &egl::DynamicInstance<egl::EGL1_5>,
        display: egl::Display,
    ) -> anyhow::Result<(egl::Context, ImageTargetTexture2D, QueryDmaBufModifiers)> {
        egl.bind_api(egl::OPENGL_ES_API)?;
        let config = egl
            .choose_first_config(
                display,
                &[egl::RENDERABLE_TYPE, egl::OPENGL_ES2_BIT, egl::NONE],
            )?
            .ok_or_else(|| anyhow!("no GLES config"))?;
        let context = egl.create_context(
            display,
            config,
            None,
            &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
        )?;
        egl.make_current(display, None, None, Some(context))?;

        gl::load_with(|name| {
            egl.get_proc_address(name)
                .map_or(std::ptr::null(), |f| f as *const c_void)
        });
        let image_target = egl
            .get_proc_address("glEGLImageTargetTexture2DOES")
            .ok_or_else(|| anyhow!("no glEGLImageTargetTexture2DOES"))?;
        let query_modifiers = egl
            .get_proc_address("eglQueryDmaBufModifiersEXT")
            .ok_or_else(|| anyhow!("no eglQueryDmaBufModifiersEXT"))?;

        // SAFETY: both were looked up by name for exactly these signatures.
        Ok((
            context,
            unsafe {
                std::mem::transmute::<extern "system" fn(), ImageTargetTexture2D>(image_target)
            },
            unsafe {
                std::mem::transmute::<extern "system" fn(), QueryDmaBufModifiers>(query_modifiers)
            },
        ))
    }

    /// The modifiers of `fourcc` this GPU can import into a texture it can
    /// read back. External-only ones can only be sampled, so they are left out.
    pub fn modifiers(&self, fourcc: u32) -> Vec<u64> {
        let display = self.display.as_ptr();
        let mut count = 0;
        unsafe {
            (self.query_modifiers)(
                display,
                fourcc as i32,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut count,
            );
        }
        let mut modifiers = vec![0u64; count.max(0) as usize];
        let mut external_only = vec![0u32; modifiers.len()];
        unsafe {
            (self.query_modifiers)(
                display,
                fourcc as i32,
                count,
                modifiers.as_mut_ptr(),
                external_only.as_mut_ptr(),
                &mut count,
            );
        }
        modifiers
            .into_iter()
            .zip(external_only)
            .filter(|&(_, external)| external == 0)
            .map(|(modifier, _)| modifier)
            .collect()
    }

    /// Read a `width`×`height` frame out of the dmabuf `fd` as tightly packed
    /// BGRA, the byte order of the stream's own `BGRA`/`BGRx`.
    #[allow(clippy::too_many_arguments)]
    pub fn read(
        &mut self,
        fd: i64,
        offset: u32,
        stride: u32,
        modifier: u64,
        fourcc: u32,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        if self.refused.contains(&fd) {
            return None;
        }
        if !self.imported.contains_key(&fd) {
            let Some(imported) = self.import(fd, offset, stride, modifier, fourcc, width, height)
            else {
                self.refused.insert(fd);
                return None;
            };
            self.imported.insert(fd, imported);
        }
        let fbo = self.imported[&fd].fbo;

        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
            gl::PixelStorei(gl::PACK_ALIGNMENT, 4);
            gl::ReadPixels(
                0,
                0,
                width as i32,
                height as i32,
                GL_BGRA_EXT,
                gl::UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
            if gl::GetError() != gl::NO_ERROR {
                // No EXT_read_format_bgra: read RGBA and swap.
                gl::ReadPixels(
                    0,
                    0,
                    width as i32,
                    height as i32,
                    gl::RGBA,
                    gl::UNSIGNED_BYTE,
                    pixels.as_mut_ptr().cast(),
                );
                pixels.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
            }
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        }
        Some(pixels)
    }

    /// Drop every import, for a pool that is about to be replaced.
    pub fn forget_buffers(&mut self) {
        self.refused.clear();
        for (_, imported) in self.imported.drain() {
            unsafe {
                gl::DeleteFramebuffers(1, &imported.fbo);
                gl::DeleteTextures(1, &imported.texture);
            }
            let _ = self.egl.destroy_image(self.display, imported.image);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn import(
        &self,
        fd: i64,
        offset: u32,
        stride: u32,
        modifier: u64,
        fourcc: u32,
        width: u32,
        height: u32,
    ) -> Option<Imported> {
        let attributes = [
            egl::WIDTH as egl::Attrib,
            width as egl::Attrib,
            egl::HEIGHT as egl::Attrib,
            height as egl::Attrib,
            LINUX_DRM_FOURCC_EXT,
            fourcc as egl::Attrib,
            DMA_BUF_PLANE0_FD_EXT,
            fd as egl::Attrib,
            DMA_BUF_PLANE0_OFFSET_EXT,
            offset as egl::Attrib,
            DMA_BUF_PLANE0_PITCH_EXT,
            stride as egl::Attrib,
            DMA_BUF_PLANE0_MODIFIER_LO_EXT,
            (modifier & 0xffff_ffff) as egl::Attrib,
            DMA_BUF_PLANE0_MODIFIER_HI_EXT,
            (modifier >> 32) as egl::Attrib,
            egl::ATTRIB_NONE,
        ];
        let image = unsafe {
            self.egl.create_image(
                self.display,
                egl::Context::from_ptr(egl::NO_CONTEXT),
                LINUX_DMA_BUF_EXT,
                egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
                &attributes,
            )
        };
        let image = match image {
            Ok(image) => image,
            Err(e) => {
                tracing::warn!("importing dmabuf (modifier {modifier:#x}) failed: {e}");
                return None;
            }
        };

        let (mut texture, mut fbo) = (0, 0);
        let complete = unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            (self.image_target)(gl::TEXTURE_2D, image.as_ptr());
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::GenFramebuffers(1, &mut fbo);
            gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture,
                0,
            );
            let status = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            status == gl::FRAMEBUFFER_COMPLETE
        };
        if !complete {
            tracing::warn!("dmabuf (modifier {modifier:#x}) cannot be read back");
            unsafe {
                gl::DeleteFramebuffers(1, &fbo);
                gl::DeleteTextures(1, &texture);
            }
            let _ = self.egl.destroy_image(self.display, image);
            return None;
        }
        Some(Imported {
            image,
            texture,
            fbo,
        })
    }
}

impl Drop for GpuReader {
    fn drop(&mut self) {
        self.forget_buffers();
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
        unsafe { gbm_device_destroy(self.gbm) };
    }
}

fn first_render_node() -> anyhow::Result<std::path::PathBuf> {
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("renderD"))
        })
        .collect();
    nodes.sort();
    nodes
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no render node in /dev/dri"))
}
