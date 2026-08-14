use std::collections::hash_map::HashMap;
use std::sync::atomic::AtomicBool;
#[cfg(feature = "metrics")]
use std::sync::Arc;

#[cfg(feature = "renderer_sync")]
use smithay::backend::renderer::sync::SyncPoint;
use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::DrmCompositor, exporter::gbm::GbmFramebufferExporter, DrmDevice,
            DrmDeviceFd, DrmNode,
        },
        renderer::{
            multigpu::{gbm::GbmGlesBackend, GpuManager, MultiRenderer, MultiTexture},
            ContextId,
        },
        session::libseat::LibSeatSession,
    },
    desktop::utils::OutputPresentationFeedback,
    reexports::{
        calloop::RegistrationToken,
        drm::control::{connector, crtc},
        wayland_server::{backend::GlobalId, DisplayHandle},
    },
    utils::{Physical, Rectangle},
    wayland::{
        dmabuf::{DmabufFeedback, DmabufGlobal, DmabufState},
        drm_lease::DrmLeaseState,
    },
};
use smithay_drm_extras::drm_scanner::DrmScanner;

use crate::skia_renderer::SkiaRenderer;

// Supported pixel formats for rendering, in preference order.
// Argb8888 maps to GL_BGRA_EXT which is Skia's native kN32 (BGRA8888) — no
// channel swizzle needed.  Abgr2101010 is the only 10-bit format with a GL
// mapping in smithay; Argb2101010 has none and is omitted.
pub const SUPPORTED_FORMATS: &[Fourcc] = &[Fourcc::Abgr2101010, Fourcc::Argb8888, Fourcc::Abgr8888];

pub const SUPPORTED_FORMATS_8BIT_ONLY: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Abgr8888];

/// Multi-GPU renderer type for udev backend
pub type UdevRenderer<'a> = MultiRenderer<
    'a,
    'a,
    GbmGlesBackend<SkiaRenderer, DrmDeviceFd>,
    GbmGlesBackend<SkiaRenderer, DrmDeviceFd>,
>;

/// DRM compositor using GBM allocation
pub type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

/// Unique identifier for a udev output (device + CRTC)
#[derive(Debug, PartialEq)]
pub struct UdevOutputId {
    pub device_id: DrmNode,
    pub crtc: crtc::Handle,
    pub is_laptop_panel: bool,
}

/// Main udev backend data
pub struct UdevData {
    pub session: LibSeatSession,
    pub(super) dh: DisplayHandle,
    pub(super) dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    pub(super) syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    pub(super) primary_gpu: DrmNode,
    pub(super) gpus: GpuManager<GbmGlesBackend<SkiaRenderer, DrmDeviceFd>>,
    pub backends: HashMap<DrmNode, BackendData>,
    #[cfg(feature = "fps_ticker")]
    pub(super) fps_texture: Option<smithay::backend::renderer::multigpu::MultiTexture>,
    pub context_id: Option<ContextId<MultiTexture>>,
    /// Flag set by `request_redraw` to trigger a render on next loop iteration.
    pub(super) render_requested: AtomicBool,
    /// Monotonic count of scene ticks that reported damage. The lay-rs
    /// damage flag is consumed by whichever output ticks first; surfaces
    /// compare `SurfaceData::rendered_damage_gen` against this to know a
    /// damage event happened that they haven't drawn yet. Engine damage is
    /// cleared only once every surface has caught up.
    pub(super) damage_generation: u64,
    /// Adaptive plane budget: bumped when the kernel reports a display
    /// FIFO underrun (the display engine starving on plane fetch — the
    /// affected pipe scans out solid garbage from mid-frame down).
    /// 0 = full plane use, 1 = no window promotion, 2 = no plane
    /// decomposition (full GPU composite). Sticky for the session:
    /// display bandwidth is shared, so the reduction applies globally.
    pub(super) underrun_penalty: u8,
    /// Last time `kick_screencast_outputs` forced a frame. Damage-driven
    /// rendering already feeds the screenshare blit during activity; the
    /// kick only keeps a *static* screen's stream alive, so it is
    /// rate-limited hard — each kick drops the primary swapchain, and at
    /// tick rate that reallocates a full-screen buffer per tick.
    pub(super) last_screencast_kick: Option<std::time::Instant>,
    /// Cursor position at the last kick. Cursor motion moves a hardware
    /// plane without damaging the scene, so it produces no renders — and
    /// with cursor_mode=embedded the remote feed only shows the cursor
    /// where a blit drew it. A moved cursor therefore bypasses the kick
    /// rate limit or the remote pointer visibly staggers.
    pub(super) last_kick_cursor_pos: Option<smithay::utils::Point<f64, smithay::utils::Logical>>,
}

/// Per-device backend data
pub struct BackendData {
    pub(super) surfaces: HashMap<crtc::Handle, SurfaceData>,
    pub(super) non_desktop_connectors: Vec<(connector::Handle, crtc::Handle)>,
    pub(super) leasing_global: Option<DrmLeaseState>,
    pub(super) active_leases: Vec<smithay::wayland::drm_lease::DrmLease>,
    pub(super) gbm: GbmDevice<DrmDeviceFd>,
    pub drm: DrmDevice,
    pub(super) drm_scanner: DrmScanner,
    pub(super) render_node: DrmNode,
    pub(super) registration_token: RegistrationToken,
}

/// Per-surface rendering data
pub struct SurfaceData {
    pub(super) dh: DisplayHandle,
    pub(super) device_id: DrmNode,
    pub(super) render_node: DrmNode,
    pub(super) global: Option<GlobalId>,
    pub(super) compositor: GbmDrmCompositor,
    #[cfg(feature = "fps_ticker")]
    pub(super) fps: fps_ticker::Fps,
    #[cfg(feature = "fps_ticker")]
    pub(super) fps_element:
        Option<crate::drawing::FpsElement<smithay::backend::renderer::multigpu::MultiTexture>>,
    pub(super) dmabuf_feedback: Option<DrmSurfaceDmabufFeedback>,
    /// Rendering metrics
    #[cfg(feature = "metrics")]
    pub(super) render_metrics: Option<Arc<crate::render_metrics::RenderMetrics>>,
    /// Exponential moving average of render time in microseconds.
    /// Used to schedule reschedule timers with proper headroom.
    pub(super) avg_render_time_us: f32,
    /// Frames remaining before going idle. Reset on activity, counts down
    /// each no-damage frame so animations that briefly report zero pending
    /// transactions aren't cut short.
    pub(super) idle_countdown: u32,
    /// Whether this surface has ever submitted a frame. An output must
    /// always draw its first frame: the global scene-damage flag may have
    /// been consumed by another output's render before this surface gets
    /// its turn, and skipping here would leave the display black.
    pub(super) has_rendered_once: bool,
    /// Debug: whether this surface already honoured the current
    /// `/tmp/otto-full-redraw` trigger (reset when the file is removed).
    pub(super) full_redraw_done: bool,
    /// The damage generation this surface last rendered (see
    /// `UdevData::damage_generation`). A surface behind the global counter
    /// must render even when its own tick reports no damage — the damage
    /// was produced (and the flag consumed) on another output's tick.
    pub(super) rendered_damage_gen: u64,
    /// Whether the pointer was inside this output on the last drawn frame.
    /// When it leaves, one farewell frame must render without the cursor
    /// element — otherwise the hardware cursor plane keeps scanning out the
    /// stale cursor image at its last position on this output.
    pub(super) cursor_was_in_output: bool,
    /// Pre-computed scene-graph damage state for the upcoming draw phase.
    ///
    /// Frame pipelining splits each render cycle into two phases:
    ///
    /// 1. **Update (CPU)** – `scene_element.update()` is called at VBlank time
    ///    (inside `frame_finish`).  Because the GPU is still scanning out the
    ///    previous frame, the CPU and GPU overlap, maximising throughput.
    ///    The resulting damage flag is stored here.
    ///
    /// 2. **Draw (GPU)** – `render_surface` fires near the VBlank deadline.
    ///    It takes this pre-computed value instead of calling `update()` again,
    ///    so the critical path on the draw side is only GPU command recording
    ///    and page-flip submission.
    ///
    /// The field is `take`n at the start of every `render_surface` call.
    /// `None` means no update has been pre-computed yet (e.g. on the very
    /// first frame or after an idle wakeup); the draw phase falls back to
    /// calling `update()` inline in that case.
    pub(super) prefetched_scene_damage: Option<bool>,
    /// Background subtree rendered into its own dmabuf — the bottom of the
    /// plane stack, direct-scanned on the PRIMARY plane via
    /// `UnderlyingStorage::Dmabuf`. Allocated lazily on the first
    /// planes-mode render of this surface.
    pub(super) scene_dmabuf_element:
        Option<crate::render_elements::scene_dmabuf_element::SceneDmabufElement>,
    /// KMS plane for all workspace windows (overlay plane above the background).
    pub(super) windows_dmabuf_element:
        Option<crate::render_elements::scene_dmabuf_element::SceneDmabufElement>,
    /// KMS plane for the expose / window-selector view (overlay plane, mutually
    /// exclusive with windows_plane in practice — one is hidden when the other shows).
    pub(super) expose_dmabuf_element:
        Option<crate::render_elements::scene_dmabuf_element::SceneDmabufElement>,
    /// KMS plane for overlay UI: workspace selector, layer shell, OSD, DnD,
    /// popups — chrome above windows that changes rarely.
    pub(super) overlay_dmabuf_element:
        Option<crate::render_elements::scene_dmabuf_element::SceneDmabufElement>,
    /// Strip-sized KMS plane for the app switcher (middle band), above the
    /// overlay plane. Pushed only while the switcher is alive.
    pub(super) switcher_dmabuf_element:
        Option<crate::render_elements::scene_dmabuf_element::SceneDmabufElement>,
    /// Strip-sized KMS plane for the dock (bottom band). Topmost plane.
    pub(super) dock_dmabuf_element:
        Option<crate::render_elements::scene_dmabuf_element::SceneDmabufElement>,
    /// Downscaled composite of the planes below the overlay-UI plane
    /// (bg + windows/expose), seeding cross-plane backdrop blur (dock
    /// vibrancy). Rebuilt only when a lower plane changes under the
    /// overlay's blur region.
    pub(super) backdrop_surface: Option<BackdropSurface>,
    /// Blurred *desktop-only* composite (bg + windows/expose). Handed to the
    /// dock and switcher planes, which must not show popups in their vibrancy.
    pub(super) backdrop_image: Option<layers::skia::Image>,
    /// Blurred composite of the desktop PLUS the popup subtree, for the overlay
    /// plane. Popups stack in that plane, so a submenu must blur the popup(s)
    /// beneath it: we draw the popups onto the unblurred desktop cache and blur
    /// the whole image before it is seeded, so the blur happens before any
    /// per-popup clip (no faded edge rim, like the islands). Falls back to
    /// `backdrop_image` when there are no popups.
    pub(super) backdrop_overlay_image: Option<layers::skia::Image>,
    /// The *unblurred* desktop composite (same content as `backdrop_image`
    /// before the blur, popups excluded). Handed to the overlay plane as the
    /// raw backdrop so its `blur_include_content` layers — stacked popups —
    /// blur this plus whatever the same pass already painted behind them (the
    /// menu a submenu overlaps). Without a raw copy the pre-blurred seed lands
    /// *behind* that same-pass content, leaving the parent menu sharp.
    pub(super) backdrop_raw_image: Option<layers::skia::Image>,
    /// Whether the backdrop images are already blurred — consumers seed them
    /// directly and skip their own shape-clipped blur (which would leave a rim).
    pub(super) backdrop_preblurred: bool,
    /// Lower-plane damage occurred while no blur consumer needed the
    /// composite (or outside every active consumer's region); the next
    /// frame with an active consumer must rebuild even without new damage.
    pub(super) backdrop_dirty: bool,
    /// Last frame the expose / switcher / overlay UI was active — drives
    /// releasing their swapchains after prolonged inactivity
    /// (see `planes::maybe_release_plane`).
    pub(super) expose_last_active: Option<std::time::Instant>,
    pub(super) switcher_last_active: Option<std::time::Instant>,
    pub(super) overlay_last_active: Option<std::time::Instant>,
    /// Whether the overlay / switcher UI was active on the previous frame.
    /// On the inactive→active edge the plane's buffer still shows whatever
    /// was rendered before it left the frame (removal damage was cleared
    /// while it sat out), so the edge forces a full re-render instead of
    /// flashing ghost content (`SceneDmabufElement::request_full_render`).
    pub(super) overlay_was_active: bool,
    pub(super) switcher_was_active: bool,
    /// Whether the windows plane has already been warmed for the current
    /// expose session. While expose is up the windows buffer is rendered but
    /// never pushed as a plane element, so re-rendering it per frame blocks
    /// the CPU on a GPU sync for pixels that never reach the screen (measured
    /// at ~109 ms per second of expose). One render on the entry edge keeps
    /// the warm expose→windows transition; reset when expose ends.
    pub(super) windows_warmed_for_expose: bool,
    /// Last `PopupOverlayView::teardown_generation()` this surface has drawn.
    /// A popup teardown removes nodes that painted outside the bounds damage
    /// is derived from (drop shadow, blur rim), so the frame after a teardown
    /// redraws the overlay plane in full and rebuilds the backdrop instead of
    /// trusting partial damage — otherwise faint marks survive in the plane
    /// buffer (and in the popup-bearing backdrop) where the popup used to be.
    pub(super) popup_teardown_seen: usize,
    /// Same, for the dock's own context menu — it lives in the dock plane's
    /// subtree, so its teardown forces a full redraw of that plane.
    pub(super) dock_menu_teardown_seen: usize,
    /// Promotion hysteresis: the candidate set currently waiting out its
    /// stability window, and since when it has been produced unchanged.
    /// Demotions apply instantly (compositing is always correct); adding a
    /// window to the scanout set waits until the same candidates have been
    /// requested continuously for `PROMOTE_STABLE` — a one-frame eligibility
    /// pulse (activation animation, transient tooltip) otherwise thrashes
    /// promote/demote, and every transition resets the primary swapchain
    /// (a visible full-screen flicker).
    pub(super) promote_candidates: Vec<smithay::reexports::wayland_server::backend::ObjectId>,
    pub(super) promote_since: Option<std::time::Instant>,
    /// Whether the previous frame rendered as a forced full-GPU composite
    /// (minimize genie). Composite frames starve the plane buffers — the
    /// scene element consumes and clears all engine damage — so on the
    /// composite→planes edge every plane must redraw in full or the first
    /// planes frame scans out pre-composite ghosts (e.g. the window that
    /// was just minimized).
    pub(super) was_force_composite: bool,
    /// Keep rendering forced-composite frames until this instant even after
    /// the trigger (minimize animation) ended: the settle work (reparent,
    /// rescale, unhide) lands from an async task over several engine
    /// updates, and returning to planes mid-settle scans out a stale frame.
    pub(super) composite_hold_until: Option<std::time::Instant>,
    /// Debug (`/tmp/otto-dump-transition`): dump every plane buffer for this
    /// many frames after the composite→planes edge, to catch transition
    /// ghosts frame-exactly.
    pub(super) transition_dump_left: u8,
    /// Which element set the previous frame was built from. The compositor
    /// swapchain is reset on transitions so stale buffer ages don't leak
    /// across the mode switch.
    pub(super) last_frame_mode: FrameMode,
    /// Whether this output uses the per-purpose plane decomposition.
    /// Requires an atomic driver, enough overlay planes for the scene
    /// buffers, and the primary GPU (cross-device EGL import of the plane
    /// dmabufs is unreliable). When false the output renders as a single
    /// scene element — the plane path would pay all its intermediate
    /// renders and then GPU-composite every buffer anyway.
    pub(super) planes_enabled: bool,
    /// Windows currently in shadow-only mode for direct surface scanout.
    /// Their `content_layer` is hidden in lay-rs so only the shadow renders in
    /// `windows_dmabuf_element`. The client buffer is pushed directly as a
    /// `ScanoutCandidate` render element on the plane above.
    pub(super) shadow_only_windows: Vec<smithay::reexports::wayland_server::backend::ObjectId>,
    /// Window that was fullscreen direct-scanned-out on the previous frame.
    /// Fullscreen scanout never renders the window into the scene, so when it
    /// ends (e.g. an expose gesture) the window is re-imported like a demotion
    /// so the composited scene / expose mirror have real content to show.
    pub(super) last_fullscreen_scanout:
        Option<smithay::reexports::wayland_server::backend::ObjectId>,
    /// Deferred GPU sync point from the previous frame.
    ///
    /// Instead of blocking immediately after `render_frame()`, we store the
    /// fence here and wait for it at the **start** of the next `render_surface()`
    /// call.  This lets the GPU finish rendering in parallel with the CPU work
    /// that happens between frames (scene-graph update, input processing, etc.).
    #[cfg(feature = "renderer_sync")]
    pub(super) pending_gpu_fence: SyncPoint,
}

impl Drop for SurfaceData {
    fn drop(&mut self) {
        if let Some(global) = self.global.take() {
            self.dh
                .remove_global::<crate::state::Otto<UdevData>>(global);
        }
    }
}

/// Dmabuf feedback for a DRM surface (render vs scanout)
pub struct DrmSurfaceDmabufFeedback {
    pub render_feedback: DmabufFeedback,
    pub scanout_feedback: DmabufFeedback,
}

/// Error type for device addition
#[derive(Debug, thiserror::Error)]
pub enum DeviceAddError {
    #[error("Failed to open device using libseat: {0}")]
    DeviceOpen(smithay::backend::session::libseat::Error),
    #[error("Failed to initialize drm device: {0}")]
    DrmDevice(smithay::backend::drm::DrmError),
    #[error("Failed to initialize gbm device: {0}")]
    GbmDevice(std::io::Error),
    #[error("Failed to access drm node: {0}")]
    DrmNode(smithay::backend::drm::CreateDrmNodeError),
    #[error("Failed to add device to GpuManager: {0}")]
    AddNode(smithay::backend::egl::Error),
}

/// Skia GPU surface + context for the cross-plane backdrop composite.
/// Confined to the single render thread (same discipline as the slot
/// surfaces inside `SceneDmabufElement`), hence the manual Send/Sync.
pub(super) struct BackdropSurface {
    pub(super) surface: layers::skia::Surface,
    pub(super) context: layers::skia::gpu::DirectContext,
}
// SAFETY: accessed only from the render thread; never aliased across threads.
unsafe impl Send for BackdropSurface {}
unsafe impl Sync for BackdropSurface {}

/// Which element set a frame is built from. Buffer ages recorded in one
/// mode are meaningless in another: without a swapchain reset on
/// transition, regions the damage tracker considers clean would show the
/// other mode's stale content (e.g. a black background on the first
/// screencopy composite after running on planes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FrameMode {
    /// Per-purpose plane elements (normal desktop).
    Planes,
    /// Direct scanout of client buffers (fullscreen or promoted windows).
    DirectScanout,
    /// Single full-scene GPU composite (screencopy capture).
    Composite,
}

/// Outcome of a render operation
pub struct RenderOutcome {
    pub rendered: bool,
    /// Damage regions from the render.
    pub damage: Option<Vec<Rectangle<i32, Physical>>>,
}

impl RenderOutcome {
    pub fn skipped() -> Self {
        Self {
            rendered: false,
            damage: None,
        }
    }
}
