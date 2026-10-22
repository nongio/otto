//! The scale each surface is drawn at, and the buffer that scale asks for.
//!
//! Every surface renders at the scale the compositor prefers *for that
//! surface* (`wp_fractional_scale_v1.preferred_scale`), so the compositor can
//! show its buffer pixel for pixel: no resampling on the way to the screen,
//! which is what font hinting and subpixel text depend on, and what keeps text
//! sharp at 1x and at fractional scales alike. A surface on a 1.5x output gets
//! a buffer one and a half times its logical size, shown at its logical size
//! through a `wp_viewport`; one on a 2x output gets a plain `buffer_scale = 2`.
//!
//! The scale is not fixed when the surface is made. The compositor sends a new
//! preferred scale when the surface moves to an output of another scale or the
//! output's scale is changed, and the surface's next draw resizes its buffer
//! and re-rasters everything in it.
//!
//! What is kept here is what the compositor *said*. Which buffer a surface
//! actually has is the surface's own business — see
//! `BaseWaylandSurface::sync_scale`.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};

use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

/// What the compositor has said about one surface's scale. 0 is "nothing yet"
/// in both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Reported {
    /// `wp_fractional_scale_v1.preferred_scale`, in 120ths.
    preferred_120: u32,
    /// The integer scale SCTK derives from the outputs the surface is on (or
    /// `wl_surface.preferred_buffer_scale`).
    integer: i32,
}

/// Keyed by `wl_surface`. Behind a mutex rather than in a thread-local because
/// [`global_scale_120`] is read from wherever a draw closure runs.
static SURFACES: LazyLock<Mutex<HashMap<ObjectId, Reported>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The scale most recently *changed to* on any surface, in 120ths: what
/// [`global_scale_120`] answers for code that has no surface in hand. Moved
/// only by a change, not by every `preferred_scale`: the compositor repeats
/// the current value each frame, and with surfaces on two outputs a value that
/// followed every event would flip between their scales sixty times a second.
static LAST_PREFERRED_120: AtomicU32 = AtomicU32::new(0);

/// The integer counterpart, from `scale_factor_changed`.
static LAST_INTEGER: AtomicI32 = AtomicI32::new(0);

/// The first output's scale in 120ths, read from `wl_output` and `xdg_output`
/// at startup, before the app makes a surface. The stand-in for a surface
/// whose own preferred scale has not arrived yet.
static OUTPUT_SEED_120: AtomicU32 = AtomicU32::new(0);

/// Whether the compositor offers `wp_fractional_scale_manager_v1`. With it, a
/// surface's integer scale is only ever a rounded-up stand-in (2 on a 1.5x
/// output) and must not displace the seed; without it, the integer scale is
/// all there is and is more specific than the seed.
static HAS_FRACTIONAL_MANAGER: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The scale of the surface whose draw is running, in 120ths; 0 outside
    /// one. See [`DrawingScope`].
    static DRAWING_120: Cell<u32> = const { Cell::new(0) };
}

/// Resolve what is known about a surface into the scale it draws at, in 120ths.
fn resolve(reported: Reported, seed_120: u32, fractional_manager: bool) -> u32 {
    if reported.preferred_120 > 0 {
        return reported.preferred_120;
    }
    let integer = (reported.integer > 0).then_some(reported.integer as u32 * 120);
    let seed = (seed_120 > 0).then_some(seed_120);
    let fallback = if fractional_manager {
        seed.or(integer)
    } else {
        integer.or(seed)
    };
    fallback.unwrap_or(120)
}

pub(crate) fn set_fractional_manager(present: bool) {
    HAS_FRACTIONAL_MANAGER.store(present, Relaxed);
}

pub(crate) fn seed_output_scale_120(scale_120: u32) {
    OUTPUT_SEED_120.store(scale_120, Relaxed);
}

/// Record a change to `surface`'s report, and say whether the scale it draws
/// at moved because of it.
fn note(surface: &ObjectId, update: impl FnOnce(&mut Reported)) -> Option<u32> {
    let seed = OUTPUT_SEED_120.load(Relaxed);
    let manager = HAS_FRACTIONAL_MANAGER.load(Relaxed);
    let mut surfaces = SURFACES.lock().unwrap_or_else(|e| e.into_inner());
    let reported = surfaces.entry(surface.clone()).or_default();
    let before = resolve(*reported, seed, manager);
    update(reported);
    let after = resolve(*reported, seed, manager);
    (before != after).then_some(after)
}

/// The compositor's `preferred_scale` for `surface`. Returns the new scale
/// when the surface's scale changed, `None` when it was the one it already had
/// — the compositor resends it every frame.
pub(crate) fn note_preferred_120(surface: &ObjectId, scale_120: u32) -> Option<u32> {
    let changed = note(surface, |r| r.preferred_120 = scale_120);
    if changed.is_some() || LAST_PREFERRED_120.load(Relaxed) == 0 {
        LAST_PREFERRED_120.store(scale_120, Relaxed);
    }
    changed
}

/// SCTK's integer scale for `surface`. As [`note_preferred_120`].
pub(crate) fn note_integer(surface: &ObjectId, factor: i32) -> Option<u32> {
    LAST_INTEGER.store(factor, Relaxed);
    note(surface, |r| r.integer = factor)
}

/// Drop what was said about a surface that is being destroyed.
pub(crate) fn forget(surface: &ObjectId) {
    SURFACES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(surface);
}

/// The scale `surface` draws at, in 120ths: its preferred scale, else a
/// stand-in until that arrives (the startup output scale, the integer surface
/// scale), else 1.
pub(crate) fn surface_scale_120(surface: &ObjectId) -> u32 {
    let reported = SURFACES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(surface)
        .copied()
        .unwrap_or_default();
    resolve(
        reported,
        OUTPUT_SEED_120.load(Relaxed),
        HAS_FRACTIONAL_MANAGER.load(Relaxed),
    )
}

/// The scale for code that has no surface in hand, in 120ths, or `None` when
/// nothing at all is known — a process with no connection, such as the
/// compositor drawing kit components for itself.
///
/// Inside a surface's draw it is that surface's scale. Otherwise it is the
/// scale the compositor most recently moved one of the app's surfaces to,
/// then the startup output scale, then the last integer scale.
pub(crate) fn global_scale_120() -> Option<u32> {
    let drawing = DRAWING_120.with(Cell::get);
    [
        drawing,
        LAST_PREFERRED_120.load(Relaxed),
        OUTPUT_SEED_120.load(Relaxed),
        LAST_INTEGER.load(Relaxed).max(0) as u32 * 120,
    ]
    .into_iter()
    .find(|&scale| scale > 0)
}

/// Marks a surface's draw as running at `scale_120` for as long as it lives,
/// so [`global_scale_120`] — and every component that rasterises from
/// `AppContext::scale_factor` — answers for the surface being drawn rather
/// than for whichever one changed last. Nests: the previous value comes back
/// on drop.
pub(crate) struct DrawingScope(u32);

impl DrawingScope {
    pub(crate) fn enter(scale_120: u32) -> Self {
        Self(DRAWING_120.with(|d| d.replace(scale_120)))
    }
}

impl Drop for DrawingScope {
    fn drop(&mut self) {
        DRAWING_120.with(|d| d.set(self.0));
    }
}

/// How a surface's buffer carries its scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferScale {
    /// `wl_surface.set_buffer_scale(n)`: a buffer `n` times the logical size,
    /// and no viewport. Whole scales, and every scale when the compositor has
    /// no `wp_viewporter` to show a fractional buffer with.
    Integer(i32),
    /// A buffer `round(logical × scale)` pixels at `buffer_scale = 1`, with a
    /// `wp_viewport` destination of the logical size. In 120ths.
    Fractional(u32),
}

impl BufferScale {
    /// The buffer for a surface drawn at `scale_120`.
    ///
    /// Without a viewporter a fractional scale rounds *up*: the compositor
    /// then shrinks a slightly larger buffer, which reads better than
    /// stretching a smaller one.
    pub fn for_scale(scale_120: u32, viewporter: bool) -> Self {
        let scale_120 = scale_120.max(1);
        if scale_120.is_multiple_of(120) {
            Self::Integer((scale_120 / 120) as i32)
        } else if viewporter {
            Self::Fractional(scale_120)
        } else {
            Self::Integer(scale_120.div_ceil(120) as i32)
        }
    }

    /// Buffer pixels per logical point: what the canvas is scaled by.
    pub fn factor(self) -> f32 {
        match self {
            Self::Integer(n) => n as f32,
            Self::Fractional(s) => s as f32 / 120.0,
        }
    }

    /// The value for `wl_surface.set_buffer_scale`.
    pub fn wl_buffer_scale(self) -> i32 {
        match self {
            Self::Integer(n) => n,
            Self::Fractional(_) => 1,
        }
    }

    /// The buffer size, in pixels, for a surface `logical` points big.
    ///
    /// A fractional size rounds to the nearest pixel, which is the size the
    /// compositor's own rounding of `logical × scale` lands on. Never empty:
    /// a surface laid out to nothing still gets a pixel, so a crop of it has
    /// a buffer to stay inside.
    pub fn buffer_size(self, (width, height): (i32, i32)) -> (i32, i32) {
        match self {
            Self::Integer(n) => (width.max(1) * n, height.max(1) * n),
            Self::Fractional(s) => {
                let s = f64::from(s) / 120.0;
                (
                    ((f64::from(width) * s).round() as i32).max(1),
                    ((f64::from(height) * s).round() as i32).max(1),
                )
            }
        }
    }
}

/// A crop: `source`, in the surface's own points, shown at `destination`
/// points. See `SubsurfaceSurface::set_view`.
pub type View = (skia_safe::Rect, (i32, i32));

/// What a surface's `wp_viewport` should hold. Both `None` unsets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportState {
    /// `set_source`, in the surface-local coordinates the protocol takes —
    /// buffer pixels divided by the buffer scale.
    pub source: Option<(f64, f64, f64, f64)>,
    /// `set_destination`, in logical points.
    pub destination: Option<(i32, i32)>,
}

impl ViewportState {
    pub const UNSET: Self = Self {
        source: None,
        destination: None,
    };

    /// The viewport for a surface `logical` points big with a `buffer` the
    /// given size, cropped to `crop` if it has one.
    ///
    /// A crop is given in points; at a fractional scale the buffer is in
    /// pixels at `buffer_scale = 1`, so the source is mapped through the
    /// buffer's own size rather than the scale — exact at the far edge, where
    /// `logical × scale` can round past the buffer — and clamped to it, since
    /// a source that leaves the buffer is a protocol error. So is an empty
    /// one: a crop that is empty, or past the buffer's edge because the
    /// layout is ahead of the buffer drawn for it, keeps one buffer pixel.
    pub fn for_surface(
        scale: BufferScale,
        logical: (i32, i32),
        buffer: (i32, i32),
        crop: Option<View>,
    ) -> Self {
        let Some((source, (dest_w, dest_h))) = crop else {
            return match scale {
                BufferScale::Integer(_) => Self::UNSET,
                BufferScale::Fractional(_) => Self {
                    source: None,
                    destination: Some((logical.0.max(1), logical.1.max(1))),
                },
            };
        };
        // Surface-local extent of the buffer, points to that space, and one
        // buffer pixel in it.
        let (extent, per_point, pixel) = match scale {
            BufferScale::Integer(n) => {
                let n = f64::from(n.max(1));
                (
                    (f64::from(buffer.0) / n, f64::from(buffer.1) / n),
                    (1.0, 1.0),
                    1.0 / n,
                )
            }
            BufferScale::Fractional(_) => (
                (f64::from(buffer.0), f64::from(buffer.1)),
                (
                    f64::from(buffer.0) / f64::from(logical.0.max(1)),
                    f64::from(buffer.1) / f64::from(logical.1.max(1)),
                ),
                1.0,
            ),
        };
        let (left, width) = clamp_span(
            f64::from(source.left) * per_point.0,
            f64::from(source.right) * per_point.0,
            extent.0,
            pixel,
        );
        let (top, height) = clamp_span(
            f64::from(source.top) * per_point.1,
            f64::from(source.bottom) * per_point.1,
            extent.1,
            pixel,
        );
        Self {
            source: Some((left, top, width, height)),
            destination: Some((dest_w.max(1), dest_h.max(1))),
        }
    }
}

/// `start..end` clamped to `0..extent` and at least `pixel` long, as
/// `(start, length)`.
fn clamp_span(start: f64, end: f64, extent: f64, pixel: f64) -> (f64, f64) {
    let pixel = pixel.min(extent);
    let start = start.clamp(0.0, extent - pixel);
    let end = end.clamp(start + pixel, extent);
    (start, end - start)
}

/// The buffer geometry a surface was last given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferGeometry {
    /// How the buffer carries its scale.
    pub scale: BufferScale,
    /// The buffer's size in pixels.
    pub buffer: (i32, i32),
    /// What the surface's viewport holds.
    pub viewport: ViewportState,
}

/// What a [`ScaledBuffer::sync`] changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScaleSync {
    /// The buffer is a new size: nothing drawn in the old one can be kept.
    pub buffer: bool,
    /// The scale moved: everything rasterised for the surface is stale.
    pub scale: bool,
}

/// The scale half of a surface: its buffer scale and its `wp_viewport`,
/// kept in line with the scale the surface is drawn at.
///
/// Every otto-kit surface has one inside it. A client that drives its own
/// `wl_surface` — one that paints `wl_shm` buffers, say — uses this directly:
/// it listens for the surface's `wp_fractional_scale_v1.preferred_scale`
/// itself, and asks [`Self::sync`] for the buffer to paint before each frame.
/// The buffer it gets is then shown pixel for pixel.
///
/// What is sent to the surface describes a buffer, and the compositor
/// checks it against the buffer a commit carries: a viewport source outside
/// that buffer, or a buffer scale its size is no multiple of, is a protocol
/// error. So state for a buffer of a new size, or a new scale, waits in
/// `wanted` until [`Self::flush`], which the draw calls just before the
/// buffer of that size is attached. A commit in between — a crop or a
/// position sent without a repaint — carries the old buffer with the state
/// that was sent for it. A crop that keeps the buffer's size and scale is
/// sent at once: it is clamped to the buffer the surface already shows.
#[derive(Debug)]
pub struct ScaledBuffer {
    surface: WlSurface,
    viewport: Option<WpViewport>,
    /// What the next buffer is to be: the last [`Self::sync`]'s answer.
    wanted: Option<BufferGeometry>,
    /// What the surface was last sent, with a buffer of that size.
    sent: Option<BufferGeometry>,
}

impl ScaledBuffer {
    pub fn new(surface: &WlSurface) -> Self {
        Self {
            surface: surface.clone(),
            viewport: None,
            wanted: None,
            sent: None,
        }
    }

    /// Work out the surface's buffer scale, buffer size and viewport for
    /// `scale_120`, a `logical` size in points and a `crop`, and say what
    /// changed. The buffer to paint is then [`Self::geometry`]'s; what the
    /// surface is sent waits for [`Self::flush`] unless the buffer keeps its
    /// size and scale.
    ///
    /// `viewporter` is what shows a fractional buffer at its logical size;
    /// without one, a fractional scale is drawn at the next whole one and
    /// left to the compositor to shrink.
    pub fn sync<D>(
        &mut self,
        scale_120: u32,
        logical: (i32, i32),
        crop: Option<View>,
        viewporter: Option<&WpViewporter>,
        qh: &QueueHandle<D>,
    ) -> ScaleSync
    where
        D: Dispatch<WpViewport, ()> + 'static,
    {
        let scale = BufferScale::for_scale(scale_120, viewporter.is_some());
        let buffer = scale.buffer_size(logical);
        let viewport = ViewportState::for_surface(scale, logical, buffer, crop);
        let next = BufferGeometry {
            scale,
            buffer,
            viewport,
        };
        let previous = self.wanted;
        if previous == Some(next) {
            return ScaleSync::default();
        }
        self.wanted = Some(next);
        if sendable_now(self.sent, next) {
            self.flush(viewporter, qh);
        }
        ScaleSync {
            buffer: previous.map(|p| p.buffer) != Some(buffer),
            scale: previous.is_some_and(|p| p.scale != scale),
        }
    }

    /// Send what the last [`Self::sync`] worked out, just before the buffer
    /// it describes is attached: the next commit has to carry a buffer of
    /// [`Self::geometry`]'s size. Sends only what moved.
    pub fn flush<D>(&mut self, viewporter: Option<&WpViewporter>, qh: &QueueHandle<D>)
    where
        D: Dispatch<WpViewport, ()> + 'static,
    {
        let Some(next) = self.wanted else {
            return;
        };
        if self.sent == Some(next) {
            return;
        }
        if self.sent.map(|s| s.scale) != Some(next.scale) {
            self.surface.set_buffer_scale(next.scale.wl_buffer_scale());
        }
        if self.sent.map(|s| s.viewport) != Some(next.viewport) {
            self.send_viewport(next.viewport, viewporter, qh);
        }
        self.sent = Some(next);
    }

    /// Send `state` to the viewport, making it if the surface has none yet
    /// and needs one.
    fn send_viewport<D>(
        &mut self,
        state: ViewportState,
        viewporter: Option<&WpViewporter>,
        qh: &QueueHandle<D>,
    ) where
        D: Dispatch<WpViewport, ()> + 'static,
    {
        if self.viewport.is_none() {
            if state == ViewportState::UNSET {
                return;
            }
            let Some(viewporter) = viewporter else {
                return;
            };
            self.viewport = Some(viewporter.get_viewport(&self.surface, qh, ()));
        }
        let Some(viewport) = self.viewport.as_ref().filter(|v| v.is_alive()) else {
            return;
        };
        // -1 unsets: the buffer's own size, at its buffer scale.
        let (x, y, w, h) = state.source.unwrap_or((-1.0, -1.0, -1.0, -1.0));
        viewport.set_source(x, y, w, h);
        let (w, h) = state.destination.unwrap_or((-1, -1));
        viewport.set_destination(w, h);
    }

    /// The buffer to paint: what the last [`Self::sync`] worked out, `None`
    /// before the first.
    pub fn geometry(&self) -> Option<BufferGeometry> {
        self.wanted
    }

    /// Forget what was worked out and sent, so the next [`Self::sync`] and
    /// [`Self::flush`] send everything again.
    pub fn reset(&mut self) {
        self.wanted = None;
        self.sent = None;
    }
}

/// Whether `next` can be sent while the surface still shows a buffer of
/// what was last `sent`: only when it is the same buffer, so the state is
/// checked against the buffer it was clamped to. A new size or scale waits
/// for the draw that attaches it.
fn sendable_now(sent: Option<BufferGeometry>, next: BufferGeometry) -> bool {
    sent.is_some_and(|sent| sent.scale == next.scale && sent.buffer == next.buffer)
}

impl Drop for ScaledBuffer {
    fn drop(&mut self) {
        if let Some(viewport) = self.viewport.take().filter(|v| v.is_alive()) {
            viewport.destroy();
        }
    }
}

/// The globals that scale a surface's buffer; either may be missing.
#[derive(Debug, Clone, Default)]
pub struct Scaling {
    pub fractional: Option<WpFractionalScaleManagerV1>,
    pub viewporter: Option<WpViewporter>,
}

/// The buffer to paint for a frame of a [`ScaledSurface`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// In pixels.
    pub size: (i32, i32),
    /// Pixels per logical point: what the canvas is scaled by.
    pub factor: f32,
}

/// A surface outside the kit's runtime that is drawn at its preferred scale:
/// a [`ScaledBuffer`] plus the `wp_fractional_scale_v1` that says which.
///
/// For a client that runs its own event queue and paints its own buffers
/// (`wl_shm`, typically). The surface's `preferred_scale` events arrive at
/// `D` with the surface as their user data; `D` hands them to
/// [`Self::set_preferred`] and redraws when that says the scale moved.
pub struct ScaledSurface<D> {
    fractional: Option<WpFractionalScaleV1>,
    scale_120: Option<u32>,
    stand_in_120: u32,
    buffer: ScaledBuffer,
    viewporter: Option<WpViewporter>,
    qh: QueueHandle<D>,
}

/// By hand: the derive would ask the client's state for `Debug` too.
impl<D> std::fmt::Debug for ScaledSurface<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScaledSurface")
            .field("scale_120", &self.scale_120)
            .field("stand_in_120", &self.stand_in_120)
            .field("buffer", &self.buffer)
            .finish_non_exhaustive()
    }
}

impl<D> ScaledSurface<D>
where
    D: Dispatch<WpFractionalScaleV1, WlSurface> + Dispatch<WpViewport, ()> + 'static,
{
    /// Ask for `surface`'s preferred scale. Until it arrives — or for good,
    /// without fractional scaling — the surface is drawn at `stand_in_120`.
    pub fn new(
        surface: &WlSurface,
        scaling: &Scaling,
        stand_in_120: u32,
        qh: &QueueHandle<D>,
    ) -> Self {
        Self {
            fractional: scaling
                .fractional
                .as_ref()
                .map(|manager| manager.get_fractional_scale(surface, qh, surface.clone())),
            scale_120: None,
            stand_in_120,
            buffer: ScaledBuffer::new(surface),
            viewporter: scaling.viewporter.clone(),
            qh: qh.clone(),
        }
    }

    /// The compositor's preferred scale arrived. Returns whether it is a
    /// change, which the surface has to be redrawn for: the compositor
    /// repeats an unchanged one every frame.
    pub fn set_preferred(&mut self, scale_120: u32) -> bool {
        self.scale_120.replace(scale_120) != Some(scale_120)
    }

    /// Physical pixels per logical point, for geometry given to the
    /// compositor in physical pixels.
    pub fn scale(&self) -> f64 {
        f64::from(self.scale_120.unwrap_or(self.stand_in_120)) / 120.0
    }

    /// The buffer to paint for a surface `logical` points big. Sets the
    /// surface's buffer scale and viewport to match, both landing with the
    /// commit that carries the buffer: the caller paints a buffer of this
    /// size and commits it next.
    pub fn frame(&mut self, logical: (i32, i32)) -> Frame {
        self.buffer.sync(
            self.scale_120.unwrap_or(self.stand_in_120),
            logical,
            None,
            self.viewporter.as_ref(),
            &self.qh,
        );
        self.buffer.flush(self.viewporter.as_ref(), &self.qh);
        let geometry = self
            .buffer
            .geometry()
            .expect("a synced buffer has a geometry");
        Frame {
            size: geometry.buffer,
            factor: geometry.scale.factor(),
        }
    }

    /// Let go of the protocol objects, before the surface is destroyed.
    pub fn destroy(self) {
        if let Some(fractional) = self.fractional.as_ref().filter(|f| f.is_alive()) {
            fractional.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process-wide statics are shared; tests that set them take turns,
    /// and put them back as a process with no connection has them, which is
    /// what every other test in the crate draws under.
    static STATICS: Mutex<()> = Mutex::new(());

    struct Clean(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    fn clear() {
        SURFACES.lock().unwrap_or_else(|e| e.into_inner()).clear();
        LAST_PREFERRED_120.store(0, Relaxed);
        LAST_INTEGER.store(0, Relaxed);
        OUTPUT_SEED_120.store(0, Relaxed);
        HAS_FRACTIONAL_MANAGER.store(false, Relaxed);
    }

    impl Drop for Clean {
        fn drop(&mut self) {
            clear();
        }
    }

    fn reset() -> Clean {
        let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        Clean(guard)
    }

    fn reported(preferred_120: u32, integer: i32) -> Reported {
        Reported {
            preferred_120,
            integer,
        }
    }

    /// Before its preferred scale arrives a surface draws at a stand-in. With
    /// fractional scaling on offer the integer scale (2 on a 1.5x output) must
    /// not displace the startup output scale; without it, the surface's own
    /// integer scale is the more specific answer.
    #[test]
    fn stand_in_scale_until_the_preferred_one_arrives() {
        assert_eq!(resolve(reported(0, 2), 180, true), 180);
        assert_eq!(resolve(reported(0, 2), 120, false), 240);
        assert_eq!(resolve(reported(0, 0), 180, false), 180);
        assert_eq!(resolve(reported(0, 2), 0, true), 240);
        assert_eq!(
            resolve(reported(0, 0), 0, true),
            120,
            "nothing known is 1x, not a guessed 2x that doubles every panel"
        );
        assert_eq!(resolve(reported(150, 2), 240, true), 150);
    }

    /// A live scale change reaches the surface — no latch — and is reported
    /// once: the compositor resending the same value every frame is not a
    /// change, and must not re-raster anything.
    #[test]
    fn preferred_scale_follows_changes_and_reports_them_once() {
        let _guard = reset();
        let surface = ObjectId::null();
        OUTPUT_SEED_120.store(180, Relaxed);

        assert_eq!(
            note_preferred_120(&surface, 180),
            None,
            "the seed was right"
        );
        assert_eq!(note_preferred_120(&surface, 240), Some(240));
        assert_eq!(note_preferred_120(&surface, 240), None);
        assert_eq!(surface_scale_120(&surface), 240);
        assert_eq!(global_scale_120(), Some(240));
    }

    /// Code without a surface asks inside a draw and gets the drawing
    /// surface's scale, not the app-wide one.
    #[test]
    fn a_draw_answers_for_its_own_surface() {
        let _guard = reset();
        assert_eq!(global_scale_120(), None);
        OUTPUT_SEED_120.store(240, Relaxed);
        {
            let _scope = DrawingScope::enter(120);
            assert_eq!(global_scale_120(), Some(120));
        }
        assert_eq!(global_scale_120(), Some(240));
    }

    #[test]
    fn buffer_for_each_kind_of_scale() {
        assert_eq!(BufferScale::for_scale(240, true), BufferScale::Integer(2));
        assert_eq!(BufferScale::for_scale(120, false), BufferScale::Integer(1));
        assert_eq!(
            BufferScale::for_scale(180, true),
            BufferScale::Fractional(180)
        );
        assert_eq!(BufferScale::for_scale(150, false), BufferScale::Integer(2));

        assert_eq!(BufferScale::Integer(2).buffer_size((101, 33)), (202, 66));
        // 101 x 1.5 = 151.5 and 33 x 1.5 = 49.5: rounded, as the compositor
        // rounds the logical size it shows the buffer at.
        assert_eq!(
            BufferScale::Fractional(180).buffer_size((101, 33)),
            (152, 50)
        );
    }

    /// A fractional buffer is shown at its logical size; an integer one needs
    /// no viewport at all.
    #[test]
    fn uncropped_viewport() {
        assert_eq!(
            ViewportState::for_surface(BufferScale::Integer(2), (100, 50), (200, 100), None),
            ViewportState::UNSET
        );
        assert_eq!(
            ViewportState::for_surface(BufferScale::Fractional(180), (101, 33), (152, 50), None),
            ViewportState {
                source: None,
                destination: Some((101, 33)),
            }
        );
    }

    /// State for a buffer of another size, or at another scale, is not sent
    /// while the surface shows the old one: a commit without a repaint would
    /// land it on a buffer it does not fit (the compositor kills the client
    /// for a viewport source outside the buffer). A crop that keeps the
    /// buffer goes out at once.
    #[test]
    fn state_for_a_buffer_not_yet_attached_waits_for_the_draw() {
        let at = |scale, buffer, crop| BufferGeometry {
            scale,
            buffer,
            viewport: ViewportState::for_surface(scale, (100, 50), buffer, crop),
        };
        let crop = Some((skia_safe::Rect::from_ltrb(0.0, 0.0, 50.0, 50.0), (50, 50)));
        let shown = at(BufferScale::Fractional(150), (150, 75), None);

        assert!(!sendable_now(None, shown), "nothing attached yet");
        assert!(
            sendable_now(
                Some(shown),
                at(BufferScale::Fractional(150), (150, 75), crop)
            ),
            "a crop of the buffer on screen"
        );
        assert!(
            !sendable_now(
                Some(shown),
                at(BufferScale::Fractional(150), (151, 75), crop)
            ),
            "a resize: the buffer is not there yet"
        );
        assert!(
            !sendable_now(Some(shown), at(BufferScale::Integer(2), (200, 100), None)),
            "a scale change"
        );
    }

    /// A crop in points becomes buffer pixels at a fractional scale and stays
    /// inside the buffer even where `logical × scale` rounds past it.
    #[test]
    fn cropped_viewport_stays_inside_the_buffer() {
        let crop = (skia_safe::Rect::from_ltrb(0.0, 10.0, 101.0, 33.0), (80, 20));

        let integer =
            ViewportState::for_surface(BufferScale::Integer(2), (101, 33), (202, 66), Some(crop));
        assert_eq!(integer.source, Some((0.0, 10.0, 101.0, 23.0)));
        assert_eq!(integer.destination, Some((80, 20)));

        // 1.25x: 101 x 1.25 = 126.25 rounds down to 126, 33 x 1.25 = 41.25 to 41.
        let fractional = ViewportState::for_surface(
            BufferScale::Fractional(150),
            (101, 33),
            (126, 41),
            Some(crop),
        );
        let (x, y, w, h) = fractional.source.unwrap();
        assert_eq!((x, x + w), (0.0, 126.0));
        assert!((y - 10.0 * 41.0 / 33.0).abs() < 1e-9);
        assert_eq!(y + h, 41.0);
        assert_eq!(fractional.destination, Some((80, 20)));
    }

    /// A crop past the buffer's edge, or an empty one, still sends a source
    /// with a size: a zero-size source is a protocol error that disconnects
    /// the client.
    #[test]
    fn crop_outside_the_buffer_keeps_a_pixel() {
        let past = (
            skia_safe::Rect::from_ltrb(120.0, 60.0, 180.0, 90.0),
            (60, 30),
        );
        let empty = (skia_safe::Rect::from_ltrb(10.0, 10.0, 10.0, 10.0), (0, 0));

        let integer =
            ViewportState::for_surface(BufferScale::Integer(2), (100, 50), (200, 100), Some(past));
        assert_eq!(integer.source, Some((99.5, 49.5, 0.5, 0.5)));

        // A pane laid out to no height, cropped to nothing.
        let collapsed = BufferScale::Fractional(210);
        let buffer = collapsed.buffer_size((260, 0));
        let (_, _, w, h) = ViewportState::for_surface(collapsed, (260, 0), buffer, Some(empty))
            .source
            .unwrap();
        assert!(w > 0.0 && h > 0.0, "collapsed: {w}x{h}");

        for crop in [past, empty] {
            let (x, y, w, h) = ViewportState::for_surface(
                BufferScale::Fractional(150),
                (100, 50),
                (125, 63),
                Some(crop),
            )
            .source
            .unwrap();
            assert!(w >= 1.0 && h >= 1.0, "{crop:?}: {w}x{h}");
            assert!(x >= 0.0 && x + w <= 125.0 && y >= 0.0 && y + h <= 63.0);
        }
    }
}
