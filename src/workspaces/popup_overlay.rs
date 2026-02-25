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
        }
    }

    /// Get or create a popup layer for the given popup surface
    pub fn get_or_create_popup_layer(
        &mut self,
        popup_id: ObjectId,
        root_window_id: ObjectId,
        _warm_cache: Option<HashMap<String, std::collections::VecDeque<layers::prelude::NodeRef>>>,
    ) -> &mut PopupLayer {
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

                let content_layer = self.layers_engine.new_layer();
                content_layer.set_layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    ..Default::default()
                });
                content_layer.set_pointer_events(false);

                self.layers_engine.append_layer(&layer, self.layer.id());
                self.layers_engine.append_layer(&content_layer, layer.id());

                PopupLayer {
                    popup_id,
                    root_window_id,
                    layer,
                    content_layer,
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

        // Account for the layer's size and anchor point when positioning
        // The position parameter represents where we want the top-left corner to be,
        // but set_position places the layer's anchor point at that position.
        // So we need to offset by (size * anchor_point) to get the correct visual position.
        let anchor_point = popup.layer.anchor_point();
        let size = popup.layer.render_size();
        let adjusted_position = Point {
            x: position.x + (size.x * anchor_point.x),
            y: position.y + (size.y * anchor_point.y),
        };
        popup.layer.set_position(adjusted_position, None);

        // Map surface IDs to their layers
        let mut surface_layers: HashMap<ObjectId, Layer> = HashMap::new();

        for wvs in surfaces.iter() {
            if wvs.phy_dst_w <= 0.0 || wvs.phy_dst_h <= 0.0 {
                continue;
            }

            // Reuse layer from cache if it exists, otherwise create new one
            let layer = if let Some(cached_layer) = existing_surface_layers.get(&wvs.id) {
                cached_layer.clone()
            } else {
                let new_layer = layers_engine.new_layer();
                let key = format!("surface_{:?}", wvs.id);
                new_layer.set_key(&key);
                new_layer
            };

            // Configure layer with all properties and draw callback
            crate::workspaces::utils::configure_surface_layer(&layer, wvs);

            // Set up parent-child relationship
            if let Some(ref parent_id) = wvs.parent_id {
                if let Some(parent_layer) = surface_layers.get(parent_id) {
                    layers_engine.append_layer(&layer, parent_layer.id());
                } else {
                    // Parent not yet created, append to content layer
                    layers_engine.append_layer(&layer, popup.content_layer.id());
                }
            } else {
                // Root surface, append to content layer
                layers_engine.append_layer(&layer, popup.content_layer.id());
            }

            surface_layers.insert(wvs.id.clone(), layer);
        }

        surface_layers
    }

    /// Remove a popup layer
    pub fn remove_popup(&mut self, popup_id: &ObjectId) {
        if let Some(popup) = self.popup_layers.remove(popup_id) {
            popup.layer.remove();
        }
    }

    /// Remove all popups belonging to a specific root window
    pub fn remove_popups_for_window(&mut self, root_window_id: &ObjectId) -> Vec<ObjectId> {
        let to_remove: Vec<ObjectId> = self
            .popup_layers
            .iter()
            .filter(|(_, popup)| &popup.root_window_id == root_window_id)
            .map(|(id, _)| id.clone())
            .collect();

        for id in to_remove.iter() {
            self.remove_popup(id);
        }

        to_remove
    }

    /// Clear all popup layers
    pub fn clear(&mut self) {
        for (_, popup) in self.popup_layers.drain() {
            popup.layer.remove();
        }
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
    pub fn hide_popups_for_window(&self, root_window_id: &ObjectId) {
        for popup in self.popup_layers.values() {
            if &popup.root_window_id == root_window_id {
                popup.layer.set_hidden(true);
            }
        }
    }

    /// Show all popups belonging to a specific root window
    pub fn show_popups_for_window(&self, root_window_id: &ObjectId) {
        for popup in self.popup_layers.values() {
            if &popup.root_window_id == root_window_id {
                popup.layer.set_hidden(false);
            }
        }
    }
}
