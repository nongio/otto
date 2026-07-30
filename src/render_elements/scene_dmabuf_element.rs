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

/// When true, the GPU-composite fallback path washes its output red so it is
/// visually distinguishable from zero-copy plane scanout. Toggled at runtime
/// by `touch /tmp/otto-tint` (checked once per second in the udev renderer),
/// mirroring Smithay's `DebugFlags::TINT` which tints client textures.
pub static TINT_COMPOSITE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `touch /tmp/otto-no-scanout` — disables window promotion for A/B
/// comparisons. Polled at 1 Hz by the udev renderer, read per-frame here.
pub static NO_SCANOUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One-shot request to dump every plane buffer to PNG
/// (`touch /tmp/otto-dump-planes`; polled at 1 Hz, consumed by the renderer).
pub static DUMP_PLANES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);


struct SlotSurface {
    surface: UnsafeCell<SkiaSurface>,
    /// Element commit this slot's content was last rendered at. Swapchain
    /// slots rotate, so a reacquired slot is several commits old — the
    /// renderer clips to the damage accumulated since this commit instead
    /// of clearing and redrawing the whole buffer.
    last_commit: std::cell::Cell<Option<CommitCounter>>,
    /// Thread that created the surface. The GL/Skia state is thread-affine;
    /// all access AND the drop must happen on this thread.
    owner: std::thread::ThreadId,
}

impl SlotSurface {
    fn new(surface: SkiaSurface) -> Self {
        Self {
            surface: UnsafeCell::new(surface),
            last_commit: std::cell::Cell::new(None),
            owner: std::thread::current().id(),
        }
    }
}

impl Drop for SlotSurface {
    fn drop(&mut self) {
        // The keepalive Arc handed to Smithay nominally allows the last drop
        // to happen anywhere, but DrmCompositor releases it on the calloop
        // thread that runs frame_finish — the same render thread. Make a
        // violation of that assumption loud instead of silently corrupting
        // GL state.
        debug_assert_eq!(
            std::thread::current().id(),
            self.owner,
            "SlotSurface dropped off the render thread"
        );
    }
}

// SAFETY: accessed only from the render thread (enforced by the Drop assert
// above for the destructor; render()/snapshot() are only called from the
// backend render path). Send+Sync are required because Slot::userdata()
// demands them.
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
    /// Element kind reported to Smithay. `ScanoutCandidate` makes the element
    /// eligible for OVERLAY planes. The background must use `Unspecified`:
    /// it may only direct-scan the PRIMARY plane (which ignores kind) — on an
    /// overlay it stacks ABOVE the primary swapchain and, being opaque and
    /// full-output, hides every element that fell back to GPU compositing.
    pub kind: Kind,
    /// Short name used in trace logging (e.g. "bg", "windows", "overlay").
    pub label: &'static str,
}

struct Inner {
    commit_counter: CommitCounter,
    engine: Arc<Engine>,
    /// Subtree to render. `None` → render a placeholder.
    node_ref: Option<NodeRef>,
    size: (i32, i32),
    /// Viewport origin in scene coordinates (physical px). The buffer shows
    /// the scene rect `viewport .. viewport+size`; the caller positions the
    /// KMS plane at the same origin. Lets a small strip-sized buffer render a
    /// crop of a full-screen container (dock strip, app-switcher band) while
    /// the content keeps positioning itself with normal layout.
    viewport: (i32, i32),
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
    /// Composite of the planes BELOW this one (global scene coordinates) plus
    /// its resolution relative to the scene (1.0 = full, 0.25 = quarter).
    /// Passed to `render_node_tree` so `BackgroundBlur` layers in the subtree
    /// sample the real content behind the plane (cross-buffer vibrancy).
    backdrop: Option<(layers::skia::Image, f32, bool)>,
    /// `unique_id()` of the backdrop the current buffer was rendered with —
    /// a backdrop swap forces a re-render even when the subtree is clean.
    last_backdrop_id: Option<u32>,
    /// One-shot: the next `render()` runs unconditionally with full damage.
    /// Set when the plane re-activates after being idle — its buffer still
    /// holds whatever was rendered last (e.g. a tooltip that has since been
    /// destroyed), and subtree damage from the removal was cleared while the
    /// plane sat out of the frame, so pushing the stale buffer would flash
    /// ghost content.
    force_full: bool,
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
                viewport: (0, 0),
                backdrop: None,
                last_backdrop_id: None,
                force_full: false,
            })),
            position: (0, 0),
            plane_alpha: 1.0,
            opaque: false,
            kind: Kind::ScanoutCandidate,
            label,
        }
    }

    /// Set the viewport origin (scene physical px). Callers must also set
    /// `self.position` to the same origin so the plane lands where the
    /// buffer's content was cropped from.
    pub fn set_viewport(&self, origin: (i32, i32)) {
        self.inner.lock().unwrap().viewport = origin;
    }

    /// Set the lay-rs subtree this element renders.
    pub fn set_node_ref(&self, node: NodeRef) {
        self.inner.lock().unwrap().node_ref = Some(node);
    }

    /// Request that the next `render()` redraws the full buffer even if the
    /// subtree reports no damage. Call when the plane re-enters the frame
    /// after sitting out — see `Inner::force_full`.
    pub fn request_full_render(&self) {
        self.inner.lock().unwrap().force_full = true;
    }

    /// Provide the composite of the planes below this one so `BackgroundBlur`
    /// layers in the subtree sample real content (cross-buffer vibrancy).
    /// `scale` is the image's resolution relative to the scene. A new image
    /// (different `unique_id`) triggers a re-render on the next `render()`.
    /// The `bool` is whether `image` is already blurred — if so the consuming
    /// `BackgroundBlur` layer seeds it directly and skips its own (shape-clipped)
    /// blur pass, avoiding a faded rim at the layer edge.
    pub fn set_backdrop(&self, backdrop: Option<(layers::skia::Image, f32, bool)>) {
        self.inner.lock().unwrap().backdrop = backdrop;
    }

    /// Snapshot of the most recently rendered buffer, if any. Cheap when the
    /// buffer hasn't been re-rendered since the last call (Skia returns the
    /// cached snapshot with the same `unique_id`).
    pub fn snapshot(&self) -> Option<layers::skia::Image> {
        let inner = self.inner.lock().unwrap();
        let slot = inner.current_slot.as_ref()?;
        let slot_surface = slot.userdata().get::<SlotSurface>()?;
        // SAFETY: single render thread; no concurrent access to this slot.
        let skia_surface = unsafe { &mut *slot_surface.surface.get() };
        Some(skia_surface.surface.image_snapshot())
    }

    /// Skia GPU context of the current slot surface, if one exists yet.
    pub fn gr_context(&self) -> Option<layers::skia::gpu::DirectContext> {
        let inner = self.inner.lock().unwrap();
        let slot = inner.current_slot.as_ref()?;
        let slot_surface = slot.userdata().get::<SlotSurface>()?;
        // SAFETY: single render thread; no concurrent access to this slot.
        let skia_surface = unsafe { &*slot_surface.surface.get() };
        Some(skia_surface.gr_context.clone())
    }

    /// Damage recorded under this element's subtree since the engine's last
    /// `clear_damage()`, in global scene coordinates.
    pub fn subtree_damage(&self) -> Option<layers::skia::Rect> {
        let inner = self.inner.lock().unwrap();
        inner.node_ref.and_then(|n| inner.engine.subtree_damage(n))
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
        // A backdrop swap (new unique_id) means the content behind this
        // plane changed under a blur region — re-render even when the
        // subtree itself is clean so vibrancy tracks the planes below.
        let backdrop_id = inner.backdrop.as_ref().map(|(img, _, _)| img.unique_id());
        let backdrop_changed = backdrop_id != inner.last_backdrop_id;
        let force_full = std::mem::take(&mut inner.force_full);
        let has_dmabuf = self.current_dmabuf.lock().unwrap().is_some();
        if has_dmabuf {
            match inner.node_ref {
                Some(node_ref) => {
                    dirty_rect = inner.engine.subtree_damage(node_ref);
                    if dirty_rect.is_none() && !backdrop_changed && !force_full {
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
                        .insert_if_missing(|| SlotSurface::new(surface));
                }
                Err(e) => {
                    tracing::warn!(target: "otto::planes", "SceneDmabufElement: surface error: {e:?}");
                    return false;
                }
            }
        }

        // Map the scene-space dirty rect into buffer space and clamp to buffer
        // bounds — used both to clip this render and to report damage below.
        // The canvas is translated by the root node's render_position(), so
        // buffer coords = scene coords − root_pos − viewport.
        let (w, h) = inner.size;
        let root_pos = inner
            .node_ref
            .and_then(|r| inner.engine.get_layer(&r))
            .map(|l| l.render_position())
            .unwrap_or_default();
        let (vx, vy) = inner.viewport;
        let damage_rect = dirty_rect
            .filter(|_| !backdrop_changed && !force_full)
            .map(|r| {
                let x = ((r.left() - root_pos.x) as i32 - vx).clamp(0, w);
                let y = ((r.top() - root_pos.y) as i32 - vy).clamp(0, h);
                let x2 = ((r.right() - root_pos.x) as i32 - vx).clamp(0, w);
                let y2 = ((r.bottom() - root_pos.y) as i32 - vy).clamp(0, h);
                Rectangle::<i32, Physical>::new(
                    (x, y).into(),
                    ((x2 - x).max(1), (y2 - y).max(1)).into(),
                )
            })
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (w, h).into()));

        // Render into the slot's Skia surface.
        {
            let slot_surface = slot.userdata().get::<SlotSurface>().unwrap();
            // SAFETY: single render thread; no concurrent access to this slot.
            let skia_surface = unsafe { &mut *slot_surface.surface.get() };

            // Clip to the damage accumulated since THIS slot last rendered
            // (slots rotate — a reacquired slot is several commits behind) so
            // an unchanged region is neither cleared nor redrawn. Full render
            // when the backdrop changed (blur can repaint anywhere), on a
            // slot's first use, or when the damage history no longer reaches
            // back to the slot's commit.
            let clip: Option<Rectangle<i32, Physical>> = if backdrop_changed
                || !has_dmabuf
                || force_full
            {
                None
            } else {
                match inner.damage.damage_since(slot_surface.last_commit.get()) {
                    Some(rects) => {
                        let mut acc = damage_rect;
                        for r in rects.iter() {
                            acc = acc.merge(*r);
                        }
                        Some(acc)
                    }
                    None => None,
                }
            };

            let canvas = skia_surface.canvas();
            let save_point = canvas.save();
            if let Some(clip) = clip {
                canvas.clip_rect(
                    layers::skia::Rect::from_xywh(
                        clip.loc.x as f32,
                        clip.loc.y as f32,
                        clip.size.w as f32,
                        clip.size.h as f32,
                    ),
                    layers::skia::ClipOp::Intersect,
                    Some(false),
                );
            }
            // Clear (within the clip) so stale swapchain slot content doesn't
            // accumulate under transparent regions.
            canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 0.0));

            let scene = inner.engine.scene();
            let root = inner.node_ref;

            if let Some(root_id) = root {
                // Translate so the node's scene-space position maps to (0,0)
                // on the dmabuf canvas — same correction SceneElement::draw() applies.
                let (vx, vy) = inner.viewport;
                if vx != 0 || vy != 0 {
                    // Map the viewport origin to the buffer origin.
                    canvas.translate((-vx as f32, -vy as f32));
                }
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
                let external_backdrop =
                    inner
                        .backdrop
                        .as_ref()
                        .map(|(img, s, blurred)| layers::drawing::ExternalBackdrop {
                            image: img,
                            scale: *s,
                            blurred: *blurred,
                        });
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
                            external_backdrop,
                        );
                    });
                });
            } else {
                // No subtree — solid black (used as a test/placeholder plane).
                canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0));
            }

            canvas.restore_to_count(save_point);
            // CPU-blocking sync is REQUIRED before the buffer reaches KMS:
            // Mesa iris does not attach implicit dma-fences to offscreen
            // EGLImage render targets (EXEC_OBJECT_ASYNC on everything but
            // winsys buffers), so the atomic commit does NOT wait for these
            // GL writes — without this wait every plane flip visibly
            // flickers with an unfinished buffer. Removing the wait needs
            // an explicit fence instead: export an EGL native fence here
            // and deliver it as the plane's IN_FENCE_FD (follow-up).
            skia_surface.gr_context.flush_and_submit_surface(
                &mut skia_surface.surface,
                layers::skia::gpu::SyncCpu::Yes,
            );
        }

        // Update damage and commit counter. The tight damage rect keeps
        // FB_DAMAGE_CLIPS accurate for PSR partial-refresh; full-buffer on
        // the first render, when no node_ref is set, or when the backdrop
        // changed (blur regions repaint without subtree damage). Record the
        // new commit on the slot so its next render clips to the delta.
        inner.last_backdrop_id = backdrop_id;
        inner.commit_counter.increment();
        inner.damage.add(vec![damage_rect]);
        if let Some(ss) = slot.userdata().get::<SlotSurface>() {
            ss.last_commit.set(Some(inner.commit_counter));
        }

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
        let skia_surface = unsafe { &mut *slot_surface.surface.get() };
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
        self.kind
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
        tracing::debug!(
            target: "otto::planes",
            "plane demoted to GPU composite: {} dst={dst:?}",
            self.label,
        );
        RenderElement::<SkiaRenderer>::draw(self, frame.as_mut(), src, dst, damage, opaque_regions)
            .map_err(|e| e.into())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut UdevRenderer<'renderer>,
    ) -> Option<UnderlyingStorage<'_>> {
        let dmabuf = self.current_dmabuf.lock().unwrap().clone()?;
        let keepalive = self
            .inner
            .lock()
            .unwrap()
            .current_slot
            .clone()
            .map(|s| s as Arc<dyn std::any::Any + Send + Sync>);
        Some(UnderlyingStorage::Dmabuf(dmabuf, keepalive))
    }
}

impl RenderElement<SkiaRenderer> for SceneDmabufElement {
    fn draw<'frame>(
        &self,
        frame: &mut <SkiaRenderer as RendererSuper>::Frame<'frame, 'frame>,
        src: Rectangle<f64, BufferCoord>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), <SkiaRenderer as RendererSuper>::Error> {
        // GPU-composite fallback: this element did not get a hardware plane
        // this frame, so Smithay composites it into the primary swapchain.
        // Blit the current slot's rendered content — a no-op here makes the
        // whole plane's content vanish (black) whenever assignment fails.
        let Some(image) = self.snapshot() else {
            return Ok(());
        };
        let mut surface = frame.skia_surface.clone();
        let canvas = surface.canvas();
        let src_rect = layers::skia::Rect::from_xywh(
            src.loc.x as f32,
            src.loc.y as f32,
            src.size.w as f32,
            src.size.h as f32,
        );
        let dst_rect = layers::skia::Rect::from_xywh(
            dst.loc.x as f32,
            dst.loc.y as f32,
            dst.size.w as f32,
            dst.size.h as f32,
        );
        let sampling = layers::skia::SamplingOptions::new(
            layers::skia::FilterMode::Linear,
            layers::skia::MipmapMode::None,
        );
        let mut paint = layers::skia::Paint::default();
        paint.set_alpha_f(self.plane_alpha);
        // Damage rects are element-local; offset to dst and clip each blit.
        for rect in damage {
            let save = canvas.save();
            let clip = layers::skia::Rect::from_xywh(
                (dst.loc.x + rect.loc.x) as f32,
                (dst.loc.y + rect.loc.y) as f32,
                rect.size.w as f32,
                rect.size.h as f32,
            );
            canvas.clip_rect(clip, layers::skia::ClipOp::Intersect, Some(false));
            canvas.draw_image_rect_with_sampling_options(
                &image,
                Some((
                    &src_rect,
                    layers::skia::canvas::SrcRectConstraint::Fast,
                )),
                dst_rect,
                sampling,
                &paint,
            );
            if TINT_COMPOSITE.load(std::sync::atomic::Ordering::Relaxed) {
                // Debug: mark GPU-composited plane content with a red wash.
                canvas.draw_color(
                    layers::skia::Color4f::new(1.0, 0.0, 0.0, 0.25),
                    layers::skia::BlendMode::SrcOver,
                );
            }
            canvas.restore_to_count(save);
        }
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut SkiaRenderer) -> Option<UnderlyingStorage<'_>> {
        let dmabuf = self.current_dmabuf.lock().unwrap().clone()?;
        let keepalive = self
            .inner
            .lock()
            .unwrap()
            .current_slot
            .clone()
            .map(|s| s as Arc<dyn std::any::Any + Send + Sync>);
        Some(UnderlyingStorage::Dmabuf(dmabuf, keepalive))
    }
}
