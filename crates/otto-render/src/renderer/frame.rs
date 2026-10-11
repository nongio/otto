//! Frame rendering implementation for SkiaFrame.
//!
//! This module contains all the trait implementations for SkiaFrame,
//! handling frame rendering, texture drawing, and buffer blitting operations.

use layers::skia;
use smithay::{
    backend::renderer::{gles::GlesError, sync::SyncPoint, Color32F, ContextId, Frame, Renderer},
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

use super::{SkiaFrame, SkiaSync, SkiaTexture};

impl Frame for SkiaFrame<'_> {
    type Error = GlesError;
    type TextureId = SkiaTexture;

    fn context_id(&self) -> ContextId<Self::TextureId> {
        self.renderer.context_id()
    }
    fn clear(
        &mut self,
        color: Color32F,
        at: &[Rectangle<i32, Physical>],
    ) -> Result<(), Self::Error> {
        self.draw_solid(Rectangle::new((0, 0).into(), self.size), at, color)?;
        Ok(())
    }
    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), Self::Error> {
        super::draw::draw_solid(&mut self.skia_surface.surface, dst, damage, color);
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
        super::draw::render_texture(
            &mut self.skia_surface.surface,
            &texture.image,
            src,
            dst,
            damage,
            src_transform,
            alpha,
            texture.padding_alpha,
        );
        Ok(())
    }
    fn transformation(&self) -> Transform {
        // self.frame.transformation()
        Transform::Normal
    }

    fn output_size(&self) -> Size<i32, Physical> {
        self.size
    }
    #[profiling::function]
    fn finish(self) -> Result<SyncPoint, Self::Error> {
        let mut surface = self.skia_surface;

        // IMPORTANT: Use the *surface-specific* flush, not a bare context flush.
        //
        // Skia defers draw calls and records them against the target surface.
        // `gr_context.flush_and_submit()` flushes the context globally, but does
        // NOT guarantee that the pending ops recorded against *this* surface have
        // been resolved to GL commands on its FBO.
        //
        // `flush_and_submit_surface()` explicitly resolves the given surface's
        // recorded ops first, then submits the resulting GL commands.  Without
        // this, the EGL fence we create below may be inserted *before* Skia has
        // actually emitted the GL draw calls for this frame's content — causing
        // the GPU to scan out a partially/un-rendered buffer (black frame).
        let flush_t = std::time::Instant::now();
        surface
            .gr_context
            .flush_and_submit_surface(&mut surface.surface, None);
        crate::render_phase_stats::record_skia_flush(flush_t.elapsed());

        // Diagnostic (`touch /tmp/otto-probe-black`): sample five patches of
        // the finished frame and log their brightness, to catch a frame
        // that reached the screen black.
        if crate::debug_hooks::toggle("/tmp/otto-probe-black") {
            let w = surface.surface.width();
            let h = surface.surface.height();
            let info = skia::ImageInfo::new(
                (8, 8),
                skia::ColorType::RGBA8888,
                skia::AlphaType::Premul,
                None,
            );
            let mut buf = vec![0u8; 8 * 8 * 4];
            let points = [
                (w / 2, h / 2),
                (w / 8, h / 8),
                (w * 7 / 8, h / 8),
                (w / 8, h * 7 / 8),
                (w * 7 / 8, h * 7 / 8),
            ];
            let mut means = Vec::with_capacity(5);
            for (x, y) in points {
                let ok = surface
                    .surface
                    .read_pixels(&info, &mut buf, 8 * 4, (x.max(0), y.max(0)));
                let sum: u32 = buf
                    .chunks(4)
                    .map(|p| (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3)
                    .sum();
                means.push(if ok { (sum / 64) as i32 } else { -1 });
            }
            let target = match self.renderer.current_target.as_ref() {
                Some(crate::skia_renderer::SkiaTarget::EGLSurface(_)) => "egl",
                Some(crate::skia_renderer::SkiaTarget::Texture(_)) => "texture",
                Some(crate::skia_renderer::SkiaTarget::Renderbuffer(_)) => "renderbuffer",
                Some(crate::skia_renderer::SkiaTarget::Dmabuf(_)) => "dmabuf",
                Some(crate::skia_renderer::SkiaTarget::Fbo(_)) => "fbo",
                None => "none",
            };
            tracing::info!(target: "otto::probe", "frame {target} {w}x{h} brightness {means:?}");
        }

        let sync = SkiaSync::create(self.renderer.egl_context().display())
            .map(SyncPoint::from)
            .unwrap_or_else(|err| {
                tracing::warn!(?err, "Failed to create EGL fence, falling back to signaled");
                SyncPoint::signaled()
            });

        // Submit the fence command itself.
        //
        // `EGL_ANDROID_native_fence_sync` — which Smithay's `EGLFence` picks
        // whenever the driver has it — does NOT implicitly flush: the fence is
        // appended to the command stream and only reaches the GPU on the next
        // flush. Everything drawn before it was already submitted by Skia
        // above, so without this the fence sits unsubmitted and never signals.
        //
        // A consumer that polls it (`is_reached()`) instead of blocking then
        // waits forever: that is what froze virtual outputs, whose render loop
        // only starts the next frame once the previous fence has signaled. It
        // recovered only when an unrelated render — a physical output redrawing
        // because the pointer moved — flushed the shared context for us.
        unsafe {
            self.renderer.gl.Flush();
        }

        Ok(sync)
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
