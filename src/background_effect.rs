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

use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};
use smithay::reexports::wayland_server::{
    backend::{GlobalId, ObjectId},
    protocol::wl_surface::WlSurface,
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
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
    /// The protocol clips the region to the surface, but since lay-rs frosts
    /// the whole layer, all that matters is whether the region has area: a
    /// client that adds a rectangle and then subtracts all of it is treated
    /// as asking for blur, which is a corner no real client is in.
    pub fn wants_blur(&self) -> bool {
        self.blur_region.as_ref().is_some_and(|region| {
            region.rects.iter().any(|(kind, rect)| {
                matches!(kind, RectangleKind::Add) && rect.size.w > 0 && rect.size.h > 0
            })
        })
    }
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
        let wants_blur = with_states(surface, |states| {
            states
                .cached_state
                .get::<BackgroundEffectCachedState>()
                .current()
                .wants_blur()
        });
        let was_blurred = self
            .background_effects
            .get(&surface_id)
            .copied()
            .unwrap_or(false);
        if wants_blur == was_blurred {
            return;
        }

        let Some(layer) = self.surface_layers.get(&surface_id).cloned() else {
            // No layer yet — an unmapped surface. The commit that maps it
            // comes back through here with the same committed region.
            return;
        };

        if wants_blur {
            self.background_effects.insert(surface_id.clone(), true);
            layer.set_blend_mode(layers::types::BlendMode::BackgroundBlur);
            // What the frost has to show is usually the window *below* in
            // the same plane, not just the wallpaper: read the raw backdrop
            // and blur it here, as the SSD titlebar and styled windows do.
            layer.set_blur_include_content(true);
        } else {
            self.background_effects.remove(&surface_id);
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
