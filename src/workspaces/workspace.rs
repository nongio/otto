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
    pub workspace_layer: Layer,
    pub windows_layer: Layer,

    fullscreen_mode: Arc<AtomicBool>,
    is_fullscreen_animating: Arc<AtomicBool>,
    name: Arc<RwLock<Option<String>>>,
    window_base_layers: Arc<RwLock<HashMap<ObjectId, Layer>>>,
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
///     ├── background_view
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
        parent: &Layer,
        overlay_layer: Layer,
    ) -> Self {
        println!("add_workspace {}", index);

        let workspace_layer = layers_engine.new_layer();
        workspace_layer.set_key(format!("workspace_view_{}", index));
        workspace_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        workspace_layer.set_size(layers::types::Size::auto(), None);

        workspace_layer.set_position((0.0, 0.0), None);
        workspace_layer.set_pointer_events(false);
        let background_layer = layers_engine.new_layer();
        background_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        background_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        // background_layer.set_opacity(0.0, None);

        let windows_layer = layers_engine.new_layer();
        windows_layer.set_key(format!("workspace_windows_container_{}", index));
        windows_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        windows_layer.set_pointer_events(false);

        layers_engine.append_layer(&workspace_layer, parent.id);

        layers_engine.append_layer(&background_layer, Some(workspace_layer.id));
        layers_engine.append_layer(&windows_layer, Some(workspace_layer.id));

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
            background_view.base_layer.clone(),
            overlay_layer,
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
            workspace_layer,
            fullscreen_mode: Arc::new(AtomicBool::new(false)),
            is_fullscreen_animating: Arc::new(AtomicBool::new(false)),
            name: Arc::new(RwLock::new(None)),
            window_base_layers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn update_layout(&self, logical_index: usize, width: f32, height: f32, scale: f32) {
        self.workspace_layer
            .set_size(layers::types::Size::points(width, height), None);
        // Position workspaces with spacing between them (WORKSPACE_SPACING is logical, scale to physical)
        let x = logical_index as f32 * (width + WORKSPACE_SPACING * scale);
        self.workspace_layer.set_position((x, 0.0), None);
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

            self.windows_layer
                .add_sublayer(&window_element.base_layer().id);

            let mirror_window = window_element.mirror_layer();
            let size = window_element.base_layer().render_size_transformed();
            mirror_window.set_size(Size::points(size.x, size.y), None);
            self.window_selector_view
                .windows_layer
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

        window_element.base_layer().set_position(
            layers::types::Point {
                x: location.x as f32,
                y: location.y as f32,
            },
            transition,
        );

        if let Some(l) = self
            .window_selector_view
            .layer_for_window(&window_element.id())
        {
            l.set_position(
                layers::types::Point {
                    x: location.x as f32,
                    y: location.y as f32,
                },
                None,
            );
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

    /// Internal version of unmap_window that allows controlling whether to remove the mirror layer
    /// When remove_mirror is false, the mirror layer is not removed to avoid SlotMap key issues
    /// during drag-and-drop operations when expose_show_all will be called to rebuild the layout
    pub fn unmap_window_internal(&self, window_id: &ObjectId) {
        let mut window_list = self.windows_list.write().unwrap();

        if let Some(index) = window_list.iter().position(|x| x == window_id) {
            window_list.remove(index);
        }
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
}

impl Drop for WorkspaceView {
    fn drop(&mut self) {
        self.windows_layer.remove();
        self.workspace_layer.remove();
        self.window_selector_view.layer.remove();
    }
}
