use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use layers::{
    engine::{animation::Transition, Engine},
    prelude::{taffy, Layer},
    taffy::style::Style,
    types::Size,
    view::{BuildLayerTree, LayerTreeBuilder},
};

use crate::workspaces::{
    dock::{
        draw_app_icon, draw_badge, draw_progress, setup_badge_layer, setup_progress_layer,
        BASE_ICON_SIZE,
    },
    Application,
};

struct AppIconEntry {
    pub stack: Layer,
    pub icon_layer: Layer,
    pub badge_layer: Layer,
    pub progress_layer: Layer,
    pub icon_id: Option<u32>,
}

/// Owns a persistent, hidden icon stack (icon + badge + progress) for every known app.
///
/// Both the dock and the app switcher hold mirror layers that replicate from these stacks.
/// Stacks are append-only — they are never freed, so `NodeRef`s pointing at them remain
/// valid for the lifetime of the compositor session.
pub struct AppIconsManager {
    engine: Arc<Engine>,
    /// Container for all icon stacks. Pointer events are disabled so it doesn't
    /// interfere with interaction, but it participates in layout so that
    /// `render_node_tree` can produce output for mirror followers.
    pub container: Layer,
    entries: RwLock<HashMap<String, AppIconEntry>>,
}

impl std::fmt::Debug for AppIconsManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppIconsManager")
            .field("container", &self.container)
            .finish()
    }
}

impl AppIconsManager {
    pub fn new(engine: Arc<Engine>) -> Self {
        let container = engine.new_layer();
        container.set_key("app_icons_manager");
        container.set_pointer_events(false);
        container.set_layout_style(Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        Self {
            engine,
            container,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Return the icon stack for `app_id`, creating it if it does not yet exist.
    /// Stacks are never removed, so the returned `Layer` (and its `NodeRef`) stays valid forever.
    pub fn get_or_create_stack(&self, app_id: &str, app: &Application) -> Layer {
        {
            let entries = self.entries.read().unwrap();
            if let Some(entry) = entries.get(app_id) {
                return entry.stack.clone();
            }
        }

        let stack = self.engine.new_layer();
        let icon_layer = self.engine.new_layer();
        let badge_layer = self.engine.new_layer();
        let progress_layer = self.engine.new_layer();

        let stack_tree = LayerTreeBuilder::default()
            .key(format!("icon_stack_{}", app_id))
            .layout_style(taffy::Style {
                position: taffy::Position::Absolute,
                ..Default::default()
            })
            .size(Size::points(BASE_ICON_SIZE, BASE_ICON_SIZE))
            .picture_cached(true)
            .image_cache(true)
            .pointer_events(false)
            .build()
            .unwrap();
        stack.build_layer_tree(&stack_tree);

        let icon_tree = LayerTreeBuilder::default()
            .key("icon")
            .layout_style(taffy::Style {
                display: taffy::Display::Block,
                position: taffy::Position::Relative,
                ..Default::default()
            })
            .size((
                Size {
                    width: taffy::Dimension::Percent(1.0),
                    height: taffy::Dimension::Percent(1.0),
                },
                None,
            ))
            .pointer_events(false)
            .picture_cached(false)
            .image_cache(false)
            .content(Some(draw_app_icon(app)))
            .build()
            .unwrap();
        icon_layer.build_layer_tree(&icon_tree);
        icon_layer.set_image_cached(true);

        setup_badge_layer(&badge_layer, BASE_ICON_SIZE);
        setup_progress_layer(&progress_layer, BASE_ICON_SIZE);

        self.container.add_sublayer(&stack);
        stack.add_sublayer(&icon_layer);
        stack.add_sublayer(&badge_layer);
        stack.add_sublayer(&progress_layer);

        let icon_id = app.icon.as_ref().map(|i| i.unique_id());
        self.entries.write().unwrap().insert(
            app_id.to_string(),
            AppIconEntry {
                stack: stack.clone(),
                icon_layer,
                badge_layer,
                progress_layer,
                icon_id,
            },
        );
        stack
    }

    /// Return the icon stack for `app_id` if it has been created, or `None`.
    pub fn get_stack(&self, app_id: &str) -> Option<Layer> {
        self.entries
            .read()
            .unwrap()
            .get(app_id)
            .map(|e| e.stack.clone())
    }

    /// Redraw the icon if `app`'s icon has changed since the last call.
    pub fn update_app(&self, app_id: &str, app: &Application) {
        let current_icon_id = app.icon.as_ref().map(|i| i.unique_id());
        let mut entries = self.entries.write().unwrap();
        if let Some(entry) = entries.get_mut(app_id) {
            if entry.icon_id != current_icon_id {
                entry.icon_layer.set_draw_content(draw_app_icon(app));
                entry.icon_id = current_icon_id;
            }
        }
    }

    /// Show or hide the badge on the dock/switcher icon for `app_id`.
    pub fn update_badge(&self, app_id: &str, text: Option<String>) {
        let entries = self.entries.read().unwrap();
        if let Some(entry) = entries.get(app_id) {
            match text {
                Some(t) if !t.is_empty() => {
                    entry.badge_layer.set_draw_content(draw_badge(t));
                    entry
                        .badge_layer
                        .set_opacity(1.0, Some(Transition::ease_in_quad(0.15)));
                }
                _ => {
                    entry
                        .badge_layer
                        .set_opacity(0.0, Some(Transition::ease_in_quad(0.15)));
                }
            }
        }
    }

    /// Show or hide the progress bar on the dock/switcher icon for `app_id`.
    pub fn update_progress(&self, app_id: &str, value: Option<f64>) {
        let entries = self.entries.read().unwrap();
        if let Some(entry) = entries.get(app_id) {
            match value {
                Some(v) if v >= 0.0 => {
                    entry
                        .progress_layer
                        .set_draw_content(draw_progress(v.clamp(0.0, 1.0)));
                    entry
                        .progress_layer
                        .set_opacity(1.0, Some(Transition::ease_in_quad(0.15)));
                }
                _ => {
                    entry
                        .progress_layer
                        .set_opacity(0.0, Some(Transition::ease_in_quad(0.15)));
                }
            }
        }
    }
}
