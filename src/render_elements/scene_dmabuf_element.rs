//! Dmabuf-backed scene render element.
//!
//! # Why
//!
//! The plain [`SceneElement`](crate::render_elements::scene_element::SceneElement)
//! renders the lay-rs scene into the renderer's primary framebuffer (allocator-
//! provided swapchain slot). Smithay's `DrmCompositor` then either scans out
//! that framebuffer to the primary plane or composites it together with other
//! elements. The element itself isn't a plane-eligible "buffer" — it's just a
//! drawing operation against whatever the renderer hands it.
//!
//! For overlay-plane scanout to work for a top window WHILE the scene also
//! shows around it (dock, bar, background), Otto needs the scene to be on its
//! OWN plane (typically primary), so:
//!
//! - Scene FB lives in a dmabuf-backed buffer that Smithay can hand to the
//!   primary plane directly (no per-frame composite when scene is unchanged)
//! - Window dmabuf goes on an overlay plane (still depends on Smithay's
//!   overlay-rule fix — see matrix-p writeup)
//!
//! This module provides the scene-as-dmabuf-element side of that architecture.
//!
//! # Status
//!
//! **Scaffolding only**. The type signatures are in place to lock down the
//! shape of the integration. The actual rendering path — GBM buffer allocation,
//! dmabuf import as GL texture, Skia surface creation, scene render into it —
//! is left as TODOs. Pairs with our local smithay `feat/dmabuf-scanout` branch
//! that adds `UnderlyingStorage::Dmabuf` so a render element of this kind can
//! be assigned to a KMS plane.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use layers::engine::Engine;
use smithay::{
    backend::{
        allocator::{
            dmabuf::{AsDmabuf, Dmabuf},
            gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice},
            Allocator, Buffer, Fourcc, Modifier,
        },
        drm::DrmDeviceFd,
        renderer::{
            element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
            utils::{CommitCounter, DamageBag, DamageSet},
            RendererSuper,
        },
    },
    utils::{Buffer as BufferCoord, Physical, Point, Rectangle, Scale},
};

use crate::{
    renderer::skia_surface::SkiaSurface, skia_renderer::SkiaRenderer, udev::UdevRenderer,
};

/// A scene render element whose output is rendered into a GBM/dmabuf-backed
/// buffer. The buffer is exposed to Smithay's `DrmCompositor` via
/// [`UnderlyingStorage::Dmabuf`], making the element eligible for direct KMS
/// plane assignment (primary or overlay) instead of composite-into-FB.
///
/// Lifecycle:
/// - Construct once (per output) with a target size and format
/// - On each frame: lay-rs `engine.update()` — if dirty, render into the
///   wrapped Skia surface; if clean, the previous buffer is reused
/// - Smithay reads the dmabuf via `underlying_storage()` and assigns the
///   element to a plane
///
/// Resize: not handled in this scaffold; `recreate_for_size()` will need to
/// allocate a new GBM buffer when the output dimensions change.
#[derive(Clone)]
pub struct SceneDmabufElement {
    id: Id,
    dmabuf: Arc<OnceLock<Dmabuf>>,
    inner: Arc<Mutex<Inner>>,
    /// Position of this element in physical output coordinates.
    pub position: (i32, i32),
    /// Global plane alpha (0.0–1.0). Passed to the KMS plane `alpha` property.
    pub plane_alpha: f32,
}

struct Inner {
    commit_counter: CommitCounter,
    engine: Arc<Engine>,
    /// Per-output sub-tree node. Same role as `SceneElement::output_root`.
    output_root: Option<layers::engine::NodeRef>,
    /// Physical size of the buffer.
    size: (i32, i32),
    /// Damage tracker — same shape as `SceneElement::damage` so the existing
    /// damage-tracking logic carries over.
    damage: DamageBag<i32, Physical>,
    /// The GBM buffer kept alive for the duration of `dmabuf`'s validity.
    _gbm_buffer: Option<GbmBuffer>,
    /// Skia surface wrapping the GL texture imported from `dmabuf`. Set by
    /// [`SceneDmabufElement::ensure_render_target`] which needs the renderer.
    skia_surface: Option<SkiaSurface>,
    /// Last lay-rs `engine.update()` time stamp for delta-time computation.
    last_update: Instant,
}

impl SceneDmabufElement {
    /// Construct a new dmabuf-backed scene element. Allocates the initial
    /// GBM buffer at `size` with `format`, imports it as a GL texture, and
    /// wraps that as a Skia render surface.
    ///
    /// **TODO**: implement allocation + GL import + Skia surface creation.
    /// For now this stores the inputs and leaves `target = None`.
    pub fn new(
        engine: Arc<Engine>,
        _gbm: GbmDevice<DrmDeviceFd>,
        size: (i32, i32),
        _format: Fourcc,
    ) -> Self {
        Self {
            id: Id::new(),
            dmabuf: Arc::new(OnceLock::new()),
            inner: Arc::new(Mutex::new(Inner {
                commit_counter: CommitCounter::default(),
                engine,
                output_root: None,
                size,
                damage: DamageBag::new(5),
                _gbm_buffer: None,
                skia_surface: None,
                last_update: Instant::now(),
            })),
            position: (0, 0),
            plane_alpha: 1.0,
        }
    }

    /// Set the per-output sub-tree this element renders from.
    pub fn set_output_root(&self, root: layers::engine::NodeRef) {
        self.inner.lock().unwrap().output_root = Some(root);
    }

    /// Resize the underlying GBM buffer. **TODO**: free the old buffer + GL
    /// texture + Skia surface, allocate a new one at the new size, re-bind.
    pub fn recreate_for_size(&self, _size: (i32, i32)) {
        // TODO
    }

    /// Allocate the GBM buffer + dmabuf if not yet done. Returns `Ok(())` on
    /// success or if a buffer is already present.
    ///
    /// GL texture import + Skia surface wrapping happen in
    /// [`Self::ensure_render_target`], which needs a renderer reference and
    /// is therefore called from the rendering path rather than here.
    pub fn ensure_buffer(
        &self,
        allocator: &mut GbmAllocator<DrmDeviceFd>,
        format: Fourcc,
        render_node: smithay::backend::drm::DrmNode,
    ) -> Result<(), AllocateError> {
        if self.dmabuf.get().is_some() {
            return Ok(());
        }
        let (w, h) = {
            let inner = self.inner.lock().unwrap();
            inner.size
        };
        let gbm_buffer = allocator
            .create_buffer(w as u32, h as u32, format, &[Modifier::Linear])
            .map_err(AllocateError::Gbm)?;
        let dmabuf = gbm_buffer.export().map_err(AllocateError::Export)?;
        // Tag the dmabuf with the render node so Smithay's GbmFramebufferExporter
        // accepts it (its `import_node` is the render node, while GBM export
        // defaults to the primary node).
        dmabuf.set_node(render_node);
        tracing::info!(
            target: "otto::scanout",
            "scene_dmabuf: allocated {}x{} format={:?} modifier={:?} node={:?}",
            w,
            h,
            dmabuf.format().code,
            dmabuf.format().modifier,
            dmabuf.node(),
        );
        self.inner.lock().unwrap()._gbm_buffer = Some(gbm_buffer);
        let _ = self.dmabuf.set(dmabuf);
        Ok(())
    }

    /// Returns the dmabuf if `ensure_buffer` has been called successfully.
    pub fn dmabuf(&self) -> Option<&Dmabuf> {
        self.dmabuf.get()
    }

    /// Build the GL texture + Skia surface for our dmabuf using `renderer`.
    /// Idempotent: returns immediately if the Skia surface is already set up.
    /// Must be called after [`Self::ensure_buffer`] has produced the dmabuf.
    pub fn ensure_render_target(
        &self,
        renderer: &mut SkiaRenderer,
    ) -> Result<(), smithay::backend::renderer::gles::GlesError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.skia_surface.is_some() {
            return Ok(());
        }
        let Some(dmabuf) = self.dmabuf.get() else {
            return Ok(()); // ensure_buffer not yet called; nothing to set up
        };
        let surface = renderer.create_surface_from_dmabuf(dmabuf)?;
        inner.skia_surface = Some(surface);
        Ok(())
    }

    /// Render the lay-rs scene into the dmabuf-backed Skia surface. Called
    /// each frame from the udev render path before Smithay scans the buffer
    /// out. Skips the render when lay-rs reports no damage, so the GPU does
    /// no work on idle frames.
    /// Render the current scene state into the dmabuf-backed Skia surface.
    /// Does NOT call `engine.update()` — that's done by the caller-side
    /// `scene_element.update()` earlier in the frame; we just consume the
    /// current scene state.
    pub fn update(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.last_update = Instant::now();

        let (w, h) = inner.size;

        let Some(skia_surface) = inner.skia_surface.as_mut() else {
            return false;
        };
        let canvas = skia_surface.canvas();
        let save_point = canvas.save();

        // Solid opaque fill — plane_alpha drives the blend, not pixel alpha.
        canvas.clear(layers::skia::Color4f::new(0.2, 0.6, 1.0, 1.0));
        let mut paint = layers::skia::Paint::default();
        paint.set_color(layers::skia::Color::from_rgb(255, 255, 255));
        paint.set_anti_alias(true);
        canvas.draw_circle((w as f32 / 2.0, h as f32 / 2.0), 80.0, &paint);
        canvas.restore_to_count(save_point);
        skia_surface.gr_context.flush_and_submit_surface(
            &mut skia_surface.surface,
            layers::skia::gpu::SyncCpu::Yes,
        );

        inner.commit_counter.increment();
        inner
            .damage
            .add(vec![Rectangle::new((0, 0).into(), (w, h).into())]);

        true
    }
}

/// Errors when allocating the GBM-backed render target.
#[derive(Debug, thiserror::Error)]
pub enum AllocateError {
    #[error("gbm buffer allocation failed: {0}")]
    Gbm(std::io::Error),
    #[error("dmabuf export failed: {0}")]
    Export(smithay::backend::allocator::gbm::GbmConvertError),
}

// ── Element trait ──────────────────────────────────────────────────────────
//
// The geometry/location/damage logic mirrors SceneElement so the rest of the
// render pipeline doesn't have to special-case this element.

impl Element for SceneDmabufElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        self.position.into()
    }

    fn src(&self) -> Rectangle<f64, BufferCoord> {
        let inner = self.inner.lock().unwrap();
        Rectangle::new((0, 0).into(), (inner.size.0, inner.size.1).into()).to_f64()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        let inner = self.inner.lock().unwrap();
        Rectangle::new(self.position.into(), (inner.size.0, inner.size.1).into())
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.lock().unwrap().commit_counter
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let inner = self.inner.lock().unwrap();
        let geometry_size = (inner.size.0, inner.size.1).into();
        let full = Rectangle::new((0, 0).into(), geometry_size);
        match inner.damage.damage_since(commit) {
            Some(rects) if !rects.is_empty() => DamageSet::from_slice(&rects),
            None => DamageSet::from_slice(&[full]),
            _ => DamageSet::default(),
        }
    }

    fn alpha(&self) -> f32 {
        self.plane_alpha
    }

    fn kind(&self) -> Kind {
        // Mark as scanout-eligible so Smithay's DrmCompositor will attempt
        // primary/overlay-plane assignment instead of falling through to
        // composite-into-FB.
        Kind::ScanoutCandidate
    }

    /// The whole surface is opaque: the scene render covers every pixel in
    /// our geometry (background / dock / windows). Telling Smithay this lets
    /// its overlay-plane assignment path accept us even when we overlap with
    /// the primary plane (our patched overlap rule in smithay relaxes the
    /// check for fully-opaque overlays).
    fn opaque_regions(
        &self,
        scale: Scale<f64>,
    ) -> smithay::backend::renderer::utils::OpaqueRegions<i32, Physical> {
        smithay::backend::renderer::utils::OpaqueRegions::from_slice(&[self.geometry(scale)])
    }
}

impl<'renderer> RenderElement<UdevRenderer<'renderer>> for SceneDmabufElement {
    fn draw(
        &self,
        frame: &mut <UdevRenderer<'renderer> as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <UdevRenderer<'renderer> as RendererSuper>::Error> {
        // If Smithay assigned us to a plane via underlying_storage(), draw()
        // is never called. If draw() IS called, Smithay rejected the plane
        // assignment and is asking us to composite. We log the first call so
        // the diag tells us whether scanout is succeeding.
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static LOGGED: AtomicBool = AtomicBool::new(false);
            if !LOGGED.swap(true, Ordering::Relaxed) {
                tracing::info!(
                    target: "otto::scanout",
                    "SceneDmabufElement::draw() called — Smithay fell back to composite \
                     (no plane assigned). dst={dst:?} damage_count={}",
                    damage.len()
                );
            }
        }
        let _ = (frame, src, dst, damage, opaque_regions);
        Ok(())
    }

    /// The whole point: hand Smithay the dmabuf so it can assign this
    /// element to a KMS plane directly. Returns the rendered scene's dmabuf
    /// as `UnderlyingStorage::Dmabuf` (added by our local smithay
    /// `feat/dmabuf-scanout` patch).
    fn underlying_storage(
        &self,
        _renderer: &mut UdevRenderer<'renderer>,
    ) -> Option<UnderlyingStorage<'_>> {
        self.dmabuf.get().map(UnderlyingStorage::Dmabuf)
    }
}

impl RenderElement<SkiaRenderer> for SceneDmabufElement {
    fn draw<'frame>(
        &self,
        _frame: &mut <SkiaRenderer as RendererSuper>::Frame<'frame, 'frame>,
        _src: Rectangle<f64, BufferCoord>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <SkiaRenderer as RendererSuper>::Error> {
        // TODO: same as the UdevRenderer impl — blit the dmabuf-backed
        // texture onto the SkiaRenderer's surface as the composite fallback.
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut SkiaRenderer) -> Option<UnderlyingStorage<'_>> {
        self.dmabuf.get().map(UnderlyingStorage::Dmabuf)
    }
}

#[allow(dead_code)]
fn _build_allocator(gbm: GbmDevice<DrmDeviceFd>) -> GbmAllocator<DrmDeviceFd> {
    // The allocator pattern Otto uses elsewhere (see `udev/device.rs`). The
    // SCANOUT flag is what makes the buffer plane-eligible.
    GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT)
}

// Suppress warnings for fields that aren't fully wired yet but lock down
// the eventual ownership shape.
#[allow(dead_code)]
fn _shape_check<A: Allocator>(_a: &A) {}
