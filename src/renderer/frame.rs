//! Frame rendering implementation for SkiaFrame.
//!
//! This module contains all the trait implementations for SkiaFrame,
//! handling frame rendering, texture drawing, and buffer blitting operations.

use layers::{sb, skia};
use smithay::{
    backend::renderer::{gles::GlesError, sync::SyncPoint, Color32F, ContextId, Frame, Renderer},
    utils::{Buffer, Physical, Rectangle, Transform},
};
use std::sync::atomic::Ordering;

use super::{finished_proc, FlushInfo2, SkiaFrame, SkiaSync, SkiaTexture, FINISHED_PROC_STATE};

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
                .constrain(Rectangle::from_extremities((0, 0), dst.size.to_point()));
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
                .constrain(Rectangle::from_extremities((0, 0), dst.size.to_point()));
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
