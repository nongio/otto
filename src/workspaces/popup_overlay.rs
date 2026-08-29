use layers::{
    engine::Engine,
    prelude::{taffy, Layer},
    types::Point,
};
use smithay::reexports::wayland_server::backend::ObjectId;
use std::{collections::HashMap, sync::Arc};

use crate::workspaces::WindowViewSurface;

/// A popup with its layer and root window reference
pub struct PopupLayer {
    pub popup_id: ObjectId,
    pub root_window_id: ObjectId,
    pub layer: Layer,
    pub content_layer: Layer,
    /// Surface IDs whose layers live under this popup's content_layer.
    /// Used to clean up `surface_layers` when the popup is destroyed.
    surface_ids: Vec<ObjectId>,
    /// Position last handed to [`PopupOverlayView::update_popup`], before the
    /// anchor adjustment. Drives [`PopupOverlayView::needs_sync`] and keeps
    /// `set_position` — which lay-rs applies without comparing — from dirtying
    /// the layer when the popup has not moved.
    last_position: Option<Point>,
}

/// View for rendering popups on top of all windows
///
/// Popups (menus, dropdowns, tooltips) need to be rendered above all windows
/// to prevent clipping when they extend beyond their parent window bounds.
pub struct PopupOverlayView {
    pub layer: Layer,
    layers_engine: Arc<Engine>,
    /// Map from popup surface ID to its layer
    popup_layers: HashMap<ObjectId, PopupLayer>,
    /// Bumped every time a popup layer leaves the scene. A popup paints past
    /// the bounds its damage is derived from (drop shadow, blur rim), so the
    /// plane pipeline watches this counter and redraws the overlay plane in
    /// full after a teardown instead of trusting partial damage.
    teardown_generation: usize,
    /// Bumped on every STRUCTURAL popup change: a popup mapping, unmapping,
    /// becoming visible, or moving. Repaints of an already-placed popup do not
    /// count. The backdrop rebuild uses this to tell "a popup appeared, the
    /// blur under it must be right *now*" from "the popup redrew", so only the
    /// former is allowed to bypass the rebuild rate limit — see
    /// `udev::backdrop::decide_rebuild`.
    structure_generation: usize,
}

impl PopupOverlayView {
    pub fn new(layers_engine: Arc<Engine>) -> Self {
        let layer = layers_engine.new_layer();
        layer.set_key("popup_overlay");
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        layer.set_pointer_events(false);

        Self {
            layer,
            layers_engine,
            popup_layers: HashMap::new(),
            teardown_generation: 0,
            structure_generation: 0,
        }
    }

    /// Get or create a popup layer for the given popup surface
    pub fn get_or_create_popup_layer(
        &mut self,
        popup_id: ObjectId,
        root_window_id: ObjectId,
        _warm_cache: Option<HashMap<String, std::collections::VecDeque<layers::prelude::NodeRef>>>,
    ) -> &mut PopupLayer {
        if !self.popup_layers.contains_key(&popup_id) {
            // A popup entering the scene is structural.
            self.structure_generation = self.structure_generation.wrapping_add(1);
        }
        self.popup_layers
            .entry(popup_id.clone())
            .or_insert_with(|| {
                let layer = self.layers_engine.new_layer();
                layer.set_key(format!("popup_{:?}", popup_id));
                layer.set_layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    ..Default::default()
                });
                layer.set_pointer_events(false);
                layer.set_picture_cached(true);

                let content_layer = self.layers_engine.new_layer();
                content_layer.set_layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    ..Default::default()
                });
                content_layer.set_pointer_events(false);
                content_layer.set_picture_cached(true);

                // Start hidden — update_popup unhides once the popup has
                // renderable content, so the first visible frame is already at
                // the correct position. (Gating on xdg initial_configure data
                // instead left non-xdg popup types permanently invisible.)
                layer.set_hidden(true);

                let _ = self.layers_engine.append_layer(&layer, self.layer.id());
                let _ = self.layers_engine.append_layer(&content_layer, layer.id());

                PopupLayer {
                    popup_id,
                    root_window_id,
                    layer,
                    content_layer,
                    surface_ids: Vec::new(),
                    last_position: None,
                }
            })
    }

    /// Update popup position and surfaces
    #[allow(clippy::mutable_key_type)]
    #[allow(clippy::too_many_arguments)]
    pub fn update_popup(
        &mut self,
        popup_id: &ObjectId,
        root_window_id: &ObjectId,
        position: Point,
        surfaces: Vec<WindowViewSurface>,
        warm_cache: Option<HashMap<String, std::collections::VecDeque<layers::prelude::NodeRef>>>,
        layers_engine: &Arc<Engine>,
        existing_surface_layers: &HashMap<ObjectId, Layer>,
    ) -> HashMap<ObjectId, Layer> {
        let popup =
            self.get_or_create_popup_layer(popup_id.clone(), root_window_id.clone(), warm_cache);

        let anchor_point = popup.layer.anchor_point();
        let size = popup.layer.render_size();
        // Snapped for the same reason a window container is: the popup's
        // position is a logical integer multiplied by the output scale (and
        // then offset by the anchor over a fractional `render_size`), so on a
        // fractional scale it lands mid-pixel and offsets the whole popup
        // subtree — every surface under it is resampled, including a client
        // buffer that matches the output exactly. A tooltip is a single
        // surface of text, so the blur is obvious. See `snap_position_px`.
        let adjusted_position = crate::workspaces::utils::snap_position_px(
            (position.x + (size.x * anchor_point.x)) as f64,
            (position.y + (size.y * anchor_point.y)) as f64,
        );
        // Only write when it actually moved: lay-rs applies `set_position`
        // without comparing the value, so an unconditional write marks the
        // node NEEDS_LAYOUT and turns every parent-window commit into popup
        // damage. Compared against what the layer already holds rather than
        // against `last_position`, so a re-derived `adjusted_position` (the
        // anchor is applied over a `render_size` that is 0 until the first
        // layout) still lands. See `needs_sync`.
        let applied = popup.layer.position();
        let moved = applied.x != adjusted_position.x || applied.y != adjusted_position.y;
        if moved {
            popup.layer.set_position(adjusted_position, None);
        }
        popup.last_position = Some(position);

        let mut surface_layers: HashMap<ObjectId, Layer> = HashMap::new();
        let mut new_surface_ids: Vec<ObjectId> = Vec::new();

        for wvs in surfaces.iter() {
            if wvs.phy_dst_w <= 0.0 || wvs.phy_dst_h <= 0.0 {
                continue;
            }

            // Reuse layer from cache if it exists and alive, otherwise create new one
            let layer = if let Some(cached_layer) = existing_surface_layers.get(&wvs.id) {
                if layers_engine.is_layer_alive(&cached_layer.id()) {
                    cached_layer.clone()
                } else {
                    let new_layer = layers_engine.new_layer();
                    let key = format!("surface_{:?}", wvs.id);
                    new_layer.set_key(&key);
                    new_layer
                }
            } else {
                let new_layer = layers_engine.new_layer();
                let key = format!("surface_{:?}", wvs.id);
                new_layer.set_key(&key);
                new_layer
            };

            crate::workspaces::utils::configure_surface_layer(
                &layer,
                wvs,
                crate::surface_style::ContentsGravity::Resize,
                false,
                None,
            );

            // Popups stack in one plane, so a blurred popup overlaps content
            // painted earlier in the same pass (the menu a submenu opened from).
            // Seeding the pre-blurred backdrop puts it *behind* that content,
            // which then shows through sharp — opt into the raw backdrop plus a
            // real blur so same-pass content below is blurred too. No-op on
            // layers that aren't `BackgroundBlur`.
            layer.set_blur_include_content(true);

            if let Some(ref parent_id) = wvs.parent_id {
                if let Some(parent_layer) = surface_layers.get(parent_id) {
                    let _ = layers_engine.append_layer(&layer, parent_layer.id());
                } else {
                    let _ = layers_engine.append_layer(&layer, popup.content_layer.id());
                }
            } else {
                let _ = layers_engine.append_layer(&layer, popup.content_layer.id());
            }

            new_surface_ids.push(wvs.id.clone());
            surface_layers.insert(wvs.id.clone(), layer);
        }

        popup.surface_ids = new_surface_ids;

        // First frame with actual content: the position set above is now the
        // real one (clients only attach buffers after the initial configure),
        // so the popup can become visible without a mispositioned first frame.
        let became_visible = !popup.surface_ids.is_empty() && popup.layer.hidden();
        if !popup.surface_ids.is_empty() {
            popup.layer.set_hidden(false);
        }
        if moved || became_visible {
            self.structure_generation = self.structure_generation.wrapping_add(1);
        }

        surface_layers
    }

    /// Whether this popup's layer tree has to be re-synced at all.
    ///
    /// The surface sync runs per window: a commit on ANY surface of a window
    /// walks that window's popups too. Re-syncing a popup that neither moved
    /// nor redrew costs a surface-tree walk and — before the guards in
    /// `update_popup` and `configure_surface_layer` — invented damage that
    /// drove a full backdrop rebuild. `popup_committed` is true when the
    /// commit that triggered this sync belongs to this popup's own surface
    /// tree; otherwise only a move (or a popup that has not been placed yet)
    /// needs work.
    pub fn needs_sync(&self, popup_id: &ObjectId, position: Point, popup_committed: bool) -> bool {
        if popup_committed {
            return true;
        }
        match self.popup_layers.get(popup_id) {
            // Not placed yet (no buffer, so still hidden): keep trying.
            Some(popup) if !popup.surface_ids.is_empty() => popup
                .last_position
                .is_none_or(|p| p.x != position.x || p.y != position.y),
            _ => true,
        }
    }

    /// Remove a popup layer, returning surface IDs that need cleanup from surface_layers
    pub fn remove_popup(&mut self, popup_id: &ObjectId) -> Vec<ObjectId> {
        if let Some(popup) = self.popup_layers.remove(popup_id) {
            tracing::debug!(target: "otto::popups", "remove_popup {:?}", popup_id);
            popup.layer.remove();
            self.teardown_generation = self.teardown_generation.wrapping_add(1);
            self.structure_generation = self.structure_generation.wrapping_add(1);
            popup.surface_ids
        } else {
            Vec::new()
        }
    }

    /// Remove all popups belonging to a specific root window
    /// Returns all surface IDs (popup + subsurface) that need cleanup
    pub fn remove_popups_for_window(&mut self, root_window_id: &ObjectId) -> Vec<ObjectId> {
        let to_remove: Vec<ObjectId> = self
            .popup_layers
            .iter()
            .filter(|(_, popup)| &popup.root_window_id == root_window_id)
            .map(|(id, _)| id.clone())
            .collect();

        let mut all_surface_ids = Vec::new();
        for id in to_remove.iter() {
            let surface_ids = self.remove_popup(id);
            all_surface_ids.extend(surface_ids);
        }

        all_surface_ids
    }

    /// Clear all popup layers
    pub fn clear(&mut self) {
        for (_, popup) in self.popup_layers.drain() {
            popup.layer.remove();
            self.teardown_generation = self.teardown_generation.wrapping_add(1);
            self.structure_generation = self.structure_generation.wrapping_add(1);
        }
    }

    /// Counter of popup teardowns so far — see `teardown_generation`.
    pub fn teardown_generation(&self) -> usize {
        self.teardown_generation
    }

    /// Counter of structural popup changes so far — see `structure_generation`.
    pub fn structure_generation(&self) -> usize {
        self.structure_generation
    }

    /// How many popups are currently in the scene. Non-zero means the plane
    /// hosting them carries `blur_include_content` layers, whose blur samples
    /// content painted earlier in the same pass — see the full-render guard in
    /// the udev render loop.
    pub fn popup_count(&self) -> usize {
        self.popup_layers.len()
    }

    /// Get a popup layer by ID
    pub fn get_popup(&self, popup_id: &ObjectId) -> Option<&PopupLayer> {
        self.popup_layers.get(popup_id)
    }

    /// Show or hide the popup overlay layer
    pub fn set_hidden(&self, hidden: bool) {
        self.layer.set_hidden(hidden);
    }

    /// Hide all popups belonging to a specific root window
    pub fn hide_popups_for_window(&mut self, root_window_id: &ObjectId) {
        self.structure_generation = self.structure_generation.wrapping_add(1);
        for popup in self.popup_layers.values() {
            if &popup.root_window_id == root_window_id {
                tracing::debug!(
                    target: "otto::popups",
                    "hide popup {:?} (root {:?})",
                    popup.popup_id,
                    root_window_id
                );
                popup.layer.set_hidden(true);
            }
        }
    }

    /// Show all popups belonging to a specific root window
    pub fn show_popups_for_window(&mut self, root_window_id: &ObjectId) {
        self.structure_generation = self.structure_generation.wrapping_add(1);
        for popup in self.popup_layers.values() {
            if &popup.root_window_id == root_window_id {
                tracing::debug!(
                    target: "otto::popups",
                    "show popup {:?} (root {:?})",
                    popup.popup_id,
                    root_window_id
                );
                popup.layer.set_hidden(false);
            }
        }
    }
}
