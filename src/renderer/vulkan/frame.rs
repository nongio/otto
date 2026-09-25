//! A frame of the Vulkan renderer.

// Rust guideline compliant 2026-02-21

use smithay::{
    backend::renderer::{sync::SyncPoint, Color32F, ContextId, Frame, Renderer},
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

use super::{SkiaVkError, SkiaVkRenderer, SkiaVkTarget, SkiaVkTexture};
use crate::renderer::{draw, SkiaSurface};

/// An in-progress frame on one render target.
///
/// Elements draw through [`Self::skia_surface`], the same field the GL
/// frame has, so element code serves both renderers.
pub struct SkiaVkFrame<'frame> {
    pub(crate) size: Size<i32, Physical>,
    /// The surface this frame draws into.
    pub skia_surface: SkiaSurface,
    pub(crate) target: SkiaVkTarget,
    pub(crate) renderer: &'frame mut SkiaVkRenderer,
}

impl Frame for SkiaVkFrame<'_> {
    type Error = SkiaVkError;
    type TextureId = SkiaVkTexture;

    fn context_id(&self) -> ContextId<Self::TextureId> {
        self.renderer.context_id()
    }

    fn clear(
        &mut self,
        color: Color32F,
        at: &[Rectangle<i32, Physical>],
    ) -> Result<(), Self::Error> {
        self.draw_solid(Rectangle::new((0, 0).into(), self.size), at, color)
    }

    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), Self::Error> {
        draw::draw_solid(&mut self.skia_surface.surface, dst, damage, color);
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
        draw::render_texture(
            &mut self.skia_surface.surface,
            &texture.image,
            src,
            dst,
            damage,
            src_transform,
            alpha,
        );
        Ok(())
    }

    fn transformation(&self) -> Transform {
        Transform::Normal
    }

    fn output_size(&self) -> Size<i32, Physical> {
        self.size
    }

    fn wait(&mut self, sync: &SyncPoint) -> Result<(), Self::Error> {
        self.renderer.wait_sync(sync)
    }

    #[profiling::function]
    fn finish(self) -> Result<SyncPoint, Self::Error> {
        let flush_t = std::time::Instant::now();
        let sync = self.renderer.submit_target(&self.target);
        crate::render_phase_stats::record_skia_flush(flush_t.elapsed());
        sync
    }
}
