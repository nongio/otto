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

/// `touch /tmp/otto-no-window-plane` — disables the subtree-plane tier (a
/// promoted window's own KMS plane) while leaving raw client-buffer scanout
/// alone, so the two tiers can be measured against each other and against
/// plain compositing. Polled at 1 Hz alongside `NO_SCANOUT`.
pub static NO_WINDOW_PLANE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// One-shot request to dump every plane buffer to PNG
/// (`touch /tmp/otto-dump-planes`; polled at 1 Hz, consumed by the renderer).
pub static DUMP_PLANES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct SlotSurface {
    /// Stable identity for this swapchain slot, for lifecycle tracing: which
    /// slot a render targeted, which one was handed to KMS, and which one the
    /// display ends up scanning. Assigned on the slot's first use.
    id: usize,
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
        static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
    /// Skip rendering while an ANCESTOR of the subtree root is hidden in the
    /// scene arena (see `render_inner`). Plane subtrees deliberately ignore
    /// ancestor visibility — exposé lives under the hidden `workspaces_layer`
    /// and must keep rendering there — so this is opt-in, for planes whose
    /// buffer stays on screen while their content is hidden. Today: the
    /// background plane, which exposé covers without replacing.
    pub honor_ancestor_visibility: bool,
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
    /// Allocator inputs kept from `ensure_swapchain`, so `resize` can rebuild
    /// the swapchain at a new size without the caller replumbing them. Only
    /// the window plane resizes today — every other plane is allocated once
    /// at the output's mode size.
    gbm: Option<smithay::backend::allocator::gbm::GbmDevice<DrmDeviceFd>>,
    format: Option<Fourcc>,
    /// Composite of the planes BELOW this one (global scene coordinates), its
    /// resolution relative to the scene (1.0 = full, 0.25 = quarter), whether
    /// that image is pre-blurred, and an optional *raw* (unblurred) copy at the
    /// same scale for `blur_include_content` layers (stacked popups). Passed to
    /// `render_node_tree` so `BackgroundBlur` layers in the subtree sample the
    /// real content behind the plane (cross-buffer vibrancy).
    backdrop: Option<(layers::skia::Image, f32, bool, Option<layers::skia::Image>)>,
    /// `unique_id()` of the backdrop the current buffer was rendered with —
    /// a backdrop swap forces a re-render even when the subtree is clean.
    last_backdrop_id: Option<u32>,
    /// Slot id behind `current_dmabuf` — what KMS is being handed.
    current_slot_id: Option<usize>,
    /// One-shot: the next `render()` runs unconditionally with full damage.
    /// Set when the plane re-activates after being idle — its buffer still
    /// holds whatever was rendered last (e.g. a tooltip that has since been
    /// destroyed), and subtree damage from the removal was cleared while the
    /// plane sat out of the frame, so pushing the stale buffer would flash
    /// ghost content.
    force_full: bool,
    /// Widen the clip to the whole buffer on frames this element renders
    /// anyway — but do NOT, unlike `force_full`, cause a render on a frame
    /// with nothing to draw.
    ///
    /// Set while a popup is in the subtree: a popup's blur samples what the
    /// same pass painted behind it (`blur_include_content`), and a partial
    /// repaint only paints — and only clears — inside the damage clip, so
    /// outside it the blur reads whatever this swapchain slot held before.
    /// Every repaint therefore has to be a full one. A frame with no damage
    /// and no backdrop change repaints nothing, so it needs no such
    /// protection — and forcing one there was re-rasterising the full-screen
    /// overlay plane on every frame for as long as any menu was open.
    full_clip_when_rendering: bool,
    /// The output's static position in scene coordinates (physical px).
    /// The root node's `render_position()` includes this offset, but the
    /// buffer is scanned out on the output's own CRTC where (0,0) is the
    /// output's top-left — so the render translate must re-apply only the
    /// dynamic part of the root position (workspace scroll), not the
    /// output placement. Always (0,0) for the leftmost output.
    scene_origin: (i32, i32),
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
                current_slot_id: None,
                swapchain: None,
                current_slot: None,
                render_node: None,
                gbm: None,
                format: None,
                viewport: (0, 0),
                backdrop: None,
                last_backdrop_id: None,
                force_full: false,
                full_clip_when_rendering: false,
                scene_origin: (0, 0),
            })),
            position: (0, 0),
            plane_alpha: 1.0,
            opaque: false,
            kind: Kind::ScanoutCandidate,
            label,
            honor_ancestor_visibility: false,
        }
    }

    /// Set the viewport origin (scene physical px). Callers must also set
    /// `self.position` to the same origin so the plane lands where the
    /// buffer's content was cropped from.
    pub fn set_viewport(&self, origin: (i32, i32)) {
        self.inner.lock().unwrap().viewport = origin;
    }

    /// Set the output's static scene position (physical px) — see
    /// `Inner::scene_origin`. Must be called for any output not at (0,0).
    pub fn set_scene_origin(&self, origin: (i32, i32)) {
        self.inner.lock().unwrap().scene_origin = origin;
    }

    /// The output's static scene position (physical px) — see `Inner::scene_origin`.
    pub fn scene_origin(&self) -> (i32, i32) {
        self.inner.lock().unwrap().scene_origin
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

    /// Request that the next render of this element covers the full buffer,
    /// WITHOUT causing a render on a frame that has nothing to draw. Call for
    /// as long as a condition holds that makes partial repaints unsafe — see
    /// `Inner::full_clip_when_rendering`. Cleared once a render consumes it.
    pub fn request_full_clip_when_rendering(&self) {
        self.inner.lock().unwrap().full_clip_when_rendering = true;
    }

    /// Provide the composite of the planes below this one so `BackgroundBlur`
    /// layers in the subtree sample real content (cross-buffer vibrancy).
    /// `scale` is the image's resolution relative to the scene. A new image
    /// (different `unique_id`) triggers a re-render on the next `render()`.
    /// The `bool` is whether `image` is already blurred — if so the consuming
    /// `BackgroundBlur` layer seeds it directly and skips its own (shape-clipped)
    /// blur pass, avoiding a faded rim at the layer edge. The trailing optional
    /// image is a raw (unblurred) copy for `blur_include_content` layers.
    pub fn set_backdrop(
        &self,
        backdrop: Option<(layers::skia::Image, f32, bool, Option<layers::skia::Image>)>,
    ) {
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
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        inner.swapchain = Some(Swapchain::new(
            allocator,
            w as u32,
            h as u32,
            format,
            vec![Modifier::Linear],
        ));
        inner.render_node = Some(render_node);
        inner.gbm = Some(gbm);
        inner.format = Some(format);
        Ok(())
    }

    /// The buffer size this element currently renders at.
    pub fn size(&self) -> (i32, i32) {
        self.inner.lock().unwrap().size
    }

    /// Re-allocate the swapchain at `size`. No-op when the size is unchanged.
    ///
    /// Only the per-window plane needs this: its buffer is the window's own
    /// bounds, so it follows every resize. The old slots are dropped, which
    /// means the element reports no dmabuf until the next `render()` — the
    /// caller must therefore resize BEFORE rendering in the same frame, or
    /// the plane sits out a frame and the window blinks. The swapchain is
    /// rebuilt rather than resized in place because Smithay's `Swapchain`
    /// has no resize that also drops slots already handed to KMS.
    pub fn resize(&self, size: (i32, i32)) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.size == size {
            return false;
        }
        let (Some(gbm), Some(format), Some(render_node)) =
            (inner.gbm.clone(), inner.format, inner.render_node)
        else {
            // Never allocated — `ensure_swapchain` has not run yet. Record the
            // size so the eventual allocation uses it.
            inner.size = size;
            return true;
        };
        let (w, h) = size;
        let allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        inner.size = size;
        inner.swapchain = Some(Swapchain::new(
            allocator,
            w as u32,
            h as u32,
            format,
            vec![Modifier::Linear],
        ));
        inner.render_node = Some(render_node);
        // Every slot the damage history referred to is gone, so the next
        // render must paint the whole buffer.
        inner.current_slot = None;
        inner.current_slot_id = None;
        inner.damage.reset();
        inner.force_full = true;
        drop(inner);
        *self.current_dmabuf.lock().unwrap() = None;
        true
    }

    /// Set the viewport origin and the on-screen position together — they are
    /// always the same point for a plane whose buffer is a crop of the scene,
    /// and setting only one of them silently offsets the plane.
    pub fn set_origin(&mut self, origin: (i32, i32)) {
        self.position = origin;
        self.set_viewport(origin);
    }

    /// Render the scene subtree into the next free swapchain slot.
    ///
    /// Returns `true` if a new frame was rendered, `false` if skipped
    /// (no subtree damage, no swapchain, no free slot, or surface creation failed).
    pub fn render(&self, renderer: &mut SkiaRenderer) -> bool {
        // Timing wrapper: under plane decomposition this call is where the
        // Skia work for a plane buffer happens, so it's the only place the
        // per-plane cost is visible. Only a real re-render is recorded — the
        // early-out paths below are cheap and would dilute the mean.
        let started = std::time::Instant::now();
        let rendered = self.render_inner(renderer);
        if rendered {
            crate::render_phase_stats::record_plane_render(self.label, started.elapsed());
        }
        rendered
    }

    fn render_inner(&self, renderer: &mut SkiaRenderer) -> bool {
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
        let backdrop_id = inner
            .backdrop
            .as_ref()
            .map(|(img, _, _, _)| img.unique_id());
        let backdrop_changed = backdrop_id != inner.last_backdrop_id;
        let force_full = std::mem::take(&mut inner.force_full);
        // Read, don't consume: a frame that skips below must leave this armed
        // for whichever later frame actually renders.
        let full_clip = inner.full_clip_when_rendering;
        let has_dmabuf = self.current_dmabuf.lock().unwrap().is_some();
        if has_dmabuf {
            match inner.node_ref {
                Some(node_ref) => dirty_rect = inner.engine.subtree_damage(node_ref),
                None => return false,
            }
        } else {
            dirty_rect = None;
        }

        // Map the scene-space dirty rect into buffer space, clipped to the
        // buffer — used both to clip this render and to report damage below.
        // `render_node_tree` draws the subtree with the root node at the canvas
        // origin and the draw path then translates back by the root's *dynamic*
        // offset (`render_position() − scene_origin`), so scene content lands at
        // `scene − scene_origin − viewport`. Subtracting the root's full
        // `render_position()` here instead would double-count the workspace
        // scroll: on any workspace but the first, the windows/background plane
        // roots sit a full output width to the left, and every dirty rect
        // clamped to a sliver at the buffer edge — a window on workspace 2
        // stopped repainting (Chrome scrolling froze on screen).
        //
        // `None` means the damage lies entirely outside this buffer, which is
        // the normal case for a window on a workspace scrolled off screen:
        // nothing visible changed, so the plane must not redraw at all. Letting
        // it through would repaint (and report FB_DAMAGE_CLIPS for) an edge
        // sliver on every frame a background workspace animates.
        let (w, h) = inner.size;
        let (ox, oy) = inner.scene_origin;
        let (vx, vy) = inner.viewport;
        let visible_damage = dirty_rect.and_then(|r| {
            let x = (r.left().floor() as i32 - ox - vx).clamp(0, w);
            let y = (r.top().floor() as i32 - oy - vy).clamp(0, h);
            let x2 = (r.right().ceil() as i32 - ox - vx).clamp(0, w);
            let y2 = (r.bottom().ceil() as i32 - oy - vy).clamp(0, h);
            if x2 <= x || y2 <= y {
                return None;
            }
            Some(Rectangle::<i32, Physical>::new(
                (x, y).into(),
                (x2 - x, y2 - y).into(),
            ))
        });
        // Debug (`/tmp/otto-bgdbg`): the inputs that decide whether this plane
        // repaints, and over what region. A background flash shows up here as a
        // frame whose clip/damage covers only part of the buffer.
        let bgdbg = std::path::Path::new("/tmp/otto-bgdbg").exists();
        if bgdbg {
            tracing::info!(
                target: "otto::bgdbg",
                "{}: dirty={:?} visible={:?} has_dmabuf={} backdrop_changed={} force_full={} origin={:?} viewport={:?} size={:?}",
                self.label, dirty_rect, visible_damage, has_dmabuf, backdrop_changed,
                force_full, inner.scene_origin, inner.viewport, inner.size,
            );
        }
        let decision = decide_plane_render(PlaneRenderInputs {
            has_dmabuf,
            damaged: visible_damage.is_some(),
            backdrop_changed,
            force_full,
            full_clip,
        });
        if !decision.render {
            if bgdbg {
                tracing::info!(
                    target: "otto::slot",
                    "{}: SKIP keeping slot={:?}", self.label, inner.current_slot_id,
                );
            }
            // `full_clip_when_rendering` is deliberately left armed here — see
            // `decide_plane_render`.
            return false;
        }
        if decision.consume_full_clip {
            inner.full_clip_when_rendering = false;
        }
        let force_full = decision.full_buffer;

        // Effective visibility from the SCENE ARENA — the state `render_node_tree`
        // will actually draw from — not from the Layer model. `set_hidden` is
        // applied to the arena a frame after the model changes, so on exposé's
        // closing edge the model reads visible while the arena still has
        // `workspaces_layer` (this subtree's ANCESTOR) hidden: a render on that
        // frame walks a hidden tree and produces a black buffer. It is also the
        // LAST render the gesture forces, so the black is what the display keeps.
        //
        // Skip the render and re-arm `force_full` instead: the arena applying
        // the un-hide marks engine damage, that damage schedules the next frame,
        // and the pending force_full makes that frame repaint in full.
        if let Some(node_ref) = inner.node_ref.filter(|_| self.honor_ancestor_visibility) {
            let ancestor_hidden = inner.engine.scene().with_arena(|arena| {
                let mut id = node_ref.0;
                loop {
                    let Some(node) = arena.get(id) else {
                        break false;
                    };
                    if node.is_removed() || node.get().hidden() {
                        break true;
                    }
                    match node.parent() {
                        Some(p) => id = p,
                        None => break false,
                    }
                }
            });
            if ancestor_hidden {
                inner.force_full = force_full || inner.force_full;
                if bgdbg {
                    tracing::info!(
                        target: "otto::slot",
                        "{}: ARENA-HIDDEN skip (force_full re-armed={})",
                        self.label, inner.force_full,
                    );
                }
                return false;
            }
        }

        let swapchain = match inner.swapchain.as_mut() {
            Some(s) => s,
            None => return false,
        };

        // Bailing after this point drops this frame's dirty_rect while the
        // engine damage still gets cleared at end of frame — the region would
        // never repaint. force_full makes the next successful render redraw
        // everything instead.
        let slot = match swapchain.acquire() {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(target: "otto::planes", "SceneDmabufElement: no free swapchain slot");
                inner.force_full = true;
                return false;
            }
            Err(e) => {
                tracing::warn!(target: "otto::planes", "SceneDmabufElement: acquire error: {e:?}");
                inner.force_full = true;
                return false;
            }
        };

        // Export dmabuf (Slot caches it in userdata on first call).
        let dmabuf = match slot.export() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(target: "otto::planes", "SceneDmabufElement: export error: {e:?}");
                inner.force_full = true;
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
                    inner.force_full = true;
                    return false;
                }
            }
        }

        // Full buffer when the backdrop changed (blur can repaint anywhere),
        // on a forced redraw, or on this element's first render.
        let damage_rect = visible_damage
            .filter(|_| !backdrop_changed && !force_full)
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (w, h).into()));

        // Render into the slot's Skia surface.
        {
            let slot_surface = slot.userdata().get::<SlotSurface>().unwrap();
            // Tell Skia its cached GL state is stale before drawing. Smithay
            // executes raw GL on the same EGL context between plane renders
            // (dmabuf imports, composite frames, cursor uploads), and Ganesh
            // trusts its cache across flushes — when the two disagree (scissor,
            // FBO binding, viewport), the plane's draws are silently dropped and
            // the buffer keeps its cleared color. On an empty workspace nothing
            // re-damages the background afterwards, so one lost render during an
            // exposé transition left the wallpaper permanently black. Verified
            // causally: with this reset toggled off at runtime the black
            // reproduced 3/3, toggled on 8/8 clean, same binary.
            {
                // SAFETY: single render thread; same access pattern as below.
                let ss = unsafe { &mut *slot_surface.surface.get() };
                ss.gr_context.reset(None);
            }
            if bgdbg {
                tracing::info!(
                    target: "otto::slot",
                    "{}: RENDER into slot={} (was slot={:?}) first_use={} force_full={} clip_will_be_full={}",
                    self.label,
                    slot_surface.id,
                    inner.current_slot_id,
                    slot_surface.last_commit.get().is_none(),
                    force_full,
                    backdrop_changed || !has_dmabuf || force_full,
                );
            }
            // SAFETY: single render thread; no concurrent access to this slot.
            let skia_surface = unsafe { &mut *slot_surface.surface.get() };

            // Clip to the damage accumulated since THIS slot last rendered
            // (slots rotate — a reacquired slot is several commits behind) so
            // an unchanged region is neither cleared nor redrawn. Full render
            // when the backdrop changed (blur can repaint anywhere), on a
            // slot's first use, or when the damage history no longer reaches
            // back to the slot's commit.
            let clip: Option<Rectangle<i32, Physical>> =
                if backdrop_changed || !has_dmabuf || force_full {
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

            if bgdbg {
                // Walk the subtree root and its children: a plane that renders
                // black is almost always one whose content is hidden or
                // transparent, not one whose damage maths went wrong.
                let tree = inner.node_ref.map(|n| {
                    let root = inner.engine.get_layer(&n);
                    let scene = inner.engine.scene();
                    let desc = |l: &layers::prelude::Layer| {
                        // `premultiplied_opacity` is the INHERITED value — the one
                        // `do_repaint` bails on — so it catches an ancestor fading
                        // the subtree out, which `hidden()` alone cannot see.
                        let premul = scene.with_arena(|arena| {
                            arena
                                .get(l.id.0)
                                .map(|n| n.get().render_layer().premultiplied_opacity)
                        });
                        format!(
                            "{}[hidden={} op={:.2} premul={:?} size={:?} pos={:?} kids={}]",
                            l.key(),
                            l.hidden(),
                            l.opacity(),
                            premul,
                            l.render_size(),
                            l.render_position(),
                            l.children().len()
                        )
                    };
                    match root {
                        Some(r) => {
                            let kids: Vec<String> = r
                                .children()
                                .iter()
                                .map(|k| {
                                    let g: Vec<String> = k.children().iter().map(&desc).collect();
                                    format!("{} => {}", desc(k), g.join(" + "))
                                })
                                .collect();
                            format!("{} -> {}", desc(&r), kids.join(" | "))
                        }
                        None => "<no layer>".to_string(),
                    }
                });
                tracing::info!(
                    target: "otto::bgdbg",
                    "{}: RENDER damage_rect={:?} clip={:?} slot_commit={:?} cur_commit={:?} tree={:?}",
                    self.label, damage_rect, clip,
                    slot_surface.last_commit.get(), inner.commit_counter, tree,
                );
            }

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
                    // render_position() returns global scene coords. Re-apply the
                    // dynamic part (workspace scroll) so swipes appear in the
                    // buffer, but not the output's static scene placement — the
                    // buffer scans out on this output's CRTC where (0,0) is the
                    // output's own top-left.
                    let (ox, oy) = inner.scene_origin;
                    let dx = pos.x - ox as f32;
                    let dy = pos.y - oy as f32;
                    if dx != 0.0 || dy != 0.0 {
                        canvas.translate((dx, dy));
                    }
                }
                let external_backdrop = inner.backdrop.as_ref().map(|(img, s, blurred, raw)| {
                    layers::drawing::ExternalBackdrop {
                        image: img,
                        scale: *s,
                        blurred: *blurred,
                        raw_image: raw.as_ref(),
                    }
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
            // A CPU-side sync IS required before any of these buffers reaches
            // KMS: Mesa iris does not attach implicit dma-fences to offscreen
            // EGLImage render targets (EXEC_OBJECT_ASYNC on everything but
            // winsys buffers), so the atomic commit does NOT wait for these GL
            // writes — without a wait, plane flips show unfinished buffers.
            //
            // But that wait does NOT belong here. Every plane's slot surface is
            // built from the renderer's single shared `DirectContext`, so one
            // wait after the last plane covers all of them. Waiting per plane
            // instead serialised CPU against GPU once per plane and measured
            // ~15 ms of blocked time each — 95-100% of a plane render, and the
            // dominant term in the frame budget. Submit without blocking here;
            // `flush_planes_for_scanout` does the single wait before the commit.
            //
            // Submitting (rather than merely recording) still matters for
            // ordering: the backdrop composite samples an earlier plane's
            // snapshot, and both live in the same GL context, so the queued
            // order is the correct order.
            let flush_started = std::time::Instant::now();
            skia_surface.gr_context.flush_and_submit_surface(
                &mut skia_surface.surface,
                // A/B (`/tmp/otto-bgsync`): block on this plane's own GPU work.
                if self.label == "bg" && std::path::Path::new("/tmp/otto-bgsync").exists() {
                    layers::skia::gpu::SyncCpu::Yes
                } else {
                    layers::skia::gpu::SyncCpu::No
                },
            );
            crate::render_phase_stats::record_plane_flush(self.label, flush_started.elapsed());
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
        inner.current_slot_id = slot.userdata().get::<SlotSurface>().map(|s| s.id);
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
            None => {
                tracing::warn!(target: "otto::planes", "save_to_png: no current slot");
                return;
            }
        };
        let slot_surface = match slot.userdata().get::<SlotSurface>() {
            Some(s) => s,
            None => {
                tracing::warn!(target: "otto::planes", "save_to_png: no surface on slot");
                return;
            }
        };
        // SAFETY: single render thread, same safety invariant as render().
        let skia_surface = unsafe { &mut *slot_surface.surface.get() };
        skia_surface
            .gr_context
            .flush_and_submit_surface(&mut skia_surface.surface, layers::skia::gpu::SyncCpu::Yes);
        let image = skia_surface.surface.image_snapshot();
        let data = image
            .encode(
                Some(&mut skia_surface.gr_context),
                layers::skia::EncodedImageFormat::PNG,
                None,
            )
            .or_else(|| {
                // Fallback: read pixels to CPU then encode.
                let info = image.image_info();
                let mut pixels = vec![0u8; (info.width() * info.height() * 4) as usize];
                let row_bytes = (info.width() * 4) as usize;
                if image.read_pixels(
                    info,
                    &mut pixels,
                    row_bytes,
                    layers::skia::IPoint::new(0, 0),
                    layers::skia::image::CachingHint::Disallow,
                ) {
                    let raster = layers::skia::images::raster_from_data(
                        info,
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

/// Inputs to the per-frame decision of whether a plane element redraws its
/// buffer, and over how much of it. Pre-reduced to booleans so the decision is
/// unit-testable — the regression it guards is described below.
#[derive(Clone, Copy)]
pub(crate) struct PlaneRenderInputs {
    /// A buffer already exists (its absence forces a first, full render).
    pub has_dmabuf: bool,
    /// The subtree reported damage that lands on screen.
    pub damaged: bool,
    /// A new backdrop image arrived — blur can repaint anywhere.
    pub backdrop_changed: bool,
    /// One-shot: render unconditionally, in full (plane re-activation,
    /// teardown, composite→planes edge).
    pub force_full: bool,
    /// Level-triggered: while set, any render must cover the whole buffer —
    /// but this must NOT itself cause a render.
    pub full_clip: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PlaneRenderDecision {
    /// Draw at all this frame.
    pub render: bool,
    /// Clip to the whole buffer rather than to accumulated damage.
    pub full_buffer: bool,
    /// Clear `full_clip_when_rendering` — only ever on a frame that draws.
    pub consume_full_clip: bool,
}

/// Whether this plane redraws this frame, and over how much of its buffer.
///
/// The regression this guards: `full_clip` (set for as long as a popup is in
/// the subtree, because a popup's blur samples what the same pass painted
/// behind it) used to be expressed as `force_full`, which also defeats the
/// no-damage skip. An open menu therefore re-rasterised the full-screen
/// overlay plane on EVERY frame for as long as it was open — measured at 33%
/// → 77% GPU busy with an unchanged frame rate, rebuild rate and zero popup
/// damage. The two are separate questions: `force_full` answers "must this
/// frame draw?", `full_clip` only answers "if it draws, how much?". A frame
/// with no damage and no backdrop change repaints nothing, so a partial clip
/// has nothing to get wrong and the buffer is already whole — and the flag
/// must survive such a frame for whichever later frame does draw.
pub(crate) fn decide_plane_render(i: PlaneRenderInputs) -> PlaneRenderDecision {
    let render = !i.has_dmabuf || i.damaged || i.backdrop_changed || i.force_full;
    PlaneRenderDecision {
        render,
        full_buffer: render && (!i.has_dmabuf || i.backdrop_changed || i.force_full || i.full_clip),
        consume_full_clip: render,
    }
}

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
        let out = match inner.damage.damage_since(commit) {
            Some(rects) if !rects.is_empty() => DamageSet::from_slice(&rects),
            None => DamageSet::from_slice(&[full]),
            _ => DamageSet::default(),
        };
        if std::path::Path::new("/tmp/otto-bgdbg").exists() {
            tracing::info!(
                target: "otto::bgdbg",
                "{}: damage_since(asked={:?} cur={:?}) -> {:?}",
                self.label, commit, inner.commit_counter, &*out,
            );
        }
        out
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
        let inner = self.inner.lock().unwrap();
        if std::path::Path::new("/tmp/otto-bgdbg").exists() {
            tracing::info!(
                target: "otto::slot",
                "{}: TO-KMS slot={:?} keepalive={}",
                self.label,
                inner.current_slot_id,
                inner.current_slot.is_some(),
            );
        }
        let keepalive = inner
            .current_slot
            .clone()
            .map(|s| s as Arc<dyn std::any::Any + Send + Sync>);
        drop(inner);
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
                Some((&src_rect, layers::skia::canvas::SrcRectConstraint::Fast)),
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

#[cfg(test)]
mod plane_render_tests {
    use super::*;

    /// Steady state: a buffer exists, nothing changed.
    fn quiet() -> PlaneRenderInputs {
        PlaneRenderInputs {
            has_dmabuf: true,
            damaged: false,
            backdrop_changed: false,
            force_full: false,
            full_clip: false,
        }
    }

    #[test]
    fn an_open_popup_alone_does_not_cause_a_render() {
        // THE regression. `full_clip` is set for every frame a popup is in the
        // subtree. On a frame with nothing to draw it must not drag the
        // full-screen plane through a re-rasterisation.
        let d = decide_plane_render(PlaneRenderInputs {
            full_clip: true,
            ..quiet()
        });
        assert!(
            !d.render,
            "an idle frame must still skip while a popup is up"
        );
        assert!(
            !d.consume_full_clip,
            "the flag must survive a skipped frame — a later frame still draws"
        );
    }

    #[test]
    fn a_popup_makes_any_render_a_full_one() {
        let d = decide_plane_render(PlaneRenderInputs {
            damaged: true,
            full_clip: true,
            ..quiet()
        });
        assert!(d.render);
        assert!(
            d.full_buffer,
            "a popup's blur samples the same pass, so partial repaints are unsafe"
        );
        assert!(d.consume_full_clip);
    }

    #[test]
    fn the_full_clip_survives_idle_frames_until_a_render_uses_it() {
        // Popup opens, several idle frames pass, then something damages: that
        // frame must still get the full clip.
        let mut armed = true;
        for _ in 0..5 {
            let d = decide_plane_render(PlaneRenderInputs {
                full_clip: armed,
                ..quiet()
            });
            assert!(!d.render);
            if d.consume_full_clip {
                armed = false;
            }
        }
        assert!(armed, "five idle frames must not have disarmed it");
        let d = decide_plane_render(PlaneRenderInputs {
            damaged: true,
            full_clip: armed,
            ..quiet()
        });
        assert!(d.render && d.full_buffer && d.consume_full_clip);
    }

    #[test]
    fn without_a_popup_a_damaged_frame_clips_to_its_damage() {
        let d = decide_plane_render(PlaneRenderInputs {
            damaged: true,
            ..quiet()
        });
        assert!(d.render);
        assert!(
            !d.full_buffer,
            "partial repaint is the whole point of damage"
        );
    }

    #[test]
    fn quiet_frames_skip() {
        let d = decide_plane_render(quiet());
        assert!(!d.render);
        assert!(!d.consume_full_clip);
    }

    #[test]
    fn force_full_still_forces_a_render() {
        // The edge-triggered cases (plane re-activation, popup teardown,
        // composite→planes) must keep working: they DO have to draw a frame
        // that has no damage of its own.
        let d = decide_plane_render(PlaneRenderInputs {
            force_full: true,
            ..quiet()
        });
        assert!(d.render && d.full_buffer);
    }

    #[test]
    fn a_new_backdrop_or_a_first_buffer_renders_in_full() {
        let d = decide_plane_render(PlaneRenderInputs {
            backdrop_changed: true,
            ..quiet()
        });
        assert!(d.render && d.full_buffer, "blur can repaint anywhere");
        let d = decide_plane_render(PlaneRenderInputs {
            has_dmabuf: false,
            ..quiet()
        });
        assert!(d.render && d.full_buffer, "first render is never partial");
    }
}
