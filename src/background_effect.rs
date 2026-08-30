//! `ext-background-effect-v1`: a client asks for the pixels behind its
//! surface to be blurred.
//!
//! The protocol is tiny — one global that reports what the compositor can do
//! (blur, here), and one object per `wl_surface` carrying a double-buffered
//! `wl_region`. The region is the part of the surface whose background the
//! client wants frosted; `NULL` (or never set) means none. Terminals with a
//! translucent background (foot, wezterm, ghostty) and panels are the
//! intended users.
//!
//! Otto already knows how to frost a surface: `otto-surface-style`'s
//! `BackgroundBlur` blend mode sets the surface's own scene layer to
//! [`layers::types::BlendMode::BackgroundBlur`], and everything downstream —
//! the plane backdrop seeding, keeping such a window off a raw scanout plane
//! through [`WindowElement::has_material`](crate::shell::WindowElement::has_material)
//! — keys off that. This module maps the protocol onto that same path, so a
//! foot window blurs exactly like an otto-kit popup does.
//!
//! What the compositor does with the region's *shape* is its own policy. lay-rs
//! blurs a layer's whole (rounded) bounds, so a non-empty region frosts the
//! entire surface and an empty one nothing: the region is a switch, its
//! rectangles are not honoured individually. Every client seen so far asks
//! for the full surface anyway.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};
use smithay::reexports::wayland_server::{
    backend::{GlobalId, ObjectId},
    protocol::wl_surface::WlSurface,
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::{
    get_region_attributes, with_states, Cacheable, RectangleKind, RegionAttributes,
};

use crate::{state::Backend, Otto};

/// The blur region a client committed, double-buffered by smithay alongside
/// the rest of the surface state so it lands on `wl_surface.commit` — and,
/// for a synchronized subsurface, on the parent's.
#[derive(Debug, Default, Clone)]
pub struct BackgroundEffectCachedState {
    /// `None` is "no blur": never set, set to `NULL`, or the effect object
    /// was destroyed.
    pub blur_region: Option<RegionAttributes>,
}

impl Cacheable for BackgroundEffectCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

impl BackgroundEffectCachedState {
    /// Whether the committed region asks for any blur at all.
    ///
    /// A client that adds a rectangle and then subtracts all of it is treated
    /// as asking for blur, which is a corner no real client is in.
    pub fn wants_blur(&self) -> bool {
        self.blur_shape().is_some()
    }

    /// The committed region as a rounded rectangle in surface-local logical
    /// coordinates: the bounding box of its additive rectangles, plus the
    /// corner radius they describe.
    ///
    /// lay-rs frosts one rounded rect per layer, so an arbitrary region cannot
    /// be reproduced exactly — but the regions clients actually send are
    /// rounded rectangles, handed over as a stack of scanlines. Two things
    /// have to come back out of that stack:
    ///
    /// * The **bounds**, because the region is generally smaller than the
    ///   surface. A panel that draws its own drop shadow into a transparent
    ///   margin asks for the body only; frosting the whole surface blurs the
    ///   content behind the margin away and the shadow then falls on the
    ///   blurred smear instead of on the window below.
    /// * The **radius**, because the scanlines near the top are inset and a
    ///   square frost would show as blurred corners outside a rounded panel.
    ///   Each inset row is a point on the corner arc, so it pins the radius:
    ///   with the circle centred at `(r, r)` in the bounding box,
    ///   `(inset - r)^2 + (row - r)^2 = r^2` solves to
    ///   `r = row + inset + sqrt(2 * row * inset)`. One row is not enough —
    ///   the client rasterised the curve to integer rows, and near the ends of
    ///   the arc that rounding dominates — so every inset row votes and the
    ///   median wins. A region with no inset rows is a plain rectangle and
    ///   yields 0, which is the right answer for it.
    ///
    /// Subtractive rectangles are ignored: they can only carve the region
    /// smaller, and no client has been seen sending them here.
    pub fn blur_shape(&self) -> Option<(Rectangle<i32, Logical>, i32)> {
        let region = self.blur_region.as_ref()?;
        let adds = || {
            region.rects.iter().filter_map(|(kind, rect)| {
                (matches!(kind, RectangleKind::Add) && rect.size.w > 0 && rect.size.h > 0)
                    .then_some(*rect)
            })
        };

        let bounds = adds().reduce(|acc, rect| acc.merge(rect))?;

        let half_height = bounds.size.h as f64 / 2.0;
        let mut votes: Vec<f64> = adds()
            .filter_map(|rect| {
                let row = (rect.loc.y - bounds.loc.y) as f64;
                let inset = (rect.loc.x - bounds.loc.x) as f64;
                // Only the top corner, and only rows the arc actually bites
                // into: a flush row sits past the end of the curve and would
                // vote for its own offset rather than for the radius.
                (inset > 0.0 && row < half_height).then(|| row + inset + (2.0 * row * inset).sqrt())
            })
            .collect();

        let radius = if votes.is_empty() {
            0
        } else {
            votes.sort_by(|a, b| a.partial_cmp(b).expect("insets are finite"));
            votes[votes.len() / 2].round() as i32
        }
        .clamp(0, bounds.size.w.min(bounds.size.h) / 2);

        Some((bounds, radius))
    }
}

/// Trim a blur shape to the surface it belongs to.
///
/// "The whole surface" is spelled as an unbounded region — foot sends
/// `wl_region.add(0, 0, i32::MAX, i32::MAX)` — so the region cannot be taken at
/// its word. Nothing outside the surface may be frosted anyway: the bounds go
/// to lay-rs as their own rounded rect rather than being clipped to the layer,
/// so an oversized one frosts a rectangle of desktop beside the window.
///
/// A surface with no size yet has no buffer, so there is nothing to trim
/// against and nothing on screen to get wrong; the next commit brings both.
fn clamp_to_surface(
    shape: Option<(Rectangle<i32, Logical>, i32)>,
    surface_size: Option<smithay::utils::Size<i32, Logical>>,
) -> Option<(Rectangle<i32, Logical>, i32)> {
    let (bounds, radius) = shape?;
    let Some(size) = surface_size else {
        return Some((bounds, radius));
    };
    let clamped = bounds
        .intersection(Rectangle::from_size(size))
        .unwrap_or_default();
    // The radius was clamped to the region's own half-extent; the trimmed
    // shape may be smaller.
    let radius = radius.clamp(0, clamped.size.w.min(clamped.size.h) / 2);
    Some((clamped, radius))
}

/// Per-surface marker: the protocol allows one effect object per surface and
/// makes a second `get_background_effect` a protocol error.
#[derive(Debug, Default)]
struct BackgroundEffectSurfaceMarker {
    taken: AtomicBool,
}

/// User data of an `ext_background_effect_surface_v1`.
#[derive(Debug)]
pub struct BackgroundEffectSurfaceUserData {
    /// The surface the effect belongs to. A `WlSurface` handle does not keep
    /// the surface alive, so `is_alive` is the "inert" check the protocol
    /// asks for once the client destroys the surface first.
    surface: WlSurface,
}

/// The `ext_background_effect_manager_v1` global.
#[derive(Debug, Clone)]
pub struct BackgroundEffectState {
    global: GlobalId,
}

impl BackgroundEffectState {
    /// Create and advertise the global.
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ExtBackgroundEffectManagerV1, ()>
            + Dispatch<ExtBackgroundEffectManagerV1, ()>
            + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceUserData>
            + 'static,
    {
        let global = display.create_global::<D, ExtBackgroundEffectManagerV1, _>(1, ());
        Self { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

impl<BackendData: Backend + 'static> GlobalDispatch<ExtBackgroundEffectManagerV1, ()>
    for Otto<BackendData>
{
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        // Sent on bind and whenever it changes; Otto's blur is always on.
        manager.capabilities(ext_background_effect_manager_v1::Capability::Blur);
    }
}

impl<BackendData: Backend + 'static> Dispatch<ExtBackgroundEffectManagerV1, ()>
    for Otto<BackendData>
{
    fn request(
        _state: &mut Self,
        _client: &Client,
        manager: &ExtBackgroundEffectManagerV1,
        request: ext_background_effect_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } => {
                let already = with_states(&surface, |states| {
                    states
                        .data_map
                        .insert_if_missing_threadsafe(BackgroundEffectSurfaceMarker::default);
                    states
                        .data_map
                        .get::<BackgroundEffectSurfaceMarker>()
                        .unwrap()
                        .taken
                        .swap(true, Ordering::AcqRel)
                });
                if already {
                    manager.post_error(
                        ext_background_effect_manager_v1::Error::BackgroundEffectExists,
                        "the surface already has a background effect object",
                    );
                    return;
                }
                data_init.init(id, BackgroundEffectSurfaceUserData { surface });
            }
            ext_background_effect_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<BackendData: Backend + 'static>
    Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceUserData> for Otto<BackendData>
{
    fn request(
        _state: &mut Self,
        _client: &Client,
        effect: &ExtBackgroundEffectSurfaceV1,
        request: ext_background_effect_surface_v1::Request,
        data: &BackgroundEffectSurfaceUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { region } => {
                if !data.surface.is_alive() {
                    effect.post_error(
                        ext_background_effect_surface_v1::Error::SurfaceDestroyed,
                        "set_blur_region on an effect whose surface was destroyed",
                    );
                    return;
                }
                // Copy semantics: the client may destroy the wl_region right
                // after this request, so take the rectangles now.
                let attributes = region.as_ref().map(get_region_attributes);
                with_states(&data.surface, |states| {
                    states
                        .cached_state
                        .get::<BackgroundEffectCachedState>()
                        .pending()
                        .blur_region = attributes;
                });
            }
            ext_background_effect_surface_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        _effect: &ExtBackgroundEffectSurfaceV1,
        data: &BackgroundEffectSurfaceUserData,
    ) {
        if !data.surface.is_alive() {
            return;
        }
        // "The effect regions will be removed on the next commit" — clear
        // the pending state rather than the live one, and let the surface
        // ask again later.
        with_states(&data.surface, |states| {
            states
                .cached_state
                .get::<BackgroundEffectCachedState>()
                .pending()
                .blur_region = None;
            if let Some(marker) = states.data_map.get::<BackgroundEffectSurfaceMarker>() {
                marker.taken.store(false, Ordering::Release);
            }
        });
    }
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Apply the blur region a surface just committed to its scene layer.
    ///
    /// Runs from `CompositorHandler::commit`, after the commit has built or
    /// refreshed the surface's layer, so the layer is there to set. Cheap on
    /// the common path: the last applied value is remembered per surface in
    /// [`Otto::background_effects`] and nothing is touched when it has not
    /// changed — `set_blend_mode` on every commit would damage the layer.
    ///
    /// Only turns a blur *off* that it turned on: a surface that also holds
    /// an `otto-surface-style` `BackgroundBlur` keeps it, that style owns
    /// the layer's blend mode too.
    pub(crate) fn apply_background_effect(&mut self, surface: &WlSurface) {
        let surface_id = surface.id();
        let (shape, surface_size) = with_states(surface, |states| {
            let shape = states
                .cached_state
                .get::<BackgroundEffectCachedState>()
                .current()
                .blur_shape();
            // The surface's own logical extent, from the same place
            // `window_view_for_surface` reads it.
            let size = states
                .data_map
                .get::<RendererSurfaceStateUserData>()
                .and_then(|data| data.lock().unwrap().view().map(|view| view.dst));
            (shape, size)
        });
        let shape = clamp_to_surface(shape, surface_size);
        let applied = self.background_effects.get(&surface_id).copied();
        // Compared as a shape, not as a flag: a client that moves or resizes
        // the region it wants frosted (a candidate panel growing a row) keeps
        // `wants_blur` true throughout, and bailing on that would leave the
        // frost at the old geometry.
        if shape == applied {
            return;
        }

        let Some(layer) = self.surface_layers.get(&surface_id).cloned() else {
            // No layer yet — an unmapped surface. The commit that maps it
            // comes back through here with the same committed region.
            return;
        };

        if let Some((bounds, radius)) = shape {
            self.background_effects
                .insert(surface_id.clone(), (bounds, radius));
            layer.set_blend_mode(layers::types::BlendMode::BackgroundBlur);
            // What the frost has to show is usually the window *below* in
            // the same plane, not just the wallpaper: read the raw backdrop
            // and blur it here, as the SSD titlebar and styled windows do.
            layer.set_blur_include_content(true);
            // The region is in surface-local logical coordinates and the layer
            // is in physical pixels, so it scales the same way the surface's
            // own geometry does in `configure_surface_layer`.
            let scale = crate::config::Config::with(|c| c.screen_scale) as f32;
            let rect = layers::skia::Rect::from_xywh(
                bounds.loc.x as f32 * scale,
                bounds.loc.y as f32 * scale,
                bounds.size.w as f32 * scale,
                bounds.size.h as f32 * scale,
            );
            let r = radius as f32 * scale;
            layer.set_blur_bounds(Some(layers::skia::RRect::new_rect_xy(rect, r, r)));
        } else {
            self.background_effects.remove(&surface_id);
            layer.set_blur_bounds(None);
            let style_blurs = self
                .surfaces_style
                .get(&surface_id)
                .is_some_and(|styles| styles.iter().any(|s| s.background_blur));
            if !style_blurs {
                layer.set_blend_mode(layers::types::BlendMode::default());
                layer.set_blur_include_content(false);
            }
        }

        self.refresh_window_material(&surface_id);
        if let Some(window) = self.workspaces.get_window_for_surface(&surface_id).cloned() {
            self.update_window_view(&window);
        }
    }

    /// Forget a destroyed surface's blur.
    pub(crate) fn forget_background_effect(&mut self, surface_id: &ObjectId) {
        self.background_effects.remove(surface_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Point;

    fn region(rects: Vec<(RectangleKind, Rectangle<i32, Logical>)>) -> BackgroundEffectCachedState {
        BackgroundEffectCachedState {
            blur_region: Some(RegionAttributes { rects }),
        }
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), (w, h).into())
    }

    /// A rounded rectangle reaches clients as a stack of scanlines whose top
    /// rows are inset. The first row flush with the left edge sits at `y =
    /// radius`, which is what `blur_shape` reads back out.
    fn rounded_scanlines(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
    ) -> Vec<(RectangleKind, Rectangle<i32, Logical>)> {
        (0..h)
            .map(|row| {
                let from_edge = row.min(h - 1 - row);
                let inset = if from_edge >= radius {
                    0
                } else {
                    // circular corner: horizontal inset at this row
                    let dy = (radius - from_edge) as f64;
                    (radius as f64 - ((radius as f64).powi(2) - dy * dy).sqrt()).round() as i32
                };
                (
                    RectangleKind::Add,
                    rect(x + inset, y + row, w - 2 * inset, 1),
                )
            })
            .collect()
    }

    #[test]
    fn no_region_is_no_blur() {
        let state = BackgroundEffectCachedState::default();
        assert_eq!(state.blur_shape(), None);
        assert!(!state.wants_blur());
    }

    #[test]
    fn empty_region_is_no_blur() {
        let state = region(vec![]);
        assert_eq!(state.blur_shape(), None);
        assert!(!state.wants_blur());
    }

    #[test]
    fn zero_area_rectangles_are_no_blur() {
        let state = region(vec![
            (RectangleKind::Add, rect(0, 0, 0, 40)),
            (RectangleKind::Add, rect(0, 0, 100, 0)),
        ]);
        assert_eq!(state.blur_shape(), None);
    }

    #[test]
    fn a_plain_rectangle_keeps_square_corners() {
        let state = region(vec![(RectangleKind::Add, rect(0, 0, 200, 40))]);
        assert_eq!(state.blur_shape(), Some((rect(0, 0, 200, 40), 0)));
    }

    /// The reported bug: a panel that draws its own drop shadow asks for the
    /// body only, so the frost must stop short of the surface on every side.
    #[test]
    fn an_inset_region_keeps_its_offset() {
        let state = region(vec![(RectangleKind::Add, rect(6, 6, 188, 28))]);
        let (bounds, _) = state.blur_shape().unwrap();
        assert_eq!(bounds, rect(6, 6, 188, 28));
    }

    #[test]
    fn scanlines_recover_the_corner_radius() {
        for radius in [4, 8, 10, 16] {
            let state = region(rounded_scanlines(6, 6, 200, 60, radius));
            let (bounds, r) = state.blur_shape().unwrap();
            assert_eq!(bounds, rect(6, 6, 200, 60), "bounds for radius {radius}");
            assert_eq!(r, radius, "radius {radius}");
        }
    }

    /// lay-rs draws one rounded rect, so a radius past half the shorter side
    /// would be nonsense geometry rather than a tighter curve.
    #[test]
    fn radius_is_clamped_to_the_shape() {
        let state = region(vec![
            (RectangleKind::Add, rect(0, 30, 100, 10)),
            (RectangleKind::Add, rect(20, 0, 60, 40)),
        ]);
        let (bounds, r) = state.blur_shape().unwrap();
        assert_eq!(bounds, rect(0, 0, 100, 40));
        assert_eq!(r, 20);
    }

    /// The regression the frost beside a foot window came from: foot asks for
    /// the whole surface with an unbounded region, and lay-rs blurs whatever
    /// rect it is given whether or not the layer is that big.
    #[test]
    fn an_unbounded_region_is_trimmed_to_the_surface() {
        let state = region(vec![(RectangleKind::Add, rect(0, 0, i32::MAX, i32::MAX))]);
        let shape = clamp_to_surface(state.blur_shape(), Some((800, 500).into()));
        assert_eq!(shape, Some((rect(0, 0, 800, 500), 0)));
    }

    /// A panel that asks for its body only keeps asking for its body.
    #[test]
    fn a_region_inside_the_surface_is_left_alone() {
        let shape = clamp_to_surface(Some((rect(6, 6, 188, 28), 8)), Some((400, 100).into()));
        assert_eq!(shape, Some((rect(6, 6, 188, 28), 8)));
    }

    /// Trimming can leave a shape too small for the radius the region asked
    /// for, and a radius past half the shorter side is nonsense geometry.
    #[test]
    fn trimming_tightens_the_radius() {
        let shape = clamp_to_surface(Some((rect(0, 0, 200, 200), 40)), Some((200, 50).into()));
        assert_eq!(shape, Some((rect(0, 0, 200, 50), 25)));
    }

    /// Before the first buffer there is nothing to trim against, and nothing
    /// on screen to get wrong either.
    #[test]
    fn a_surface_with_no_size_yet_is_not_trimmed() {
        let shape = clamp_to_surface(Some((rect(0, 0, 10, 10), 2)), None);
        assert_eq!(shape, Some((rect(0, 0, 10, 10), 2)));
    }

    #[test]
    fn subtractive_rectangles_do_not_grow_the_bounds() {
        let state = region(vec![
            (RectangleKind::Add, rect(10, 10, 50, 20)),
            (RectangleKind::Subtract, rect(0, 0, 500, 500)),
        ]);
        let (bounds, _) = state.blur_shape().unwrap();
        assert_eq!(bounds, rect(10, 10, 50, 20));
    }
}
