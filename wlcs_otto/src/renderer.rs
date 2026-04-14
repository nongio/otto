use std::cell::Cell;

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Fourcc},
        renderer::{
            sync::SyncPoint, ContextId, DebugFlags, Frame, ImportDma, ImportDmaWl, ImportEgl,
            ImportMem, ImportMemWl, Renderer, RendererSuper, Texture, TextureFilter,
        },
        SwapBuffersError,
    },
    reexports::wayland_server::protocol::wl_buffer,
    utils::{Buffer, Physical, Rectangle, Size, Transform},
    wayland::compositor::SurfaceData,
};

#[derive(Debug)]
pub struct DummyRenderer {}

impl DummyRenderer {
    pub fn new() -> DummyRenderer {
        DummyRenderer {}
    }
}

#[derive(Debug, Clone)]
pub struct DummyFramebuffer {}

impl Texture for DummyFramebuffer {
    fn width(&self) -> u32 {
        800
    }
    fn height(&self) -> u32 {
        600
    }
    fn format(&self) -> Option<Fourcc> {
        None
    }
}

impl RendererSuper for DummyRenderer {
    type Error = SwapBuffersError;
    type TextureId = DummyTexture;
    type Framebuffer<'buffer> = DummyFramebuffer;
    type Frame<'frame, 'buffer>
        = DummyFrame
    where
        'buffer: 'frame,
        Self: 'frame;
}

impl Renderer for DummyRenderer {
    fn context_id(&self) -> ContextId<Self::TextureId> {
        ContextId::new()
    }

    fn render<'frame, 'buffer>(
        &'frame mut self,
        _framebuffer: &'frame mut Self::Framebuffer<'buffer>,
        _output_size: Size<i32, Physical>,
        _dst_transform: Transform,
    ) -> Result<Self::Frame<'frame, 'buffer>, Self::Error>
    where
        'buffer: 'frame,
    {
        Ok(DummyFrame {})
    }

    fn upscale_filter(&mut self, _filter: TextureFilter) -> Result<(), Self::Error> {
        Ok(())
    }
    fn downscale_filter(&mut self, _filter: TextureFilter) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_debug_flags(&mut self, _flags: DebugFlags) {}
    fn debug_flags(&self) -> DebugFlags {
        DebugFlags::empty()
    }
    fn wait(&mut self, _sync: &SyncPoint) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ImportMem for DummyRenderer {
    fn import_memory(
        &mut self,
        _data: &[u8],
        _format: Fourcc,
        _size: Size<i32, Buffer>,
        _flipped: bool,
    ) -> Result<Self::TextureId, Self::Error> {
        unimplemented!()
    }

    fn update_memory(
        &mut self,
        _texture: &Self::TextureId,
        _data: &[u8],
        _region: Rectangle<i32, Buffer>,
    ) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        Box::new([Fourcc::Argb8888, Fourcc::Xrgb8888].iter().copied())
    }
}

impl ImportMemWl for DummyRenderer {
    fn import_shm_buffer(
        &mut self,
        buffer: &wl_buffer::WlBuffer,
        surface: Option<&SurfaceData>,
        _damage: &[Rectangle<i32, Buffer>],
    ) -> Result<Self::TextureId, Self::Error> {
        use smithay::wayland::shm::with_buffer_contents;
        use std::ptr;
        let ret = with_buffer_contents(buffer, |ptr, len, data| {
            let offset = data.offset as u32;
            let width = data.width as u32;
            let height = data.height as u32;
            let stride = data.stride as u32;

            let mut x = 0;
            for h in 0..height {
                for w in 0..width {
                    let idx = (offset + w + h * stride) as usize;
                    assert!(idx < len);
                    x |= unsafe { ptr::read(ptr.offset(idx as isize)) };
                }
            }

            if let Some(data) = surface {
                data.data_map.insert_if_missing(|| Cell::new(0u8));
                data.data_map.get::<Cell<u8>>().unwrap().set(x);
            }

            (width, height)
        });

        match ret {
            Ok((width, height)) => Ok(DummyTexture { width, height }),
            Err(e) => Err(SwapBuffersError::TemporaryFailure(Box::new(e))),
        }
    }
}

impl ImportDma for DummyRenderer {
    fn import_dmabuf(
        &mut self,
        _dmabuf: &Dmabuf,
        _damage: Option<&[Rectangle<i32, Buffer>]>,
    ) -> Result<Self::TextureId, Self::Error> {
        unimplemented!()
    }
}

impl ImportEgl for DummyRenderer {
    fn bind_wl_display(
        &mut self,
        _display: &smithay::reexports::wayland_server::DisplayHandle,
    ) -> Result<(), smithay::backend::egl::Error> {
        unimplemented!()
    }

    fn unbind_wl_display(&mut self) {
        unimplemented!()
    }

    fn egl_reader(&self) -> Option<&smithay::backend::egl::display::EGLBufferReader> {
        unimplemented!()
    }

    fn import_egl_buffer(
        &mut self,
        _buffer: &wl_buffer::WlBuffer,
        _surface: Option<&smithay::wayland::compositor::SurfaceData>,
        _damage: &[Rectangle<i32, Buffer>],
    ) -> Result<Self::TextureId, Self::Error> {
        unimplemented!()
    }
}

impl ImportDmaWl for DummyRenderer {}

pub struct DummyFrame {}

impl Frame for DummyFrame {
    type Error = SwapBuffersError;
    type TextureId = DummyTexture;

    fn context_id(&self) -> ContextId<Self::TextureId> {
        ContextId::new()
    }

    fn clear(
        &mut self,
        _color: smithay::backend::renderer::Color32F,
        _at: &[Rectangle<i32, Physical>],
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn draw_solid(
        &mut self,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _color: smithay::backend::renderer::Color32F,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn render_texture_at(
        &mut self,
        _texture: &Self::TextureId,
        _pos: smithay::utils::Point<i32, Physical>,
        _texture_scale: i32,
        _output_scale: impl Into<smithay::utils::Scale<f64>>,
        _src_transform: Transform,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _alpha: f32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn render_texture_from_to(
        &mut self,
        _texture: &Self::TextureId,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _src_transform: Transform,
        _alpha: f32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn transformation(&self) -> Transform {
        Transform::Normal
    }

    fn finish(self) -> Result<SyncPoint, Self::Error> {
        Ok(SyncPoint::default())
    }

    fn wait(&mut self, _sync: &SyncPoint) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DummyTexture {
    width: u32,
    height: u32,
}

impl Texture for DummyTexture {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn format(&self) -> Option<Fourcc> {
        None
    }
}
