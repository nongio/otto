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
            Buffer as AllocBuffer, Fourcc, Modifier, Slot, Swapchain,
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
    /// Whether the element covers its geometry with fully opaque pixels.
    /// Background plane = true. All overlay planes = false.
    pub opaque: bool,
    /// Short name used in trace logging (e.g. "bg", "windows", "overlay").
    pub label: &'static str,
}

struct Inner {
    commit_counter: CommitCounter,
    engine: Arc<Engine>,
    /// Subtree to render. `None` → render a placeholder.
    node_ref: Option<NodeRef>,
    size: (i32, i32),
    /// When true, skip the root-position translate so nodes render at their
    /// global scene positions. Used for scanout_windows whose children are
    /// already scroll-compensated.
    skip_root_translate: bool,
    damage: DamageBag<i32, Physical>,
    swapchain: Option<Swapchain<GbmAllocator<DrmDeviceFd>>>,
    /// Arc wrapping the most-recently submitted swapchain slot.
    /// Passed to Smithay as `UnderlyingStorage::Dmabuf` keepalive so the
    /// slot stays alive until KMS is done scanning it. Smithay holds a clone;
    /// `swapchain.acquire()` skips slots whose Arc refcount is > 1, giving
    /// automatic double/triple-buffering with no manual VBlank counting.
    current_slot: Option<Arc<Slot<GbmBuffer>>>,
    /// DRM render node — tagged onto each exported dmabuf so Smithay's
    /// GbmFramebufferExporter accepts it.
    render_node: Option<DrmNode>,
}

impl SceneDmabufElement {
    pub fn new(engine: Arc<Engine>, size: (i32, i32), label: &'static str) -> Self {
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
                skip_root_translate: false,
            })),
            position: (0, 0),
            plane_alpha: 1.0,
            opaque: false,
            label,
        }
    }

    pub fn set_skip_root_translate(&self, skip: bool) {
        self.inner.lock().unwrap().skip_root_translate = skip;
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
    /// (no subtree damage, no swapchain, no free slot, or surface creation failed).
    pub fn render(&self, renderer: &mut SkiaRenderer) -> bool {
        let mut inner = self.inner.lock().unwrap();

        // Skip re-render when a valid dmabuf already exists and there is nothing
        // new to draw.  Two cases:
        //   • Subtree element: skip when the subtree reports no damage.
        //   • Solid-black test element (no node_ref): skip always — a solid black
        //     buffer never changes, so one render is enough.
        // Capture the dirty rect before acquiring a swapchain slot.
        // `subtree_damage()` unions all per-node damage rects in the subtree.
        // Stored here so we can pass the tight rect to DamageBag instead of
        // always reporting full-buffer — lets the DRM compositor set
        // FB_DAMAGE_CLIPS correctly, enabling PSR partial-refresh on eDP.
        let dirty_rect: Option<layers::skia::Rect>;
        let has_dmabuf = self.current_dmabuf.lock().unwrap().is_some();
        if has_dmabuf {
            match inner.node_ref {
                Some(node_ref) => {
                    dirty_rect = inner.engine.subtree_damage(node_ref);
                    if dirty_rect.is_none() {
                        return false;
                    }
                }
                None => return false,
            }
        } else {
            dirty_rect = None;
        }

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
        let dmabuf = match slot.export() {
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
        tracing::trace!(target: "otto::planes", "SceneDmabufElement dmabuf: format={:?} modifier={:?}", AllocBuffer::format(&dmabuf).code, AllocBuffer::format(&dmabuf).modifier);

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
            // Clear to transparent before each render so stale swapchain slot
            // content doesn't accumulate under transparent regions.
            canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 0.0));
            let save_point = canvas.save();

            let scene = inner.engine.scene();
            let root = inner.node_ref;

            if let Some(root_id) = root {
                // Translate so the node's scene-space position maps to (0,0)
                // on the dmabuf canvas — same correction SceneElement::draw() applies.
                if !inner.skip_root_translate {
                    if let Some(layer) = inner.engine.get_layer(&root_id) {
                        let pos = layer.render_position();
                        // render_position() returns global scene coords.
                        // We apply the global offset as the initial canvas transform so
                        // that each child's accumulated local_transforms bring it to the
                        // correct output-space position. (Positive translate shifts the
                        // canvas origin to the node's global position, cancelling the
                        // parent-chain scroll encoded in local_transforms above the root.)
                        if pos.x != 0.0 || pos.y != 0.0 {
                            canvas.translate((pos.x, pos.y));
                        }
                    }
                }
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
                // No subtree — solid black (used as a test/placeholder plane).
                canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0));
            }

            canvas.restore_to_count(save_point);
            skia_surface.gr_context.flush_and_submit_surface(
                &mut skia_surface.surface,
                layers::skia::gpu::SyncCpu::Yes,
            );
        }

        // Update damage and commit counter.
        // Use the tight dirty rect when available so FB_DAMAGE_CLIPS is set
        // correctly for PSR partial-refresh; fall back to full-buffer on the
        // first render (no prior dmabuf) or when no node_ref is set.
        inner.commit_counter.increment();
        // Map the scene-space dirty rect into buffer space and clamp to buffer
        // bounds. The canvas was translated by the root node's render_position()
        // so buffer coords = scene coords − root_pos.
        let root_pos = inner.node_ref
            .and_then(|r| inner.engine.get_layer(&r))
            .map(|l| l.render_position())
            .unwrap_or_default();
        let damage_rect = dirty_rect
            .map(|r| {
                let x = ((r.left()  - root_pos.x) as i32).clamp(0, w);
                let y = ((r.top()   - root_pos.y) as i32).clamp(0, h);
                let x2 = ((r.right() - root_pos.x) as i32).clamp(0, w);
                let y2 = ((r.bottom()- root_pos.y) as i32).clamp(0, h);
                Rectangle::<i32, Physical>::new(
                    (x, y).into(),
                    ((x2 - x).max(1), (y2 - y).max(1)).into(),
                )
            })
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (w, h).into()));
        inner.damage.add(vec![damage_rect]);

        // Store the node-tagged dmabuf for underlying_storage().
        *self.current_dmabuf.lock().unwrap() = Some(dmabuf);

        // Wrap the slot in Arc and store it. The Arc clone passed via
        // underlying_storage() keeps the slot alive in Smithay until the
        // page flip completes; swapchain.acquire() skips it while refcount > 1.
        inner.current_slot = Some(Arc::new(slot));

        tracing::debug!(target: "otto::planes", "plane redrawn: {}", self.label);
        true
    }

    /// The dmabuf currently being scanned out, if any.
    pub fn current_dmabuf(&self) -> Option<Dmabuf> {
        self.current_dmabuf.lock().unwrap().clone()
    }

    /// Clear engine damage after all planes for this frame have been rendered.
    /// Must be called once per frame so `subtree_damage()` returns `None` on
    /// the next frame when nothing has changed.
    pub fn clear_engine_damage(&self) {
        self.inner.lock().unwrap().engine.clear_damage();
    }

    /// Save the current slot's rendered content to a PNG file (debug only).
    #[cfg(feature = "debug-kms")]
    pub fn save_to_png(&self, path: &str) {
        let inner = self.inner.lock().unwrap();
        if inner.swapchain.is_none() {
            tracing::warn!(target: "otto::planes", "save_to_png: no swapchain");
            return;
        }
        let slot = match inner.current_slot.as_ref() {
            Some(s) => s,
            None => { tracing::warn!(target: "otto::planes", "save_to_png: no current slot"); return; }
        };
        let slot_surface = match slot.userdata().get::<SlotSurface>() {
            Some(s) => s,
            None => { tracing::warn!(target: "otto::planes", "save_to_png: no surface on slot"); return; }
        };
        // SAFETY: single render thread, same safety invariant as render().
        let skia_surface = unsafe { &mut *slot_surface.0.get() };
        skia_surface.gr_context.flush_and_submit_surface(
            &mut skia_surface.surface,
            layers::skia::gpu::SyncCpu::Yes,
        );
        let image = skia_surface.surface.image_snapshot();
        let data = image
            .encode(Some(&mut skia_surface.gr_context), layers::skia::EncodedImageFormat::PNG, None)
            .or_else(|| {
                // Fallback: read pixels to CPU then encode.
                let info = image.image_info();
                let mut pixels = vec![0u8; (info.width() * info.height() * 4) as usize];
                let row_bytes = (info.width() * 4) as usize;
                if image.read_pixels(&info, &mut pixels, row_bytes, layers::skia::IPoint::new(0, 0), layers::skia::image::CachingHint::Disallow) {
                    let raster = layers::skia::images::raster_from_data(
                        &info,
                        layers::skia::Data::new_copy(&pixels),
                        row_bytes,
                    )?;
                    raster.encode(None, layers::skia::EncodedImageFormat::PNG, None)
                } else {
                    None
                }
            });
        if let Some(data) = data {
            if let Err(e) = std::fs::write(path, data.as_bytes()) {
                tracing::warn!(target: "otto::planes", "save_to_png write failed: {e}");
            } else {
                tracing::info!(target: "otto::planes", "saved dmabuf to {path}");
            }
        } else {
            tracing::warn!(target: "otto::planes", "save_to_png: image encode failed");
        }
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
        if self.opaque {
            smithay::backend::renderer::utils::OpaqueRegions::from_slice(&[self.geometry(scale)])
        } else {
            smithay::backend::renderer::utils::OpaqueRegions::default()
        }
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
        let inner = self.inner.lock().unwrap();
        let key = inner.node_ref
            .and_then(|n| inner.engine.get_layer(&n))
            .map(|l| l.key())
            .unwrap_or_default();
        drop(inner);
        tracing::trace!(
            target: "otto::planes",
            "SceneDmabufElement::draw() fallback — node={key} dst={dst:?}",
        );
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
        let keepalive = self
            .inner
            .lock()
            .unwrap()
            .current_slot
            .clone()
            .map(|s| s as Arc<dyn std::any::Any + Send + Sync>);
        // SAFETY: The Dmabuf lives inside Arc<Mutex<Option<Dmabuf>>> which is
        // owned by self. It is only replaced in render(), which completes
        // before underlying_storage() is called. No concurrent mutation.
        Some(UnderlyingStorage::Dmabuf(unsafe { &*ptr }, keepalive))
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
        let keepalive = self
            .inner
            .lock()
            .unwrap()
            .current_slot
            .clone()
            .map(|s| s as Arc<dyn std::any::Any + Send + Sync>);
        // SAFETY: same as UdevRenderer impl above.
        Some(UnderlyingStorage::Dmabuf(unsafe { &*ptr }, keepalive))
    }
}
