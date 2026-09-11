//! What the compositor can put behind a translucent surface.
//!
//! The toolkit's materials are translucent by design: they are meant to tint a
//! blurred backdrop, not the bare pixels behind a window. Otto blurs through
//! `otto_surface_style_v1`. Other compositors — KWin among them — offer the
//! standard `ext_background_effect_v1`, which blurs a region of a surface and
//! does nothing else: no tint, no rounding, no shadow. A compositor with
//! neither has nothing to be translucent over, and the materials are filled in
//! instead (see [`crate::theme::Theme::light`]).
//!
//! Decided once, at connection time, like the display scale: a surface picks
//! its material when it is created and does not re-derive it later.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

use wayland_client::backend::ObjectId;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

use crate::app_runner::AppContext;

/// Where a translucent material gets its blur from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// Otto's `otto_surface_style_v1`: blur, tint, corners and shadow.
    SurfaceStyle,
    /// The standard `ext_background_effect_v1`: blur behind a region, and
    /// nothing else.
    BackgroundEffect,
    /// No blur at all. Materials are opaque.
    None,
}

const UNKNOWN: u8 = 0;
const SURFACE_STYLE: u8 = 1;
const BACKGROUND_EFFECT: u8 = 2;
const NONE: u8 = 3;

static BACKDROP: AtomicU8 = AtomicU8::new(UNKNOWN);

/// The backdrop this client settled on when it connected.
///
/// Before a connection — unit tests, a theme built at startup — this answers
/// [`Backdrop::SurfaceStyle`], so nothing changes for code that never talks
/// to a compositor.
pub fn current() -> Backdrop {
    match BACKDROP.load(Ordering::Relaxed) {
        BACKGROUND_EFFECT => Backdrop::BackgroundEffect,
        NONE => Backdrop::None,
        _ => Backdrop::SurfaceStyle,
    }
}

/// Whether anything will blur behind a translucent material.
pub fn blur_available() -> bool {
    current() != Backdrop::None
}

/// Whether a subsurface can be frosted over its own window.
///
/// Only Otto's surface style blurs what is directly under a subsurface. The
/// standard background effect blurs behind the *window*: KWin composites the
/// blur first and the whole surface tree over it, so a translucent card on a
/// subsurface shows the window's own content through it, sharp.
pub fn blurs_behind_subsurfaces() -> bool {
    current() == Backdrop::SurfaceStyle
}

/// Pick the backdrop from what the compositor offers.
///
/// `OTTO_KIT_BACKDROP=effect|none` steps down from what is there, to look at a
/// fallback on a compositor that would otherwise not use it. It never steps
/// up: asking for `effect` where the global is missing still gets `none`.
fn resolve(surface_style: bool, effect_blur: bool, requested: Option<&str>) -> Backdrop {
    let offered = if surface_style {
        Backdrop::SurfaceStyle
    } else if effect_blur {
        Backdrop::BackgroundEffect
    } else {
        Backdrop::None
    };
    match requested {
        Some("none") => Backdrop::None,
        Some("effect") if offered == Backdrop::SurfaceStyle => {
            if effect_blur {
                Backdrop::BackgroundEffect
            } else {
                Backdrop::None
            }
        }
        _ => offered,
    }
}

/// Collects the manager's capabilities during the startup roundtrip.
#[derive(Default)]
pub(crate) struct Probe {
    blur: bool,
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            state.blur = match flags {
                WEnum::Value(flags) => {
                    flags.contains(ext_background_effect_manager_v1::Capability::Blur)
                }
                WEnum::Unknown(bits) => bits & 1 != 0,
            };
        }
    }
}

/// What the connection holds on to: the manager, and the queue its events
/// arrive on, kept alive so a later capability change has somewhere to land.
pub(crate) struct Binding {
    pub manager: ExtBackgroundEffectManagerV1,
    _queue: EventQueue<Probe>,
}

/// Bind `ext_background_effect_manager_v1` and settle [`current`].
///
/// The manager is bound on a queue of its own so its capabilities can be read
/// with a roundtrip before the application is ready, without dispatching
/// anything of the application's early.
pub(crate) fn init(
    conn: &Connection,
    globals: &GlobalList,
    surface_style: bool,
) -> Option<Binding> {
    let mut queue = conn.new_event_queue::<Probe>();
    let qh = queue.handle();
    let manager = globals
        .bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, ())
        .ok();
    // The capabilities are sent on bind; one roundtrip is enough to have them.
    let mut probe = Probe::default();
    if manager.is_some() {
        let _ = queue.roundtrip(&mut probe);
    }
    let requested = std::env::var("OTTO_KIT_BACKDROP").ok();
    let backdrop = resolve(surface_style, probe.blur, requested.as_deref());
    store(backdrop);
    tracing::debug!(?backdrop, "backdrop");
    manager.map(|manager| Binding {
        manager,
        _queue: queue,
    })
}

fn store(backdrop: Backdrop) {
    BACKDROP.store(
        match backdrop {
            Backdrop::SurfaceStyle => SURFACE_STYLE,
            Backdrop::BackgroundEffect => BACKGROUND_EFFECT,
            Backdrop::None => NONE,
        },
        Ordering::Relaxed,
    );
}

/// The part of a surface to blur behind, in surface-local points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlurShape {
    /// All of it. The compositor clips the region to the surface.
    Whole,
    /// A rounded rectangle — a card inside a surface that also carries its
    /// margin.
    RoundedRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
    },
}

impl BlurShape {
    /// The shape as the rectangles a `wl_region` is made of.
    ///
    /// A region cannot be round, so the corners are stepped a row at a time.
    /// Blurring a square corner would show as a faint frosted tab outside the
    /// card's painted rounding.
    pub fn rects(&self) -> Vec<(i32, i32, i32, i32)> {
        match *self {
            BlurShape::Whole => vec![(0, 0, i32::MAX / 2, i32::MAX / 2)],
            BlurShape::RoundedRect {
                x,
                y,
                width,
                height,
                radius,
            } => {
                let (x, y) = (x.round() as i32, y.round() as i32);
                let (w, h) = (width.round() as i32, height.round() as i32);
                let r = (radius.round() as i32).clamp(0, w.min(h) / 2);
                let mut rects = Vec::with_capacity(2 * r as usize + 1);
                for row in 0..r {
                    let dy = r as f32 - row as f32 - 0.5;
                    let inset = r - ((r * r) as f32 - dy * dy).max(0.0).sqrt().round() as i32;
                    let span = w - 2 * inset;
                    if span > 0 {
                        rects.push((x + inset, y + row, span, 1));
                        rects.push((x + inset, y + h - 1 - row, span, 1));
                    }
                }
                if h - 2 * r > 0 {
                    rects.push((x, y + r, w, h - 2 * r));
                }
                rects
            }
        }
    }
}

thread_local! {
    /// One effect object per surface — the protocol allows no more.
    static EFFECTS: RefCell<HashMap<ObjectId, ExtBackgroundEffectSurfaceV1>> =
        RefCell::new(HashMap::new());
}

/// Blur behind `shape` of `surface`, or stop blurring with `None`.
///
/// Only acts under [`Backdrop::BackgroundEffect`]; Otto's surfaces carry their
/// blur on the surface style. Double-buffered: it lands with the surface's
/// next commit, which is the caller's.
pub fn set_blur(surface: &WlSurface, shape: Option<BlurShape>) {
    if current() != Backdrop::BackgroundEffect {
        return;
    }
    let Some(manager) = AppContext::background_effect_manager() else {
        return;
    };
    let qh = AppContext::queue_handle();
    EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        effects.retain(|_, effect| effect.is_alive());
        let effect = effects
            .entry(surface.id())
            .or_insert_with(|| manager.get_background_effect(surface, qh, ()));
        match shape {
            Some(shape) => {
                let region = AppContext::compositor_state()
                    .wl_compositor()
                    .create_region(qh, ());
                for (x, y, w, h) in shape.rects() {
                    region.add(x, y, w, h);
                }
                effect.set_blur_region(Some(&region));
                region.destroy();
            }
            None => effect.set_blur_region(None),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_the_surface_style() {
        assert_eq!(resolve(true, true, None), Backdrop::SurfaceStyle);
        assert_eq!(resolve(false, true, None), Backdrop::BackgroundEffect);
        assert_eq!(resolve(false, false, None), Backdrop::None);
    }

    #[test]
    fn the_override_only_steps_down() {
        assert_eq!(
            resolve(true, true, Some("effect")),
            Backdrop::BackgroundEffect
        );
        assert_eq!(resolve(true, false, Some("effect")), Backdrop::None);
        assert_eq!(resolve(false, true, Some("none")), Backdrop::None);
        assert_eq!(resolve(false, false, Some("effect")), Backdrop::None);
        assert_eq!(
            resolve(false, true, Some("style")),
            Backdrop::BackgroundEffect
        );
    }

    #[test]
    fn a_rounded_region_stays_inside_its_card() {
        let shape = BlurShape::RoundedRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 60.0,
            radius: 14.0,
        };
        let rects = shape.rects();
        let area: i32 = rects.iter().map(|&(_, _, w, h)| w * h).sum();
        // Less than the square card by roughly the four corners' (1 - π/4)r².
        let square = 100 * 60;
        let corners = (4.0 * (1.0 - std::f32::consts::FRAC_PI_4) * 14.0 * 14.0) as i32;
        assert!((square - corners - area).abs() < 60, "area {area}");
        for &(x, y, w, h) in &rects {
            assert!(x >= 10 && y >= 20 && x + w <= 110 && y + h <= 80);
        }
        // The very first row is inset, the middle band is full width.
        assert!(rects[0].0 > 10);
        assert!(rects.iter().any(|&(x, _, w, _)| x == 10 && w == 100));
    }
}
