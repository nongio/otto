//! Dmabuf-backed scene render element with swapchain buffering.
//!
//! Renders a lay-rs scene subtree (identified by a `NodeRef`) into a
//! GBM-allocated swapchain. Each acquired slot is exported as a `Dmabuf`
//! and exposed to Smithay via `UnderlyingStorage::Dmabuf`, making the
//! element eligible for direct KMS plane assignment.
//!
//! The swapchain (2–3 slots) prevents the single-buffer tearing we hit
//! when KMS scans a buffer while the GPU is still writing it.

use std::cell::UnsafeCell;
use std::sync::{Arc, Mutex};

use layers::drawing::render_node_tree;
use layers::engine::{Engine, NodeRef};
use smithay::{
    backend::{
        allocator::{
            dmabuf::{AsDmabuf, Dmabuf},
            gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags},
            Fourcc, Modifier, Slot, Swapchain,
        },
        drm::{DrmDeviceFd, DrmNode},
        renderer::{
            element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
            utils::{CommitCounter, DamageBag, DamageSet},
            RendererSuper,
        },
    },
    utils::{Buffer as BufferCoord, Physical, Point, Rectangle, Scale},
};

use crate::{skia_renderer::SkiaRenderer, udev::UdevRenderer};

// ── Slot-local SkiaSurface ─────────────────────────────────────────────────
//
// Smithay's Slot::userdata() requires Send+Sync. SkiaSurface holds GL/Skia
// state that is !Send by default. We wrap it in UnsafeCell and assert Send+Sync
// because all access is confined to the single render thread.

use crate::renderer::skia_surface::SkiaSurface;

struct SlotSurface(UnsafeCell<SkiaSurface>);
// SAFETY: accessed only from the render thread; never aliased across threads.
unsafe impl Send for SlotSurface {}
unsafe impl Sync for SlotSurface {}

// ── SceneDmabufElement ─────────────────────────────────────────────────────

/// A render element that draws a lay-rs scene subtree into a GBM swapchain
/// and exposes the result as a `Dmabuf` for direct KMS plane assignment.
#[derive(Clone)]
pub struct SceneDmabufElement {
    id: Id,
    inner: Arc<Mutex<Inner>>,
    /// Last exported dmabuf, stored outside the lock so `underlying_storage()`
    /// can hand out `&Dmabuf` without holding a MutexGuard.
    current_dmabuf: Arc<Mutex<Option<Dmabuf>>>,
    /// Position in physical output coordinates.
    pub position: (i32, i32),
    /// KMS plane-level alpha (0.0–1.0).
    pub plane_alpha: f32,
}

struct Inner {
    commit_counter: CommitCounter,
    engine: Arc<Engine>,
    /// Subtree to render. `None` → render a placeholder.
    node_ref: Option<NodeRef>,
    size: (i32, i32),
    damage: DamageBag<i32, Physical>,
    swapchain: Option<Swapchain<GbmAllocator<DrmDeviceFd>>>,
    /// Slot held across the frame while KMS scans it out.
    current_slot: Option<Slot<GbmBuffer>>,
    /// DRM render node — tagged onto each exported dmabuf so Smithay's
    /// GbmFramebufferExporter accepts it.
    render_node: Option<DrmNode>,
}

impl SceneDmabufElement {
    pub fn new(engine: Arc<Engine>, size: (i32, i32)) -> Self {
        Self {
            id: Id::new(),
            current_dmabuf: Arc::new(Mutex::new(None)),
            inner: Arc::new(Mutex::new(Inner {
                commit_counter: CommitCounter::default(),
                engine,
                node_ref: None,
                size,
                damage: DamageBag::new(5),
                swapchain: None,
                current_slot: None,
                render_node: None,
            })),
            position: (0, 0),
            plane_alpha: 1.0,
        }
    }

    /// Set the lay-rs subtree this element renders.
    pub fn set_node_ref(&self, node: NodeRef) {
        self.inner.lock().unwrap().node_ref = Some(node);
    }

    /// Backwards-compatible alias used by the existing udev render path.
    pub fn set_output_root(&self, node: NodeRef) {
        self.set_node_ref(node);
    }

    /// Allocate the GBM swapchain (3 slots, RENDERING|SCANOUT flags).
    /// Idempotent — does nothing if the swapchain already exists.
    pub fn ensure_swapchain(
        &self,
        gbm: smithay::backend::allocator::gbm::GbmDevice<DrmDeviceFd>,
        format: Fourcc,
        render_node: DrmNode,
    ) -> Result<(), AllocError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.swapchain.is_some() {
            return Ok(());
        }
        let (w, h) = inner.size;
        let allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        inner.swapchain = Some(Swapchain::new(
            allocator,
            w as u32,
            h as u32,
            format,
            vec![Modifier::Linear],
        ));
        inner.render_node = Some(render_node);
        Ok(())
    }

    /// Render the scene subtree into the next free swapchain slot.
    ///
    /// Returns `true` if a new frame was rendered, `false` if skipped
    /// (no swapchain, no free slot, or surface creation failed).
    pub fn render(&self, renderer: &mut SkiaRenderer) -> bool {
        let mut inner = self.inner.lock().unwrap();

        let swapchain = match inner.swapchain.as_mut() {
            Some(s) => s,
            None => return false,
        };

        let slot = match swapchain.acquire() {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(target: "otto::planes", "SceneDmabufElement: no free swapchain slot");
                return false;
            }
            Err(e) => {
                tracing::warn!(target: "otto::planes", "SceneDmabufElement: acquire error: {e:?}");
                return false;
            }
        };

        // Export dmabuf (Slot caches it in userdata on first call).
        let mut dmabuf = match slot.export() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(target: "otto::planes", "SceneDmabufElement: export error: {e:?}");
                return false;
            }
        };

        // Tag with render node so GbmFramebufferExporter accepts it.
        if let Some(node) = inner.render_node {
            dmabuf.set_node(node);
        }

        // Create a SkiaSurface for this slot on first use.
        if slot.userdata().get::<SlotSurface>().is_none() {
            match renderer.create_surface_from_dmabuf(&dmabuf) {
                Ok(surface) => {
                    slot.userdata()
                        .insert_if_missing(|| SlotSurface(UnsafeCell::new(surface)));
                }
                Err(e) => {
                    tracing::warn!(target: "otto::planes", "SceneDmabufElement: surface error: {e:?}");
                    return false;
                }
            }
        }

        // Render into the slot's Skia surface.
        let (w, h) = inner.size;
        {
            let slot_surface = slot.userdata().get::<SlotSurface>().unwrap();
            // SAFETY: single render thread; no concurrent access to this slot.
            let skia_surface = unsafe { &mut *slot_surface.0.get() };
            let canvas = skia_surface.canvas();
            let save_point = canvas.save();

            let scene = inner.engine.scene();
            let root = inner.node_ref.or_else(|| inner.engine.scene_root());

            if let Some(root_id) = root {
                scene.with_arena(|arena| {
                    scene.with_renderable_arena(|renderable_arena| {
                        render_node_tree(
                            root_id,
                            arena,
                            renderable_arena,
                            canvas,
                            1.0,
                            None,
                            None,
                        );
                    });
                });
            } else {
                // No subtree configured — render a visible placeholder so we
                // can tell the element is alive during debugging.
                canvas.clear(layers::skia::Color4f::new(0.1, 0.1, 0.1, 1.0));
            }

            canvas.restore_to_count(save_point);
            skia_surface.gr_context.flush_and_submit_surface(
                &mut skia_surface.surface,
                layers::skia::gpu::SyncCpu::Yes,
            );
        }

        // Update damage and commit counter.
        inner.commit_counter.increment();
        inner
            .damage
            .add(vec![Rectangle::new((0, 0).into(), (w, h).into())]);

        // Store the node-tagged dmabuf for underlying_storage().
        *self.current_dmabuf.lock().unwrap() = Some(dmabuf);

        // Hold the slot until VBlank signals it's safe to release.
        inner.current_slot = Some(slot);

        true
    }

    /// Release the current swapchain slot back to the pool.
    /// Call this from the VBlank / frame_submitted callback.
    pub fn submitted(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(slot) = inner.current_slot.take() {
            if let Some(swapchain) = &mut inner.swapchain {
                swapchain.submitted(&slot);
            }
        }
    }

    /// The dmabuf currently being scanned out, if any.
    pub fn current_dmabuf(&self) -> Option<Dmabuf> {
        self.current_dmabuf.lock().unwrap().clone()
    }
}

/// Error from swapchain allocation (currently a placeholder — GBM errors
/// surface through the swapchain's Allocator trait).
#[derive(Debug, thiserror::Error)]
pub enum AllocError {
    #[error("gbm allocation failed")]
    Gbm,
}

// ── Element trait ──────────────────────────────────────────────────────────

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
        let full = Rectangle::new((0, 0).into(), (inner.size.0, inner.size.1).into());
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
        Kind::ScanoutCandidate
    }

    fn opaque_regions(
        &self,
        scale: Scale<f64>,
    ) -> smithay::backend::renderer::utils::OpaqueRegions<i32, Physical> {
        smithay::backend::renderer::utils::OpaqueRegions::from_slice(&[self.geometry(scale)])
    }
}

// ── RenderElement impls ────────────────────────────────────────────────────

impl<'renderer> RenderElement<UdevRenderer<'renderer>> for SceneDmabufElement {
    fn draw(
        &self,
        frame: &mut <UdevRenderer<'renderer> as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <UdevRenderer<'renderer> as RendererSuper>::Error> {
        // draw() is only called when Smithay fell back to GPU composite.
        // Log once so we know the plane assignment failed.
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static LOGGED: AtomicBool = AtomicBool::new(false);
            if !LOGGED.swap(true, Ordering::Relaxed) {
                tracing::info!(
                    target: "otto::planes",
                    "SceneDmabufElement::draw() — plane assignment failed, compositing. \
                     dst={dst:?} damage_count={}",
                    damage.len()
                );
            }
        }
        let _ = (frame, src, dst, damage, opaque_regions);
        Ok(())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut UdevRenderer<'renderer>,
    ) -> Option<UnderlyingStorage<'_>> {
        let guard = self.current_dmabuf.lock().unwrap();
        let ptr = guard.as_ref()? as *const Dmabuf;
        drop(guard);
        // SAFETY: The Dmabuf lives inside Arc<Mutex<Option<Dmabuf>>> which is
        // owned by self. It is only replaced in render(), which completes
        // before underlying_storage() is called. No concurrent mutation.
        Some(UnderlyingStorage::Dmabuf(unsafe { &*ptr }))
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
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut SkiaRenderer) -> Option<UnderlyingStorage<'_>> {
        let guard = self.current_dmabuf.lock().unwrap();
        let ptr = guard.as_ref()? as *const Dmabuf;
        drop(guard);
        // SAFETY: same as UdevRenderer impl above.
        Some(UnderlyingStorage::Dmabuf(unsafe { &*ptr }))
    }
}
