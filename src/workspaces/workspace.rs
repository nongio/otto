use super::{BackgroundView, WindowSelectorView};
use crate::{
    config::Config,
    shell::WindowElement,
    utils::{image_from_path, parse_hex_color},
};
use core::fmt;

use layers::{
    engine::Engine,
    prelude::{taffy, Layer, Transition},
    types::Size,
};
use smithay::reexports::wayland_server::backend::ObjectId;
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, RwLock},
};

/// Spacing between workspaces in logical pixels (multiply by screen scale when used)
pub const WORKSPACE_SPACING: f32 = 50.0;

#[derive(Clone)]
pub struct WorkspaceView {
    pub index: usize,
    pub windows_list: Arc<RwLock<Vec<ObjectId>>>,

    // views
    pub window_selector_view: Arc<WindowSelectorView>,
    pub background_view: Arc<BackgroundView>,

    // scene
    pub layers_engine: Arc<Engine>,
    pub windows_layer: Layer,
    /// Container for background_view + layer_shell_bg_mirror.
    /// NodeRef for the background KMS plane.
    pub workspace_background: Layer,
    /// Mirror of the per-output wlr-layer-shell *bottom* container — the
    /// desktop widget layer. Held so exposé can fade it out: unlike the
    /// wallpaper below it, it is chrome, not part of the overview.
    pub layer_shell_bottom_mirror: Layer,
    /// Wallpaper only — the config background plus the wlr `background`
    /// layers, without the widget layer above them. The workspace selector
    /// replicates *this* rather than `workspace_background`, so a desktop
    /// widget never appears inside a workspace preview.
    pub wallpaper_group: Layer,

    fullscreen_mode: Arc<AtomicBool>,
    is_fullscreen_animating: Arc<AtomicBool>,
    name: Arc<RwLock<Option<String>>>,
    /// Name the user typed in the workspace selector. Takes precedence over
    /// `name` (which the fullscreen path sets to the app's name) and over the
    /// positional fallback.
    custom_name: Arc<RwLock<Option<String>>>,
    window_base_layers: Arc<RwLock<HashMap<ObjectId, Layer>>>,
    /// Stacking order (bottom→top ObjectIds) saved when expose opens,
    /// so it can be restored verbatim when expose closes without selection.
    pre_expose_order: Arc<RwLock<Vec<ObjectId>>>,
}

impl fmt::Debug for WorkspaceView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // let model = self.model.read().unwrap();

        f.debug_struct("WorkspaceModel")
            // .field("applications", &model.applications_cache)
            // .field("application_list", &self.application_list)
            // .field("windows", &self.windows)
            // .field("current_application", &self.current_application)
            .finish()
    }
}

/// # Workspace Layer Structure
///
/// ```diagram
/// WorkspaceView
/// └── workspace_view
///     ├── background_view (config-driven gradient/image)
///     ├── layer_shell_bg_mirror (mirror: per-output wlr-layer-shell background)
///     ├── workspace_windows_container
///     │   ├── window
///     │   ├── window
///     │   └── window
///     └── overlay
///         └── fullscreen_surface
/// ```
///
impl WorkspaceView {
    pub fn new(
        index: usize,
        layers_engine: Arc<Engine>,
        _parent: &Layer,
        overlay_layer: Layer,
        layer_shell_background: &Layer,
        layer_shell_bottom: &Layer,
    ) -> Self {
        println!("add_workspace {}", index);

        let background_layer = layers_engine.new_layer();
        background_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        background_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        // background_layer.set_opacity(0.0, None);

        let windows_layer = layers_engine.new_layer();
        windows_layer.set_key(format!("workspace_windows_{}", index));
        windows_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // Clipping is here (was on workspace_layer) so windows don't bleed across
        // workspace boundaries when the windows_plane scrolls.
        windows_layer.set_clip_children(true, None);
        windows_layer.set_clip_content(true, None);
        windows_layer.set_image_cached(false);
        windows_layer.set_pointer_events(false);

        // Container for all background content — used as the NodeRef for the
        // background KMS plane (Phase 3). Groups background_view and the
        // layer_shell_bg_mirror so they can be rendered independently.
        let workspace_background = layers_engine.new_layer();
        workspace_background.set_key(format!("workspace_background_{}", index));
        workspace_background.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        workspace_background.set_size(layers::types::Size::percent(1.0, 1.0), None);
        workspace_background.set_pointer_events(false);

        // workspace_background is NOT attached here — the caller (Workspaces) places it
        // into the shared backgrounds_root so all workspaces' backgrounds live in one
        // layer tree that can be rendered as a single KMS plane.
        // Wallpaper pieces live one level down so the selector can replicate
        // the wallpaper alone. Widgets stay a direct child of
        // `workspace_background`, which is still what the background KMS
        // plane renders — the split is about what a *preview* shows, not
        // about what reaches the screen.
        let wallpaper_group = layers_engine.new_layer();
        wallpaper_group.set_key(format!("workspace_wallpaper_{}", index));
        wallpaper_group.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        wallpaper_group.set_size(layers::types::Size::percent(1.0, 1.0), None);
        wallpaper_group.set_pointer_events(false);
        let _ = layers_engine.append_layer(&wallpaper_group, Some(workspace_background.id));

        let _ = layers_engine.append_layer(&background_layer, Some(wallpaper_group.id));

        // Mirror the per-output wlr-layer-shell background container into this workspace,
        // above the config-driven background_view and below windows.
        let layer_shell_bg_mirror = layers_engine.new_layer();
        layer_shell_bg_mirror.set_key(format!("layer_shell_bg_mirror_{}", index));
        layer_shell_bg_mirror.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer_shell_bg_mirror.set_size(layers::types::Size::percent(1.0, 1.0), None);
        layer_shell_bg_mirror.set_draw_content(layer_shell_background.as_content());
        layer_shell_bg_mirror.set_picture_cached(false);
        layer_shell_background.add_follower_node(&layer_shell_bg_mirror);
        layer_shell_bg_mirror.set_pointer_events(false);
        let _ = layers_engine.append_layer(&layer_shell_bg_mirror, Some(wallpaper_group.id));

        // The widget layer sits on the same plane, directly above the
        // wallpaper and still behind every window. Appended after the
        // wallpaper mirror so it stacks on top of it.
        let layer_shell_bottom_mirror = layers_engine.new_layer();
        layer_shell_bottom_mirror.set_key(format!("layer_shell_bottom_mirror_{}", index));
        layer_shell_bottom_mirror.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        layer_shell_bottom_mirror.set_size(layers::types::Size::percent(1.0, 1.0), None);
        layer_shell_bottom_mirror.set_draw_content(layer_shell_bottom.as_content());
        layer_shell_bottom_mirror.set_picture_cached(false);
        layer_shell_bottom.add_follower_node(&layer_shell_bottom_mirror);
        layer_shell_bottom_mirror.set_pointer_events(false);
        let _ =
            layers_engine.append_layer(&layer_shell_bottom_mirror, Some(workspace_background.id));

        // windows_layer is NOT attached here — the caller places it into windows_plane.

        // Parse background color from config
        let background_color = Config::with(|c| parse_hex_color(&c.background_color));
        let background_view =
            BackgroundView::new(index, background_layer.clone(), background_color);
        let background_path = Config::with(|c| c.background_image.clone());
        if let Some(background_image) = image_from_path(&background_path, (2048, 2048)) {
            background_view.set_image(background_image);
        }
        let background_view = Arc::new(background_view);

        let window_selector_view = WindowSelectorView::new(
            index,
            layers_engine.clone(),
            &background_layer,
            overlay_layer,
            layer_shell_background,
            layer_shell_bottom,
        );

        let window_selector_view = Arc::new(window_selector_view);

        if let Some(background_image) = image_from_path(&background_path, (2048, 2048)) {
            background_view.set_image(background_image);
        } else {
            tracing::warn!(
                "Failed to load background image from path: {}",
                background_path
            );
        }
        Self {
            index,
            windows_list: Arc::new(RwLock::new(Vec::new())),
            window_selector_view: window_selector_view.clone(),
            background_view,
            layers_engine,
            windows_layer,
            workspace_background,
            layer_shell_bottom_mirror,
            wallpaper_group,
            fullscreen_mode: Arc::new(AtomicBool::new(false)),
            is_fullscreen_animating: Arc::new(AtomicBool::new(false)),
            name: Arc::new(RwLock::new(None)),
            custom_name: Arc::new(RwLock::new(None)),
            window_base_layers: Arc::new(RwLock::new(HashMap::new())),
            pre_expose_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn update_layout(&self, logical_index: usize, width: f32, height: f32, scale: f32) {
        let x = logical_index as f32 * (width + WORKSPACE_SPACING * scale);
        // workspace_background mirrors the workspace position inside background_plane
        self.workspace_background
            .set_size(layers::types::Size::points(width, height), None);
        self.workspace_background.set_position((x, 0.0), None);
        // windows_layer mirrors the workspace position inside windows_plane
        self.windows_layer
            .set_size(layers::types::Size::points(width, height), None);
        self.windows_layer.set_position((x, 0.0), None);
    }

    /// add a window layer to the workspace windows container
    /// and append the window to the windows list
    /// and creates a clone of the window layer to be used in the window selector view
    /// (if the window is already in the windows list, it will not be added)
    pub fn map_window(
        &self,
        window_element: &WindowElement,
        location: smithay::utils::Point<i32, smithay::utils::Logical>,
        transition: Option<Transition>,
    ) {
        let mut window_list = self.windows_list.write().unwrap();
        let wid = window_element.id();
        if !window_list.contains(&wid) {
            window_list.push(wid.clone());

            let _ = self
                .windows_layer
                .add_sublayer(&window_element.base_layer().id);

            let mirror_window = window_element.mirror_layer();
            let size = window_element.base_layer().render_size_transformed();
            mirror_window.set_size(Size::points(size.x, size.y), None);
            let _ = self
                .window_selector_view
                .window_selector_windows_container
                .add_sublayer(mirror_window);

            let window_base = window_element.base_layer();
            self.window_selector_view
                .map_window(wid.clone(), mirror_window);

            self.window_base_layers
                .write()
                .unwrap()
                .insert(wid.clone(), window_base.clone());
        }

        let scale = Config::with(|c| c.screen_scale);
        let location = location.to_f64().to_physical(scale);

        let position = crate::workspaces::utils::snap_position_px(location.x, location.y);
        window_element
            .base_layer()
            .set_position(position, transition);

        if let Some(l) = self
            .window_selector_view
            .layer_for_window(&window_element.id())
        {
            l.set_position(position, None);
        }
    }

    /// remove the window from the windows list
    /// and remove the window layer from the window selector view
    pub fn unmap_window(&self, window_id: &ObjectId) {
        self.unmap_window_internal(window_id);

        if let Some(mirror_layer) = self.window_selector_view.unmap_window(window_id) {
            // Remove both the base_layer mapping and the mirror layer
            // Don't call remove_follower_node as it may cause accessing freed nodes
            // when the layer tree is being modified during window destruction
            self.window_base_layers.write().unwrap().remove(window_id);
            mirror_layer.remove();
        } else {
            self.window_base_layers.write().unwrap().remove(window_id);
        }
    }

    /// Unmap for a cross-output migration: detach the window from this view's
    /// lists and selector bookkeeping but KEEP the mirror layer node alive —
    /// the target output's view re-parents the same mirror. Deleting it here
    /// (`Layer::remove` = mark_for_delete) would leave the window's fixed
    /// mirror handle pointing at a freed node, permanently blanking its
    /// expose preview.
    pub fn unmap_window_keep_mirror(&self, window_id: &ObjectId) {
        self.unmap_window_internal(window_id);
        let _ = self.window_selector_view.unmap_window(window_id);
        self.window_base_layers.write().unwrap().remove(window_id);
    }

    /// Internal version of unmap_window that allows controlling whether to remove the mirror layer
    /// When remove_mirror is false, the mirror layer is not removed to avoid SlotMap key issues
    /// during drag-and-drop operations when expose_show_all will be called to rebuild the layout
    pub fn unmap_window_internal(&self, window_id: &ObjectId) {
        let mut window_list = self.windows_list.write().unwrap();

        if let Some(index) = window_list.iter().position(|x| x == window_id) {
            window_list.remove(index);
        }
    }

    pub fn raise_window_to_front(&self, window_id: &ObjectId) {
        {
            let mut window_list = self.windows_list.write().unwrap();
            if let Some(index) = window_list.iter().position(|x| x == window_id) {
                if index + 1 != window_list.len() {
                    let wid = window_list.remove(index);
                    window_list.push(wid);
                }
            }
        }

        if let Some(base_layer) = self
            .window_base_layers
            .read()
            .unwrap()
            .get(window_id)
            .cloned()
        {
            if let Err(e) = self.windows_layer.add_sublayer(&base_layer) {
                tracing::warn!("raise_window_to_front: failed to reparent window layer: {e}");
                return;
            }
        }

        self.window_selector_view.bring_window_to_front(window_id);
    }

    pub fn set_fullscreen_mode(&self, fullscreen: bool) {
        self.fullscreen_mode
            .store(fullscreen, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn get_fullscreen_mode(&self) -> bool {
        self.fullscreen_mode
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_fullscreen_animating(&self, animating: bool) {
        self.is_fullscreen_animating
            .store(animating, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn get_fullscreen_animating(&self) -> bool {
        self.is_fullscreen_animating
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_name(&self, name: Option<String>) {
        *self.name.write().unwrap() = name;
    }

    pub fn get_name(&self) -> Option<String> {
        self.name.read().unwrap().clone()
    }

    /// Set the user-chosen name. An empty name clears it, falling back to the
    /// fullscreen app name or the positional default.
    pub fn set_custom_name(&self, name: Option<String>) {
        let name = name.filter(|n| !n.trim().is_empty());
        *self.custom_name.write().unwrap() = name;
    }

    pub fn get_custom_name(&self) -> Option<String> {
        self.custom_name.read().unwrap().clone()
    }

    /// What the workspace is called in the UI: the user's name, else the
    /// fullscreen app name, else `Workspace <position + 1>`.
    pub fn display_name(&self, position: usize) -> String {
        self.get_custom_name()
            .or_else(|| self.get_name())
            .unwrap_or_else(|| format!("Workspace {}", position + 1))
    }

    /// Returns true if no pre-expose stacking order has been saved yet.
    pub fn peek_pre_expose_order_empty(&self) -> bool {
        self.pre_expose_order.read().unwrap().is_empty()
    }

    /// Snapshot the current stacking order so it can be restored after expose.
    pub fn save_pre_expose_order(&self, order: Vec<ObjectId>) {
        *self.pre_expose_order.write().unwrap() = order;
    }

    /// Take the saved pre-expose stacking order, leaving it empty.
    pub fn take_pre_expose_order(&self) -> Vec<ObjectId> {
        std::mem::take(&mut *self.pre_expose_order.write().unwrap())
    }
}

impl Drop for WorkspaceView {
    fn drop(&mut self) {
        self.windows_layer.remove();
        self.window_selector_view.window_selector_root.remove();
    }
}
