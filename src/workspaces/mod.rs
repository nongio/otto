use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicI32},
        Arc, RwLock, Weak,
    },
};

use apps_info::Application;
use layers::{
    engine::{Engine, TransactionRef},
    prelude::{taffy, Interpolate, Layer, Spring, TimingFunction, Transition},
    skia::{self, Contains},
    types::Size,
};
use smithay::{
    desktop::{layer_map_for_output, Space, WindowSurface},
    output::Output,
    reexports::wayland_server::{backend::ObjectId, Resource},
    utils::{IsAlive, Rectangle},
};

use wayland_server::DisplayHandle;
use workspace::WorkspaceView;

mod app_switcher;
mod background;
mod dnd_view;
mod dock;
mod osd;
mod popup_overlay;
pub mod workspace;

pub mod utils;

mod apps_info;
mod window_selector;
mod window_view;
mod workspace_selector;

pub use background::BackgroundView;
pub use window_selector::{WindowSelectorView, WindowSelectorWindow};
pub use window_view::{WindowView, WindowViewBaseModel, WindowViewSurface};

pub use app_switcher::AppSwitcherView;
pub use apps_info::ApplicationsInfo;
pub use dnd_view::DndView;
pub use dock::DockView;
pub use osd::OsdView;
pub use popup_overlay::PopupOverlayView;
pub use workspace::WORKSPACE_SPACING;
pub use workspace_selector::{WorkspaceSelectorView, WORKSPACE_SELECTOR_PREVIEW_WIDTH};

use crate::{
    config::Config,
    shell::WindowElement,
    utils::{natural_layout::LayoutRect, Observable, Observer},
};

#[derive(Debug, Default, Clone)]
pub struct WorkspacesModel {
    workspace_counter: usize,
    pub workspaces: Vec<Arc<WorkspaceView>>,
    pub current_workspace: usize,

    pub app_windows_map: HashMap<String, Vec<ObjectId>>,
    /// list of applications in the order they are visually displayed
    /// mainly used for the app switcher
    pub zindex_application_list: Vec<String>,
    /// list of applications in the order they are launched
    /// mainly used for the dock
    pub application_list: VecDeque<String>,

    pub minimized_windows: Vec<(ObjectId, String)>,
    pub current_application: usize,
    /// The physical width of the workspace
    pub width: i32,
    /// The physical height of the workspace
    pub height: i32,
    pub scale: f64,
}

pub struct Workspaces {
    model: Arc<RwLock<WorkspacesModel>>,
    spaces: Vec<Space<WindowElement>>,
    outputs: Vec<Output>,
    primary_output: Option<Output>,
    display_handle: DisplayHandle,

    pub windows_map: HashMap<ObjectId, WindowElement>,
    // views
    pub workspace_selector_view: Arc<WorkspaceSelectorView>,
    pub dock: Arc<DockView>,
    pub app_switcher: Arc<AppSwitcherView>,
    pub window_views: Arc<RwLock<HashMap<ObjectId, WindowView>>>,
    pub dnd_view: DndView,
    pub popup_overlay: PopupOverlayView,
    pub osd: OsdView,

    // gestures states
    pub show_all: Arc<AtomicBool>,
    pub show_desktop: Arc<AtomicBool>,
    pub show_all_gesture: Arc<AtomicI32>,
    pub show_desktop_gesture: Arc<AtomicI32>,
    /// Tracks whether the workspace is currently animating (e.g., scrolling between workspaces)
    pub is_animating: Arc<AtomicBool>,

    // layers
    pub layers_engine: Arc<Engine>,
    pub overlay_layer: Layer,
    pub workspaces_layer: Layer,
    /// Container for wlr-layer-shell background layer surfaces
    pub layer_shell_background: Layer,

    pub layer_shell_top: Layer,
    /// Container for wlr-layer-shell overlay layer surfaces  
    pub layer_shell_overlay: Layer,
    expose_layer: Layer,
    observers: Vec<Weak<dyn Observer<WorkspacesModel>>>,
    expose_dragged_window: Arc<std::sync::Mutex<Option<ObjectId>>>,
}

/// # Workspaces Layer Structure
///
/// ```diagram
/// Workspaces
/// root
/// ├── workspaces
/// │   ├── workspace_view_1
/// │   │   ├── background_view (mirrored)
/// │   │   └── workwspace_windows_container_1
/// │   │       ├── window_view_1
/// │   │       ├── window_view_2
/// │   │       ...
/// │   ├── workspace_view_2
/// │   ...
/// ├── expose
/// │   ├── windows_selector_root_1
/// │   │   ├── window_selector_background_1 (mirror: background_view)
/// │   │   ├── window_selector_windows_container_1
/// │   │   │   ├── mirror_window_1
/// │   │   │   ├── mirror_window_2
/// │   │   │   ...
/// │   │   ├── window_selector_view_1
/// │   ├── expose_view
/// │   ├── app_switcher
/// │
/// ├── dnd_view
/// ├── dock
/// ├── popup_overlay (popups rendered on top of everything)
/// ├── app_switcher
/// ├── workspace_selector_view
/// │   ├── workspace_selector_view_content
/// │   │   ├── workspace_selector_desktop_1
/// │   │   │   ├── workspace_selector_desktop_content_1
/// │   │   │   │   ├── workspace_selector_desktop_content_1 (mirror: workspace_view_1)
/// │   │   │   │   ├── workspace_selector_desktop_border_1
/// │   │   │   │   ├── workspace_selector_desktop_remove_1
/// │   │   │   ├── workspace_selector_desktop_label_1
/// │   │   ├── workspace_selector_desktop_2
/// │   │   │   ├── ...
/// │   │   ...
/// │   ├── workspace_selector_workspace_add
/// ```
///
impl Workspaces {
    pub fn start_window_selector_drag(&self, window_id: &ObjectId) {
        *self.expose_dragged_window.lock().unwrap() = Some(window_id.clone());
        self.expose_update_if_needed();
    }

    pub fn end_window_selector_drag(&self, window_id: &ObjectId) {
        let mut dragging = self.expose_dragged_window.lock().unwrap();
        if dragging.as_ref() == Some(window_id) {
            *dragging = None;
        }
        drop(dragging);
        self.expose_set_visible(true);
    }
    pub fn new(layers_engine: Arc<Engine>, display_handle: DisplayHandle) -> Self {
        let model = WorkspacesModel::default();
        let spaces = Vec::new();

        // Layer shell background layer (z-order: below workspaces)
        let layer_shell_background = layers_engine.new_layer();
        layer_shell_background.set_key("layer_shell_background");
        layer_shell_background.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        layer_shell_background.set_pointer_events(false);
        layers_engine.add_layer(&layer_shell_background);

        let workspaces_layer = layers_engine.new_layer();
        workspaces_layer.set_key("workspaces");
        workspaces_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            display: taffy::Display::Flex,
            ..Default::default()
        });

        workspaces_layer.set_size(layers::types::Size::auto(), None);
        workspaces_layer.set_pointer_events(false);

        layers_engine.add_layer(&workspaces_layer);

        let expose_layer = layers_engine.new_layer();
        expose_layer.set_key("expose");
        expose_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        expose_layer.set_size(layers::types::Size::percent(1.0, 1.0), None);
        expose_layer.set_pointer_events(false);
        expose_layer.set_hidden(true);
        expose_layer.set_picture_cached(false);
        expose_layer.set_image_cached(false);

        layers_engine.add_layer(&expose_layer);

        let overlay_layer = layers_engine.new_layer();
        overlay_layer.set_key("overlay_view");
        overlay_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        overlay_layer.set_pointer_events(false);
        let dnd_view = DndView::new(layers_engine.clone());

        let dock = DockView::new(layers_engine.clone());
        let dock = Arc::new(dock);
        dock.show(None);

        // Layer shell top layer (z-order: above workspaces)
        let layer_shell_top = layers_engine.new_layer();
        layer_shell_top.set_key("layer_shell_top");
        layer_shell_top.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        layer_shell_top.set_pointer_events(false);
        layers_engine.add_layer(&layer_shell_top);

        // Create popup overlay AFTER dock so it renders on top
        let popup_overlay = PopupOverlayView::new(layers_engine.clone());

        let app_switcher = AppSwitcherView::new(layers_engine.clone(), dock.clone());
        let app_switcher = Arc::new(app_switcher);

        let workspace_selector_layer = layers_engine.new_layer();
        workspace_selector_layer.set_pointer_events(false);
        layers_engine.add_layer(&workspace_selector_layer);
        layers_engine.add_layer(&overlay_layer);

        // Layer shell overlay layer (z-order: above overlay_layer, below popups)
        let layer_shell_overlay = layers_engine.new_layer();
        layer_shell_overlay.set_key("layer_shell_overlay");
        layer_shell_overlay.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            size: taffy::Size {
                width: taffy::Dimension::Percent(1.0),
                height: taffy::Dimension::Percent(1.0),
            },
            ..Default::default()
        });
        layer_shell_overlay.set_pointer_events(false);
        layers_engine.add_layer(&layer_shell_overlay);

        let workspace_selector_view = Arc::new(WorkspaceSelectorView::new(
            layers_engine.clone(),
            workspace_selector_layer.clone(),
        ));

        // Create OSD view on overlay layer
        let osd = OsdView::new(layers_engine.clone());
        overlay_layer.add_sublayer(&osd.wrap_layer);

        // layer.add_sublayer(overlay_layer.clone());

        let mut workspaces = Self {
            // layer,
            spaces,
            outputs: Vec::new(),
            primary_output: None,
            model: Arc::new(RwLock::new(model)),
            windows_map: HashMap::new(),
            workspaces_layer,
            expose_layer,
            app_switcher: app_switcher.clone(),
            workspace_selector_view: workspace_selector_view.clone(),
            dock: dock.clone(),
            dnd_view,
            popup_overlay,
            osd,
            overlay_layer,
            layer_shell_background,
            layer_shell_top,
            layer_shell_overlay,
            show_all: Arc::new(AtomicBool::new(false)),
            show_desktop: Arc::new(AtomicBool::new(false)),
            show_all_gesture: Arc::new(AtomicI32::new(0)),
            show_desktop_gesture: Arc::new(AtomicI32::new(0)),
            is_animating: Arc::new(AtomicBool::new(false)),
            window_views: Arc::new(RwLock::new(HashMap::new())),
            observers: Vec::new(),
            layers_engine,
            expose_dragged_window: Arc::new(std::sync::Mutex::new(None)),
            display_handle,
        };
        workspaces.add_workspace();
        workspaces.add_workspace();

        workspaces.add_listener(dock.clone());
        workspaces.add_listener(app_switcher.clone());
        workspaces.add_listener(workspace_selector_view.clone());
        workspaces
    }

    pub fn space(&self) -> &Space<WindowElement> {
        let index = self.with_model(|m| m.current_workspace);
        &self.spaces[index]
    }

    pub fn space_mut(&mut self) -> &mut Space<WindowElement> {
        let index = self.with_model(|m| m.current_workspace);

        &mut self.spaces[index]
    }
    /// Set the workspace screen physical size
    pub fn set_screen_dimension(&self, width: i32, height: i32) {
        let scale = Config::with(|c| c.screen_scale);
        let current_workspace = self.with_model_mut(|model| {
            model.width = width;
            model.height = height;
            model.scale = scale;
            let event = model.clone();
            self.notify_observers(&event);
            model.current_workspace
        });

        self.update_workspaces_layout();
        self.scroll_to_workspace_index(current_workspace, Some(Transition::ease_out_quad(0.0)));
        self.dock.set_screen_size(width, height);
    }

    pub fn get_logical_rect(&self) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        self.with_model(|model| {
            let scale = model.scale as f32;
            smithay::utils::Rectangle::new(
                (0, 0).into(),
                (
                    ((model.width as f32 / scale) as i32),
                    ((model.height as f32 / scale) as i32),
                )
                    .into(),
            )
        })
    }
    // Data model management

    pub fn with_model<T>(&self, f: impl FnOnce(&WorkspacesModel) -> T) -> T {
        let model = self.model.read().unwrap();
        f(&model)
    }

    fn update_workspaces_layout(&self) {
        let (width, height, scale, workspaces) = self.with_model(|model| {
            (
                model.width as f32,
                model.height as f32,
                model.scale as f32,
                model.workspaces.clone(),
            )
        });

        if width <= 0.0 || height <= 0.0 {
            return;
        }

        self.workspaces_layer
            .set_size(Size::points(width, height), None);
        self.expose_layer
            .set_size(Size::points(width, height), None);

        for (logical_index, workspace) in workspaces.iter().enumerate() {
            workspace.update_layout(logical_index, width, height, scale);
            let selector_layer = workspace.window_selector_view.layer.clone();
            selector_layer.set_size(Size::points(width, height), None);
            // Position window selector roots with same spacing as workspaces
            selector_layer.set_position((logical_index as f32 * (width + WORKSPACE_SPACING * scale), 0.0), None);
        }
    }

    pub fn with_model_mut<T>(&self, f: impl FnOnce(&mut WorkspacesModel) -> T) -> T {
        let mut model = self.model.write().unwrap();
        f(&mut model)
    }

    // Gestures

    /// Check if the current workspace has a fullscreen surface and is ready for direct scanout.
    /// Returns true only when:
    /// - The current workspace is in fullscreen mode
    /// - The workspace is not animating (not scrolling between workspaces)
    /// - The fullscreen window is not animating
    /// - Not in expose/show-all mode
    /// - App switcher is not visible
    /// - The workspace has exactly one window (the fullscreen window only)
    pub fn is_fullscreen_and_stable(&self) -> bool {
        // Check if expose mode is active
        if self.get_show_all() {
            return false;
        }

        // Check if app switcher is visible
        if self.app_switcher.alive() {
            return false;
        }

        // Check if workspace is animating
        if self.is_animating.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }

        // Get current workspace and check if it's in fullscreen mode
        let current_workspace = self.get_current_workspace();
        if !current_workspace.get_fullscreen_mode() {
            return false;
        }

        // Check if the fullscreen window is still animating
        if current_workspace.get_fullscreen_animating() {
            return false;
        }

        // Check that the workspace has exactly one window (only the fullscreen window)
        // If there are additional windows (e.g., dialogs), disable direct scanout
        let current_index = self.get_current_workspace_index();
        let window_count = self.spaces[current_index].elements().count();
        if window_count != 1 {
            return false;
        }

        true
    }

    /// Get the fullscreen window from the current workspace, if any.
    /// Returns Some(WindowElement) if the current workspace is in fullscreen mode
    /// and has a fullscreen window.
    pub fn get_fullscreen_window(&self) -> Option<WindowElement> {
        let current_workspace = self.get_current_workspace();
        if !current_workspace.get_fullscreen_mode() {
            return None;
        }

        // Find the fullscreen window in the current workspace
        let current_index = self.with_model(|m| m.current_workspace);
        self.spaces[current_index]
            .elements()
            .find(|w| w.is_fullscreen())
            .cloned()
    }

    /// Return if we are in window selection mode
    pub fn get_show_all(&self) -> bool {
        self.show_all.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Check if expose mode is currently transitioning (either via gesture or animation)
    /// Returns true if we're in the middle of opening or closing expose mode
    pub fn is_expose_transitioning(&self) -> bool {
        let gesture_value = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);
        let is_animating = self.is_animating.load(std::sync::atomic::Ordering::Relaxed);

        // We're transitioning if:
        // 1. Gesture value is between 0 and 1000 (not fully closed or fully open), OR
        // 2. Animation is in progress AND we're not at a stable state (0 or 1000)
        (gesture_value > 0 && gesture_value < 1000)
            || (is_animating && gesture_value != 0 && gesture_value != 1000)
    }

    /// Returns true if we're in the middle of opening or closing show desktop mode
    pub fn is_show_desktop_transitioning(&self) -> bool {
        let gesture_value = self
            .show_desktop_gesture
            .load(std::sync::atomic::Ordering::Relaxed);
        let is_animating = self.is_animating.load(std::sync::atomic::Ordering::Relaxed);

        // We're transitioning if:
        // 1. Gesture value is between 0 and 1000 (not fully closed or fully open), OR
        // 2. Animation is in progress AND we're not at a stable state (0 or 1000)
        (gesture_value > 0 && gesture_value < 1000)
            || (is_animating && gesture_value != 0 && gesture_value != 1000)
    }

    /// Set the window selection mode
    #[allow(dead_code)]
    fn set_show_all(&self, show_all: bool) {
        self.show_all
            .store(show_all, std::sync::atomic::Ordering::Relaxed);
    }

    /// Return if we are in show desktop mode
    pub fn get_show_desktop(&self) -> bool {
        self.show_desktop.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set the show desktop mode
    fn set_show_desktop(&self, show_all: bool) {
        self.show_desktop
            .store(show_all, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set the mode to window selection mode using a delta for gestures
    ///
    /// # Arguments
    /// * `delta` - The incremental change value from a gesture:
    ///   - For continuous gestures (e.g., three-finger swipe): use incremental values (typically -1.0 to 1.0 range)
    ///   - For layout updates without mode change: use `0.0` or `1.0` as needed
    /// * `end_gesture` - Whether the gesture/action has completed:
    ///   - `true`: Finalize the transition with animations and state commitment (snaps to nearest state based on threshold)
    ///   - `false`: Track finger movement without animation smoothing (direct 1:1 response during gesture)
    ///
    /// # Behavior
    /// The function uses a hysteresis mechanism: you must swipe at least 10% to enter the mode,
    /// but must swipe back past 90% to exit when already active, preventing accidental toggles.
    ///
    /// # Usage Examples
    /// - Keyboard toggle on: `expose_set_visible(true)`
    /// - Keyboard toggle off: `expose_set_visible(false)`
    /// - Gesture update mid-swipe: `expose_update(0.05)` (5% progress increment)
    /// - Gesture completion: `expose_end()` (finalize at current position)
    /// - Update layout during window drag: `expose_show_all(0.0, false)` (recalculate without animation)
    pub fn expose_show_all(&self, delta: f32, end_gesture: bool) {
        let current_workspace_index = self.get_current_workspace_index();
        let num_workspaces = self.with_model(|m| m.workspaces.len());

        // Update all workspaces during gesture AND at end for consistent overlay visibility
        for i in 0..num_workspaces {
            let animated = end_gesture && i == current_workspace_index;
            self.expose_show_all_workspace(i, delta, end_gesture, animated);
        }
    }

    /// Update expose mode during a gesture (no animation).
    pub fn expose_update(&self, delta: f32) {
        self.expose_show_all(delta, false);
    }

    /// Reset the accumulated expose gesture value.
    /// Called when starting a new expose gesture to prevent accumulation.
    pub fn reset_expose_gesture(&self) {
        let current_state = self.show_all.load(std::sync::atomic::Ordering::Relaxed);
        let reset_value = if current_state { 1000 } else { 0 };
        self.show_all_gesture
            .store(reset_value, std::sync::atomic::Ordering::Relaxed);
    }

    /// Reset the accumulated show desktop gesture value.
    /// Called when starting a new show desktop gesture to prevent accumulation.
    pub fn reset_show_desktop_gesture(&self) {
        let current_state = self.show_desktop.load(std::sync::atomic::Ordering::Relaxed);
        let reset_value = if current_state { 1000 } else { 0 };
        self.show_desktop_gesture
            .store(reset_value, std::sync::atomic::Ordering::Relaxed);
    }

    /// Finalize expose gesture and snap to the nearest state.
    pub fn expose_end(&self) {
        self.expose_show_all(0.0, true);
    }

    /// Finalize expose gesture with velocity-based spring animation.
    /// The velocity from the gesture is used to initialize the spring's momentum.
    pub fn expose_end_with_velocity(&self, raw_velocity: f32) {
        use layers::prelude::*;

        const MULTIPLIER: f32 = 1000.0;
        let current_gesture = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);
        let current_show_all = self.get_show_all();
        let gesture_progress = current_gesture as f32 / MULTIPLIER;

        tracing::debug!(
            raw_velocity = raw_velocity,
            gesture_progress = gesture_progress,
            current_show_all = current_show_all,
            "Expose gesture ending with velocity"
        );

        // Calculate projected position based on velocity
        // TIME_CONSTANT represents how far into the future (in gesture units) we project
        const TIME_CONSTANT: f32 = 0.15;
        let projected_progress = gesture_progress + raw_velocity * TIME_CONSTANT;

        // Determine if gesture should complete based on:
        // 1. Current position threshold (10% to open, 90% to close)
        // 2. Velocity direction and magnitude
        // 3. Projected final position
        let should_complete = if current_show_all {
            // Currently in expose mode - deciding whether to close
            // Close if: gesture is < 50% OR (< 70% AND velocity is downward)
            let velocity_suggests_close = raw_velocity < -20.0;
            gesture_progress < 0.5
                || (gesture_progress < 0.7 && velocity_suggests_close)
                || projected_progress < 0.5
        } else {
            // Currently closed - deciding whether to open expose
            // Open if: gesture is > 50% OR (> 30% AND velocity is upward)
            let velocity_suggests_open = raw_velocity > 20.0;
            gesture_progress > 0.5
                || (gesture_progress > 0.3 && velocity_suggests_open)
                || projected_progress > 0.5
        };

        let target_show_all = if current_show_all {
            !should_complete // If should_complete, we're completing the close action
        } else {
            should_complete // If should_complete, we're completing the open action
        };

        // Scale velocity to spring units
        const VELOCITY_SCALE: f32 = 0.01;
        let spring_velocity = raw_velocity * VELOCITY_SCALE;

        // Create spring with initial velocity from gesture
        let spring = Spring::with_duration_bounce_and_velocity(
            0.3,             // duration
            0.1,             // bounce
            spring_velocity, // initial velocity from gesture
        );

        let transition = Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(spring),
        };

        let current_workspace = self.get_current_workspace_index();
        // Use current delta so the spring animation can transition FROM current state TO target state
        let current_delta = if target_show_all { 1.0 } else { 0.0 };

        // Update show_all state immediately so next gesture starts from correct position
        self.show_all
            .store(target_show_all, std::sync::atomic::Ordering::Relaxed);

        // Reset gesture value to target state to prevent jumping on next gesture
        let target_gesture = if target_show_all { 1000 } else { 0 };
        self.show_all_gesture
            .store(target_gesture, std::sync::atomic::Ordering::Relaxed);

        // Update all workspaces so they all transition together
        let num_workspaces = self.with_model(|m| m.workspaces.len());
        for i in 0..num_workspaces {
            let animated = i == current_workspace;
            let workspace_transition = if animated { Some(transition) } else { None };
            self.expose_show_all_end(i, current_delta, target_show_all, workspace_transition);
        }
    }

    /// Explicitly show or hide expose mode (keyboard toggle).
    pub fn expose_set_visible(&self, show: bool) {
        use layers::prelude::*;

        // If show desktop is active, exit it first
        if self.get_show_desktop() && show {
            self.expose_show_desktop(-1.0, true);
            return;
        }

        // Set the gesture state to target value
        const MULTIPLIER: f32 = 1000.0;
        let target_gesture = if show { MULTIPLIER as i32 } else { 0 };
        self.show_all_gesture
            .store(target_gesture, std::sync::atomic::Ordering::Relaxed);
        self.show_all
            .store(show, std::sync::atomic::Ordering::Relaxed);

        // Create smooth spring transition (zero velocity for keyboard shortcuts)
        let spring = Spring::with_duration_and_bounce(0.3, 0.1);
        let transition = Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(spring),
        };

        let current_workspace = self.get_current_workspace_index();
        let delta_normalized = if show { 1.0 } else { 0.0 };
        self.expose_show_all_end(current_workspace, delta_normalized, show, Some(transition));
    }

    /// Process expose mode for a specific workspace
    /// Manages gesture state and delegates to layout/animation functions
    fn expose_show_all_workspace(
        &self,
        workspace_index: usize,
        delta: f32,
        end_gesture: bool,
        animated: bool,
    ) {
        const MULTIPLIER: f32 = 1000.0;
        let gesture = self
            .show_all_gesture
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut new_gesture = gesture + (delta * MULTIPLIER) as i32;
        let mut show_all = self.get_show_all();

        if end_gesture {
            if show_all {
                if new_gesture <= (9.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = 0;
                    show_all = false;
                } else {
                    new_gesture = MULTIPLIER as i32;
                    show_all = true;
                }
            } else {
                // animation_duration = 0.200;
                #[allow(clippy::collapsible_else_if)]
                if new_gesture >= (1.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = MULTIPLIER as i32;
                    show_all = true;
                } else {
                    new_gesture = 0;
                    show_all = false;
                }
            }
        }

        // Persist desired show_all state immediately on gesture end so that
        // follow-up calls (e.g. focus_app_with_window) don't re-open expose
        if end_gesture {
            self.show_all
                .store(show_all, std::sync::atomic::Ordering::Relaxed);
        }

        let delta_normalized = new_gesture as f32 / 1000.0;

        let transition = if animated {
            Some(Transition {
                delay: 0.0,
                timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.3, 0.1)),
            })
        } else {
            None
        };

        self.show_all_gesture
            .store(new_gesture, std::sync::atomic::Ordering::Relaxed);

        // Update/animate based on current state
        if end_gesture {
            self.expose_show_all_end(workspace_index, delta_normalized, show_all, transition);
        } else {
            self.expose_show_all_update(workspace_index, delta_normalized, show_all);
        }
    }

    /// Update the layout bin and window selector state for a workspace
    /// This ensures the bin has correct layout positions for all windows
    /// Returns true when a relayout was performed.
    fn expose_show_all_layout(&self, workspace_index: usize) -> bool {
        let Some(workspace) = self.get_workspace_at(workspace_index) else {
            tracing::warn!("Workspace {} not found for expose layout", workspace_index);
            return false;
        };

        // FIXME: remove hardcoded values
        let workspace_selector_height = 250.0;
        let padding_top = 10.0;
        let padding_bottom = 10.0;

        let size = self.workspaces_layer.render_size_transformed();
        let scale = Config::with(|c| c.screen_scale);
        let screen_size_w = size.x;
        let screen_size_h = size.y - padding_top - padding_bottom - workspace_selector_height;

        let offset_y = 200.0;
        let layout_rect = LayoutRect::new(
            0.0,
            workspace_selector_height,
            screen_size_w,
            screen_size_h - offset_y,
        );
        let dragging_window = self.expose_dragged_window.lock().unwrap().clone();
        let windows = self.with_model(|model| {
            if let Some(workspace_model) = model.workspaces.get(workspace_index) {
                let windows_list = workspace_model.windows_list.read().unwrap();
                let space = self.spaces.get(workspace_index).unwrap();
                let mut windows = Vec::new();

                for window_id in windows_list.iter() {
                    if dragging_window.as_ref() == Some(window_id) {
                        continue;
                    }
                    if let Some(window) = self.get_window_for_surface(window_id) {
                        if window.is_minimised() {
                            continue;
                        }
                        if let Some(bbox) = space.element_geometry(window) {
                            let bbox = bbox.to_f64().to_physical(scale);
                            window.mirror_layer().set_size(
                                Size::points(bbox.size.w as f32, bbox.size.h as f32),
                                None,
                            );
                            windows.push(WindowSelectorWindow {
                                id: window_id.clone(),
                                rect: LayoutRect::new(
                                    bbox.loc.x as f32,
                                    bbox.loc.y as f32,
                                    bbox.size.w as f32,
                                    bbox.size.h as f32,
                                ),
                                title: window.xdg_title().to_string(),
                            });
                        }
                    }
                }

                windows
            } else {
                Vec::new()
            }
        });

        // Skip relayout if window set and geometry match previous layout
        if workspace
            .window_selector_view
            .is_layout_up_to_date(&layout_rect, offset_y, &windows)
        {
            return false;
        }

        workspace
            .window_selector_view
            .update_windows(layout_rect, offset_y, &windows);
        true
    }

    /// Update expose state during gesture (no animation).
    fn expose_show_all_update(&self, workspace_index: usize, delta: f32, show_all: bool) {
        self.expose_show_all_apply(workspace_index, delta, None, show_all, false);
    }

    /// Finalize expose state and animate to the target.
    fn expose_show_all_end(
        &self,
        workspace_index: usize,
        delta: f32,
        show_all: bool,
        transition: Option<Transition>,
    ) {
        let velocity = if let Some(Transition {
            timing: TimingFunction::Spring(spring),
            ..
        }) = &transition
        {
            spring.initial_velocity
        } else {
            0.0
        };

        tracing::debug!(
            workspace = workspace_index,
            delta = delta,
            show_all = show_all,
            animated = transition.is_some(),
            velocity = velocity,
            "Ending expose show all gesture animation"
        );
        self.expose_show_all_apply(workspace_index, delta, transition, show_all, true);
    }

    /// Apply expose window positions and UI elements based on current delta and state.
    fn expose_show_all_apply(
        &self,
        workspace_index: usize,
        delta: f32,
        transition: Option<Transition>,
        show_all: bool,
        end_gesture: bool,
    ) {
        let delta = delta.clamp(0.0, 1.0);
        let is_gesture_ongoing = delta > 0.0 && delta < 1.0 && !end_gesture;
        let is_starting_animation = transition.is_some();
        let show_expose = delta > 0.0 || transition.is_some();

        // Check if this is the current workspace early, so we can use it for window animations
        let current_workspace_index = self.get_current_workspace_index();
        let is_current_workspace = workspace_index == current_workspace_index;

        // Hide popup overlay when entering expose mode
        self.popup_overlay.set_hidden(is_gesture_ongoing);
        let scale = Config::with(|c| c.screen_scale);

        let offset_y = 200.0;
        let mut changes = Vec::new();
        let Some(workspace_view) = self.get_workspace_at(workspace_index) else {
            tracing::warn!(
                "Workspace {} not found for expose animation",
                workspace_index
            );
            return;
        };
        let bin = workspace_view
            .window_selector_view
            .expose_bin
            .read()
            .unwrap();
        let dragged_window = self.expose_dragged_window.lock().unwrap().clone();

        // Show overlay only when: not animating, gesture ended, and value is 1.0
        let window_selector_overlay = workspace_view.window_selector_view.overlay_layer.clone();
        self.is_animating
            .store(is_starting_animation, std::sync::atomic::Ordering::Relaxed);

        // Keep layer visible (not hidden) but control opacity
        // Opacity should be 0 when expose is closed, 1.0 when fully open
        window_selector_overlay.set_hidden(false);

        // Calculate overlay opacity:
        // - During gestures (transition.is_none()): keep at 0.0 (hidden)
        // - After gesture ends (transition.is_some()): set to target, animation callback will apply it
        let overlay_opacity = if delta == 1.0 && transition.is_none() {
            delta
        } else {
            0.0 // Keep hidden during entire gesture
        };

        // Set opacity immediately for workspaces without animation
        // Workspaces with animation will have opacity set by the animation callback
        window_selector_overlay.set_opacity(overlay_opacity, None);

        workspace_view.window_selector_view.layer.set_hidden(false);

        workspace_view
            .window_selector_view
            .windows_layer
            .set_hidden(false);

        // Create animation if transition is specified
        let animation = transition
            .as_ref()
            .map(|t| self.layers_engine.add_animation_from_transition(t, false));

        // Animate window layers
        let current_workspace = self.with_model(|model| {
            if let Some(workspace) = model.workspaces.get(workspace_index) {
                let windows_list = workspace.windows_list.read().unwrap();
                let window_selector = workspace.window_selector_view.clone();
                let space = self.spaces.get(workspace_index).unwrap();

                for window_id in windows_list.iter() {
                    if dragged_window.as_ref() == Some(window_id) {
                        continue;
                    }
                    if let Some(window) = self.get_window_for_surface(window_id) {
                        if window.is_minimised() {
                            continue;
                        }
                        if let Some(bbox) = space.element_geometry(window) {
                            let bbox = bbox.to_f64().to_physical(scale);
                            if let Some(rect) = bin.get(window_id) {
                                let to_x = rect.x;
                                let to_y = rect.y + offset_y;
                                let to_width = rect.width;
                                let to_height = rect.height;
                                let (window_width, window_height) =
                                    (bbox.size.w as f32, bbox.size.h as f32);

                                let scale_x = to_width / window_width;
                                let scale_y = to_height / window_height;
                                let target_scale = scale_x.min(scale_y).min(1.0);

                                // Interpolate between current and target positions
                                let scale = 1.0.interpolate(&target_scale, delta);
                                let delta_clamped = delta.clamp(0.0, 1.0);
                                let window_x = bbox.loc.x as f32;
                                let window_y = bbox.loc.y as f32;
                                let x = window_x.interpolate(&to_x, delta_clamped);
                                let y = window_y.interpolate(&to_y, delta_clamped);

                                if let Some(layer) = window_selector.layer_for_window(window_id) {
                                    // Only animate if this is the current workspace AND a transition is provided
                                    if transition.is_some() && is_current_workspace {
                                        let translation =
                                            layer.change_position(layers::types::Point { x, y });
                                        let scale_change =
                                            layer.change_scale(layers::types::Point {
                                                x: scale,
                                                y: scale,
                                            });
                                        changes.push(translation);
                                        changes.push(scale_change);
                                    } else {
                                        // Non-current workspaces: instant update without animation
                                        layer.set_position(layers::types::Point { x, y }, None);
                                        layer.set_scale(
                                            layers::types::Point { x: scale, y: scale },
                                            None,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            model.workspaces.get(workspace_index).cloned()
        });

        // Schedule layer changes with animation
        if let Some(anim_ref) = animation {
            let _transactions = self.layers_engine.schedule_changes(&changes, anim_ref);
        };

        // Animate workspace selector and dock (only affects UI when is_current_workspace)
        let delta = delta.max(0.0);

        if is_current_workspace {
            // Workspace selector
            let workspace_selector_y = (-400.0).interpolate(&0.0, delta);
            let workspace_selector_y = workspace_selector_y.clamp(-400.0, 0.0);
            let workspace_opacity = 0.0.interpolate(&1.0, delta);
            let workspace_opacity = workspace_opacity.clamp(0.0, 1.0);

            // Layer shell overlay fades out when entering expose (inverse of workspace opacity)
            let layer_shell_overlay_opacity = 1.0.interpolate(&0.0, delta);
            let layer_shell_overlay_opacity = layer_shell_overlay_opacity.clamp(0.0, 1.0);

            // Set overlay opacity to match the workspace selector opacity (fade in as we enter expose)

            let window_selector_overlay_ref = window_selector_overlay.clone();
            let expose_layer = self.expose_layer.clone();
            let workspace_selector_view_layer = self.workspace_selector_view.layer.clone();
            let layer_shell_overlay_ref = self.layer_shell_overlay.clone();
            let show_all_ref = self.show_all.clone();

            expose_layer.set_hidden(!show_expose);
            workspace_selector_view_layer.set_hidden(!show_expose);

            let transaction = self.workspace_selector_view.layer.set_position(
                layers::types::Point {
                    x: 0.0,
                    y: workspace_selector_y,
                },
                transition,
            );
            if transition.is_some() {
                window_selector_overlay_ref.set_position((0.0, 0.0), None);
                transaction.on_finish(
                    move |_: &Layer, _: f32| {
                        let opacity = if show_all { 1.0 } else { 0.0 };
                        window_selector_overlay_ref.set_opacity(opacity, None);
                        expose_layer.set_hidden(!show_all);
                        workspace_selector_view_layer.set_hidden(!show_all);
                        // Restore layer shell overlay when exiting expose mode
                        layer_shell_overlay_ref.set_opacity(if show_all { 0.0 } else { 1.0 }, None);

                        show_all_ref.store(show_all, std::sync::atomic::Ordering::Relaxed);
                    },
                    true,
                );
            }
            let tr = self.workspace_selector_view
                .layer
                .set_opacity(workspace_opacity, transition);

            // Animate layer shell overlay opacity (fade out when entering expose)
            self.layer_shell_overlay
                .set_opacity(layer_shell_overlay_opacity, transition);

            if end_gesture {
                let wsv = self.workspace_selector_view
                .layer.clone();
                tr.on_finish(
                    move |_: &Layer, _: f32| {
                        // Animation finished
                        wsv.set_hidden(!show_expose);    
                    },
                    true,
                );
            }
        }
        // Animate dock position
        if let Some(current_workspace) = current_workspace {
            if !is_current_workspace {
                return;
            }
            // When autohide is on and the dock is visible, animate it down with the expose
            // gesture (same interpolation as the non-autohide path). The hot zone is suppressed
            // during expose (see check_dock_hot_zone).
            if self.dock.is_autohide_enabled() && !self.dock.is_hidden() {
                let dock_y = 0.0_f32.interpolate(&250.0, delta);
                self.dock.view_layer.set_position((0.0, dock_y.clamp(0.0, 250.0)), transition.clone());
                if end_gesture && show_all {
                    self.dock.schedule_autohide();
                }
            } else if !self.dock.is_autohide_enabled() {
                let mut start_position = 0.0;
                let mut end_position = 250.0;
                // Only keep dock hidden in fullscreen mode when NOT in expose mode
                // During expose mode, we want the dock to animate normally
                if current_workspace.get_fullscreen_mode() {
                    start_position = 250.0;
                    end_position = 250.0;
                }
                let dock_y = start_position.interpolate(&end_position, delta);
                let dock_y = dock_y.clamp(0.0, 250.0);
                self.dock.view_layer.set_position((0.0, dock_y), transition);
            }

            if let Some(anim_ref) = animation {
                self.layers_engine.start_animation(anim_ref, 0.0);
            }
            
        }
    }

    /// Set layer_shell_overlay visibility when entering/exiting fullscreen
    /// When entering fullscreen (is_fullscreen=true), fades out the overlay
    /// When exiting fullscreen (is_fullscreen=false), fades in the overlay
    pub fn set_fullscreen_overlay_visibility(&self, is_fullscreen: bool) {
        let target_opacity = if is_fullscreen { 0.0 } else { 1.0 };
        let transition = Some(Transition::ease_in_out_quad(1.4));

        self.layer_shell_overlay
            .set_opacity(target_opacity, transition);
    }

    /// Set the mode to show desktop mode using a delta for gestures
    pub fn expose_show_desktop(&self, delta: f32, end_gesture: bool) {
        // Don't allow mode switches while expose is animating
        if self.is_expose_transitioning() {
            return;
        }

        // If we're in expose mode, exit it first instead of transitioning directly
        if self.get_show_all() && end_gesture && delta > 0.0 {
            self.expose_set_visible(false);
            return;
        }

        const MULTIPLIER: f32 = 1000.0;
        let gesture = self
            .show_desktop_gesture
            .load(std::sync::atomic::Ordering::Relaxed);

        let mut new_gesture = gesture + (delta * MULTIPLIER) as i32;
        let show_desktop = self.get_show_desktop();

        let _model = self.model.read().unwrap();

        if end_gesture {
            if show_desktop {
                if new_gesture <= (9.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = 0;
                    self.set_show_desktop(false);
                } else {
                    new_gesture = MULTIPLIER as i32;
                    self.set_show_desktop(true);
                }
            } else {
                #[allow(clippy::collapsible_else_if)]
                if new_gesture >= (1.0 * MULTIPLIER / 10.0) as i32 {
                    new_gesture = MULTIPLIER as i32;
                    self.set_show_desktop(true);
                } else {
                    new_gesture = 0;
                    self.set_show_desktop(false);
                }
            }
        }

        let delta = new_gesture as f32 / 1000.0;
        let delta = delta.clamp(0.0, 1.0);

        // Store the accumulated gesture value back for next update
        self.show_desktop_gesture
            .store(new_gesture, std::sync::atomic::Ordering::Relaxed);

        // Use same spring transition as expose_show_all for consistency
        let mut transition = Some(Transition::spring(0.5, 0.1));
        if !end_gesture {
            transition = None;
        }

        // Get screen dimensions for calculating center
        let size = self.workspaces_layer.render_size_transformed();
        let scale = Config::with(|c| c.screen_scale);
        let screen_center_x = size.x / 2.0;
        let screen_center_y = size.y / 2.0;

        let current_workspace_index = self.get_current_workspace_index();
        let workspace = self.get_current_workspace();
        let windows_list = workspace.windows_list.read().unwrap();
        let space = self.spaces.get(current_workspace_index).unwrap();

        // Similar to expose_show_all: show_desktop_active when delta > 0
        let show_desktop_active = delta > 0.0 || transition.is_some();

        // Show/hide expose_layer (master container) when showing desktop
        // This matches the pattern in expose_show_all_apply
        let expose_layer = self.expose_layer.clone();
        let show_desktop_ref = self.show_desktop.clone();

        expose_layer.set_hidden(!show_desktop_active);

        // Show mirror windows layer when showing desktop, hide when not
        workspace
            .window_selector_view
            .windows_layer
            .set_hidden(!show_desktop_active);

        for window_id in windows_list.iter() {
            let window = self.get_window_for_surface(window_id).unwrap();
            if window.is_minimised() {
                continue;
            }

            // Get the original position from the workspace Space
            let Some(geometry) = space.element_geometry(window) else {
                continue;
            };
            let geometry = geometry.to_f64().to_physical(scale);

            let window_width = geometry.size.w as f32;
            let window_height = geometry.size.h as f32;
            let window_x = geometry.loc.x as f32;
            let window_y = geometry.loc.y as f32;

            // Set mirror layer size to match window
            window
                .mirror_layer()
                .set_size(Size::points(window_width, window_height), None);

            // Calculate window center
            let window_center_x = window_x + window_width / 2.0;
            let window_center_y = window_y + window_height / 2.0;

            // Calculate direction from screen center to window center
            let mut direction_x = window_center_x - screen_center_x;
            let mut direction_y = window_center_y - screen_center_y;

            // Normalize direction vector (with fallback for windows at exact center)
            let length = (direction_x * direction_x + direction_y * direction_y).sqrt();
            if length > 0.0 {
                direction_x /= length;
                direction_y /= length;
            } else {
                // If window is at exact center, push it to the right
                direction_x = 1.0;
                direction_y = 0.0;
            }

            // Calculate how far to push: just beyond the screen edge
            // Use screen size to push windows offscreen without going too far
            let push_distance = size.x.max(size.y);

            // Calculate target position offscreen in the direction
            let to_x = window_x + direction_x * push_distance;
            let to_y = window_y + direction_y * push_distance;

            // Interpolate between original workspace position and offscreen target
            let x = window_x.interpolate(&to_x, delta);
            let y = window_y.interpolate(&to_y, delta);

            // Animate the mirror layer, not the actual window
            window
                .mirror_layer()
                .set_position(layers::types::Point { x, y }, transition);
        }

        // If there's a transition, set up a callback to finalize visibility after animation
        if let Some(trans) = transition {
            // Mark as animating when we have a transition
            self.is_animating
                .store(true, std::sync::atomic::Ordering::Relaxed);

            let expose_layer_ref = expose_layer.clone();
            let window_selector_layer = workspace.window_selector_view.windows_layer.clone();
            let is_animating_ref = self.is_animating.clone();

            // Create a simple animation transaction to hook the on_finish callback
            let transaction = self.workspaces_layer.set_opacity(1.0, Some(trans));
            transaction.on_finish(
                move |_: &Layer, _: f32| {
                    let is_active = show_desktop_ref.load(std::sync::atomic::Ordering::Relaxed);
                    expose_layer_ref.set_hidden(!is_active);
                    window_selector_layer.set_hidden(!is_active);
                    // Clear animating flag when animation completes
                    is_animating_ref.store(false, std::sync::atomic::Ordering::Relaxed);
                },
                true,
            );
        }
    }

    pub fn expose_update_if_needed(&self) {
        let current_workspace_index = self.get_current_workspace_index();
        self.expose_update_if_needed_workspace(current_workspace_index);
    }
    pub fn expose_update_if_needed_workspace(&self, workspace_index: usize) {
        let relayout = self.expose_show_all_layout(workspace_index);
        if self.get_show_all() && relayout {
            let transition = Transition {
                delay: 0.0,
                timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.3, 0.1)),
            };
            self.expose_show_all_end(workspace_index, 1.0, true, Some(transition));
        }
    }
    /// Close all the windows of an app by its id
    pub fn quit_app(&self, app_id: &str) {
        for window_id in self.get_app_windows(app_id) {
            let window = self.get_window_for_surface(&window_id);
            if let Some(we) = window {
                match we.underlying_surface() {
                    WindowSurface::Wayland(t) => t.send_close(),
                    #[cfg(feature = "xwayland")]
                    WindowSurface::X11(w) => {
                        let _ = w.close();
                    }
                }
            }
        }
    }

    /// Close all the windows of the current focused App
    pub fn quit_current_app(&self) {
        if let Some(app_id) = self.get_current_app_id() {
            self.quit_app(&app_id);
        }
    }

    /// Close all the windows of the current focused App in th app switcher
    pub fn quit_appswitcher_app(&self) {
        if let Some(app_id) = self.app_switcher.get_current_app_id() {
            self.quit_app(&app_id);
        }
    }

    /// Minimise a WindowElement
    pub fn minimize_window(&mut self, we: &WindowElement) -> Option<ObjectId> {
        let id = we.id();

        if let Some(window) = self.windows_map.get_mut(&id) {
            window.set_is_minimised(true);
        }

        self.with_model_mut(|model| {
            model
                .minimized_windows
                .push((id.clone(), we.xdg_title().to_string()));

            if let Some(view) = self.get_window_view(&id) {
                let (drawer, _) = self.dock.add_window_element(we);

                view.window_layer.set_layout_style(taffy::Style {
                    position: taffy::Position::Absolute,
                    ..Default::default()
                });

                // Hide mirror layer so it won't appear in expose
                view.mirror_layer.set_hidden(true);

                self.layers_engine
                    .add_layer_to_positioned(view.window_layer.clone(), Some(drawer.id));
                // bounds are calculate after this call
                let drawer_bounds = drawer.render_bounds_transformed();
                view.minimize(skia::Rect::from_xywh(
                    drawer_bounds.x(),
                    drawer_bounds.y(),
                    drawer_bounds.width(),
                    drawer_bounds.height(),
                ));

                let view_ref = view.clone();
                drawer.clear_on_change_size_handlers();
                drawer.on_change_size(
                    move |layer: &Layer, _| {
                        let bounds = layer.render_bounds_transformed();
                        // Keep the minimized window scaled to the drawer bounds
                        view_ref.apply_minimized_scale(bounds);
                    },
                    false,
                );
            }

            self.notify_observers(model);
        });

        // Focus should move to the next topmost (non-minimized) window or to none.
        let index = self.with_model(|m| m.current_workspace);
        let next = self.spaces[index].elements().rev().find_map(|e| {
            let id = e.id();
            if let Some(window) = self.windows_map.get(&id) {
                if window.is_minimised() || window.id() == we.id() {
                    return None;
                }
            }
            Some(id)
        });

        if let Some(next_id) = next {
            // Raise and activate the next topmost window
            self.raise_element(&next_id, true, true);
            Some(next_id)
        } else {
            None
        }
    }

    /// Unminimise a WindowElement
    pub fn unminimize_window(&mut self, wid: &ObjectId) -> Option<ObjectId> {
        let workspace_for_window = self.with_model(|model| {
            model
                .workspaces
                .iter()
                .position(|ws| ws.windows_list.read().unwrap().contains(wid))
        });
        if workspace_for_window.is_none() {
            tracing::warn!(
                "Trying to unminimize a window that is not in any workspace: {}",
                wid
            );
            return None;
        }
        let workspace_for_window = workspace_for_window.unwrap();
        let current_workspace_index = self.get_current_workspace_index();

        let ctx = self.build_unminimize_context(wid)?;

        if workspace_for_window != current_workspace_index {
            if let Some(tr) = self.set_current_workspace_index(
                workspace_for_window,
                Some(Transition::ease_out_quad(0.2)),
            ) {
                let ctx_clone = ctx.clone();
                tr.on_finish(
                    move |_: &Layer, _: f32| {
                        ctx_clone.run();
                    },
                    true,
                );
                return Some(ctx.wid.clone());
            }
        }

        self.unminimize_window_in_workspace(ctx);
        Some(wid.clone())
    }

    fn unminimize_window_in_workspace(&self, ctx: UnminimizeContext) {
        ctx.run();
    }

    fn build_unminimize_context(&self, wid: &ObjectId) -> Option<UnminimizeContext> {
        let scale = Config::with(|c| c.screen_scale) as f32;
        let (index, space) = self
            .spaces
            .iter()
            .enumerate()
            .find(|(_, space)| space.elements().any(|e| e.id() == *wid))?;

        let workspace = self.with_model(|m| m.workspaces[index].clone());
        let window = self.get_window_for_surface(wid)?.clone();
        let view = self.get_window_view(wid)?;
        let window_geometry = space.element_geometry(&window)?;
        let pos_x = window_geometry.loc.x;
        let pos_y = window_geometry.loc.y;
        let layer_pos_x = pos_x as f32 * scale;
        let layer_pos_y = pos_y as f32 * scale;

        Some(UnminimizeContext {
            wid: wid.clone(),
            workspace,
            window,
            view,
            dock: self.dock.clone(),
            layers_engine: self.layers_engine.clone(),
            expose_layer: self.expose_layer.clone(),
            model: self.model.clone(),
            observers: self.observers.clone(),
            layer_pos: (layer_pos_x, layer_pos_y),
            pos_logical: (pos_x, pos_y),
        })
    }

    // Helpers / Windows Management

    /// Determine the initial placement of a new window within the workspace.
    /// It calculates the appropriate position and bounds for the window based
    /// on the current pointer location and the output geometry under the pointer.
    pub fn new_window_placement_at(
        &self,
        pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> (
        smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        smithay::utils::Point<i32, smithay::utils::Logical>,
    ) {
        let output = self
            .output_under(pointer_location)
            .next()
            .or_else(|| self.outputs().next())
            .cloned();
        let output_geometry = output
            .and_then(|o| {
                let geo = self.output_geometry(&o)?;
                tracing::info!("new_window_placement: output geometry = {:?}", geo);

                let map = layer_map_for_output(&o);
                let zone = map.non_exclusive_zone();
                tracing::info!("new_window_placement: non_exclusive_zone = {:?}", zone);

                let mut adjusted = Rectangle::new(geo.loc + zone.loc, zone.size);
                tracing::info!("new_window_placement: adjusted geometry (geo.loc + zone.loc, zone.size) = {:?}", adjusted);

                // Account for the dock geometry (internal compositor UI, not layer-shell)
                let dock_geom = self.get_dock_geometry();
                tracing::info!("new_window_placement: dock geometry = {:?}", dock_geom);

                if dock_geom.size.h > 0 {
                    let dock_top = dock_geom.loc.y;
                    let available_bottom = adjusted.loc.y + adjusted.size.h;

                    // If dock is in the usable area, reduce height to stop above dock
                    if dock_top < available_bottom {
                        adjusted.size.h = dock_top - adjusted.loc.y;
                        tracing::info!("new_window_placement: adjusted for dock, new height = {}", adjusted.size.h);
                    }
                }

                Some(adjusted)
            })
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (800, 800).into()));

        let num_open_windows = self.spaces_elements().count();
        let window_index = num_open_windows + 1; // Index of the new window

        tracing::info!(
            "new_window_placement: window_index = {}, num_open_windows = {}",
            window_index,
            num_open_windows
        );

        // Default window size assumption (will be adjusted by client during configure)
        const DEFAULT_WINDOW_WIDTH: i32 = 800;
        const DEFAULT_WINDOW_HEIGHT: i32 = 600;
        const CASCADE_OFFSET: i32 = 40; // Offset for each new window in cascade

        // Calculate available space within the non-exclusive zone
        let available_width = output_geometry.size.w;
        let available_height = output_geometry.size.h;

        tracing::info!(
            "new_window_placement: available_width = {}, available_height = {}",
            available_width,
            available_height
        );

        // Calculate cascade position with wrapping to stay within bounds
        let cascade_x = (window_index as i32 * CASCADE_OFFSET)
            % (available_width - DEFAULT_WINDOW_WIDTH).max(CASCADE_OFFSET);
        let cascade_y = (window_index as i32 * CASCADE_OFFSET)
            % (available_height - DEFAULT_WINDOW_HEIGHT).max(CASCADE_OFFSET);

        tracing::info!(
            "new_window_placement: cascade_x = {}, cascade_y = {}",
            cascade_x,
            cascade_y
        );

        // Calculate final position, ensuring window fits within available area
        let mut x = output_geometry.loc.x + cascade_x;
        let mut y = output_geometry.loc.y + cascade_y;

        tracing::info!("new_window_placement: initial x = {}, y = {}", x, y);

        // Clamp position to ensure window doesn't exceed boundaries
        x = x.min(
            output_geometry.loc.x + available_width - DEFAULT_WINDOW_WIDTH.min(available_width),
        );
        y = y.min(
            output_geometry.loc.y + available_height - DEFAULT_WINDOW_HEIGHT.min(available_height),
        );

        // Ensure position is not before the output start
        x = x.max(output_geometry.loc.x);
        y = y.max(output_geometry.loc.y);

        tracing::info!("new_window_placement: final position = ({}, {})", x, y);

        (output_geometry, (x, y).into())
    }

    /// map the window element, in the position on the current space,
    /// should be called on every window move / drag
    /// sets the position of the window layer in the scene
    pub fn map_window(
        &mut self,
        window_element: &WindowElement,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        activate: bool,
        transition: Option<Transition>,
    ) {
        self.space_mut()
            .map_element(window_element.clone(), location, activate);
        // self.space_mut().refresh();

        if let std::collections::hash_map::Entry::Vacant(e) =
            self.windows_map.entry(window_element.id())
        {
            e.insert(window_element.clone());

            self.update_workspace_model();
        }

        // append the window to the workspace layer
        {
            let location = self.element_location(window_element).unwrap_or_default();

            let workspace_view = self.get_current_workspace();

            workspace_view.map_window(window_element, location, transition);
            let _view = self.get_or_add_window_view(window_element);
        }
        self.refresh_space();
        self.expose_update_if_needed();
    }

    /// remove a WindowElement from the workspace model,
    /// remove the window layer from the scene,
    /// Returns the surface IDs from removed popups that need cleanup
    pub fn unmap_window(&mut self, window_id: &ObjectId) -> Vec<ObjectId> {
        tracing::info!("workspaces::unmap_window: {:?}", window_id);

        let mut workspace_index = None;

        if let Some(element) = self.get_window_for_surface(window_id).cloned() {
            for (i, space) in self.spaces.iter_mut().enumerate() {
                if space.elements().any(|e| e.id() == element.id()) {
                    workspace_index = Some(i);
                }
                space.unmap_elem(&element);
            }
        }

        self.with_model(|m| {
            for workspace_view in m.workspaces.iter() {
                workspace_view.unmap_window(window_id);
            }
        });
        self.windows_map.remove(window_id);
        // Remove debug texture snapshot for this surface
        crate::textures_storage::remove(window_id);
        let removed_surface_ids = self.remove_window_view(window_id);

        self.refresh_space();
        self.update_workspace_model();

        // Recalculate expose layout if in expose mode
        if let Some(index) = workspace_index {
            self.expose_update_if_needed_workspace(index);
        }

        // Return the surface IDs so the compositor can clean up surface_layers and sc_layers
        removed_surface_ids
    }
    /// Return if the current coordinates are over the dock
    pub fn is_cursor_over_dock(&self, x: f32, y: f32) -> bool {
        self.dock.alive()
            && self
                .dock
                .view_layer
                .render_bounds_transformed()
                .contains(skia::Point::new(x, y))
        || self.dock.has_menu_open()
    }

    /// Return the actual rendered height of the dock in logical pixels
    pub fn get_dock_height(&self) -> i32 {
        if self.dock.alive() {
            let bounds = self.dock.bar_layer.render_bounds_transformed();
            let scale = Config::with(|c| c.screen_scale);
            (bounds.height() / scale as f32).ceil() as i32
        } else {
            0
        }
    }

    /// Return the actual rendered geometry of the dock in logical coordinates
    pub fn get_dock_geometry(&self) -> Rectangle<i32, smithay::utils::Logical> {
        if self.dock.alive() {
            let bounds = self.dock.bar_layer.render_bounds_transformed();
            let scale = Config::with(|c| c.screen_scale) as f32;
            Rectangle::new(
                (
                    ((bounds.x() / scale) as i32),
                    ((bounds.y() / scale) as i32) - 2,
                )
                    .into(),
                (
                    (bounds.width() / scale).ceil() as i32,
                    (bounds.height() / scale).ceil() as i32,
                )
                    .into(),
            )
        } else {
            Rectangle::new((0, 0).into(), (0, 0).into())
        }
    }

    /// Return the list of WlSurface ids of an app by its id
    pub fn get_app_windows(&self, app_id: &str) -> Vec<ObjectId> {
        let model = self.model.read().unwrap();
        model
            .app_windows_map
            .get(app_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Return the list of Spaces where an app has windows by its id
    pub fn get_app_spaces(&self, app_id: &str) -> Vec<&Space<WindowElement>> {
        let model = self.model.read().unwrap();
        let mut spaces = Vec::new();

        model
            .app_windows_map
            .get(app_id)
            .cloned()
            .unwrap_or_default()
            .iter()
            .for_each(|id| {
                let window = self.get_window_for_surface(id);
                if let Some(we) = window {
                    for space in self.spaces.iter() {
                        if space.elements().any(|e| e == we) {
                            spaces.push(space);
                            break;
                        }
                    }
                }
            });

        spaces
    }

    /// Return the current focused Application
    pub fn get_current_app_id(&self) -> Option<String> {
        let model = self.model.read().unwrap();
        model.zindex_application_list.last().cloned()
    }

    /// Return the list of WlSurface ids of the current focused Application
    pub fn get_current_app_windows(&self) -> Vec<ObjectId> {
        self.get_current_app_id()
            .map(|app_id| self.get_app_windows(&app_id))
            .unwrap_or_default()
    }

    /// Return the Window object of WlSurface by its id
    pub fn get_window_for_surface(&self, id: &ObjectId) -> Option<&WindowElement> {
        self.windows_map.get(id)
    }

    pub fn get_or_add_window_view(
        &self,
        // object_id: &ObjectId,
        window: &WindowElement,
    ) -> WindowView {
        let mut window_views = self.window_views.write().unwrap();
        let wid = window.id();
        let entry = window_views
            .entry(wid.clone())
            .or_insert_with(|| WindowView::new(self.layers_engine.clone(), window));
        entry.clone()
    }

    /// Remove a WindowView from the scene and delete it from the window_views map
    /// Returns the surface IDs from removed popups that need cleanup
    pub fn remove_window_view(&mut self, object_id: &ObjectId) -> Vec<ObjectId> {
        // Remove any popups that belong to this window
        let removed_surface_ids = self.popup_overlay.remove_popups_for_window(object_id);

        let mut window_views = self.window_views.write().unwrap();
        if let Some(view) = window_views.remove(object_id) {
            view.window_layer.remove();
        }

        removed_surface_ids
    }

    pub fn get_window_view(&self, id: &ObjectId) -> Option<WindowView> {
        let window_views = self.window_views.read().unwrap();

        window_views.get(id).cloned()
    }

    pub fn set_window_view(&self, id: &ObjectId, window_view: WindowView) {
        let mut window_views = self.window_views.write().unwrap();

        window_views.insert(id.clone(), window_view);
    }

    /// unmap the window from the current space and workspaceview
    /// map the window to the new space and workspaceview
    pub fn move_window_to_workspace(
        &mut self,
        we: &WindowElement,
        workspace_index: usize,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
    ) {
        let location = location.into();

        let mut source_workspace_index = None;

        // unmap from old space
        if let Some((index, space)) = self
            .spaces
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.elements().any(|e| e.id() == we.id()))
        {
            source_workspace_index = Some(index);
            space.unmap_elem(we);
            let id = we.id();
            let model = self.model.read().unwrap();
            if let Some(workspace) = model.workspaces.get(index) {
                // Don't remove mirror layer during move - it causes SlotMap key issues
                // The expose view will be rebuilt with updated state after the move
                workspace.unmap_window_internal(&id);
            }
        }

        // map to new space

        if let Some(space) = self.spaces.get_mut(workspace_index) {
            tracing::info!(
                "workspaces::move_window_to_workspace: {:?}",
                workspace_index
            );
            space.map_element(we.clone(), location, false);
            let id = we.wl_surface().unwrap().id();
            let model = self.model.read().unwrap();
            if let Some(workspace) = model.workspaces.get(workspace_index) {
                if let Some(window) = self.windows_map.get(&id) {
                    workspace.map_window(window, location, None);
                }
            }
        }

        // Recalculate layout for both source and target workspaces if in expose mode
        if let Some(source_index) = source_workspace_index {
            self.expose_update_if_needed_workspace(source_index);
            self.expose_update_if_needed_workspace(workspace_index);
        }
    }

    pub fn raise_next_app_window(&mut self) -> Option<ObjectId> {
        let windows = self.get_current_app_windows();
        let mut wid = None;
        if !windows.is_empty() {
            for (i, window_id) in windows.iter().enumerate() {
                if i == 0 {
                    self.raise_element(window_id, true, true);
                    wid = Some(window_id.clone());
                }
            }
        }
        wid
    }

    pub fn raise_prev_app_window(&mut self) -> Option<ObjectId> {
        let windows = self.get_current_app_windows();
        let mut wid = None;
        if !windows.is_empty() {
            let current_window = (windows.len() as i32) - 1;
            let current_window = std::cmp::max(current_window, 0) as usize;
            for (i, window_id) in windows.iter().enumerate() {
                if i == current_window {
                    self.raise_element(window_id, true, true);
                    wid = Some(window_id.clone());
                }
            }
        }
        wid
    }

    /// Raise thw windowelement on top of all the windows in its space
    /// activate: will set the window as active
    /// update: will update the workspace model
    fn raise_element(&mut self, window_id: &ObjectId, activate: bool, update: bool) {
        // get the space with the window
        // tracing::info!("workspaces::raise_element: {:?}", window_id);
        if let Some((index, space)) = self
            .spaces
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.elements().any(|e| e.id() == *window_id))
        {
            if let Some(window) = self.windows_map.get(window_id) {
                if space.elements().last().unwrap().id() == *window_id {
                    return;
                }
                if window.is_minimised() && !activate {
                    return;
                }
                // FIXME: this is a hack to prevent raising a window that is already fullscreen
                // ideally we avoid resort a window already on top
                if window.is_fullscreen() {
                    return;
                }

                // Get the currently top window before raising the new one
                let previous_top = space.elements().last().map(|w| w.id());

                space.raise_element(window, activate);

                // Explicitly send configure to ensure activation state is communicated to client
                if activate {
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_pending_configure();
                    }
                }

                // When activating a window, manage popup visibility
                if activate {
                    // Hide popups for the previous top window
                    if let Some(prev_id) = previous_top {
                        if prev_id != *window_id {
                            self.popup_overlay.hide_popups_for_window(&prev_id);
                        }
                    }
                    // Show popups for the newly activated window
                    self.popup_overlay.show_popups_for_window(window_id);
                }

                let workspace = self.with_model(|m| m.workspaces[index].clone());
                {
                    if let Some(view) = self.get_window_view(window_id) {
                        workspace.windows_layer.add_sublayer(&view.window_layer);
                    }
                    if let Some(layer) = workspace.window_selector_view.layer_for_window(window_id)
                    {
                        workspace
                            .window_selector_view
                            .windows_layer
                            .add_sublayer(&layer);
                    }
                }
                if update {
                    self.update_workspace_model();
                }
            }
        } else {
            tracing::warn!("workspaces::raise_element: window not found");
        }
    }

    /// Raise all the windows of a given app
    /// returns the window id of the last window raised, if any
    fn raise_app_elements(
        &mut self,
        app_id: &str,
        focus_window: Option<&ObjectId>,
    ) -> Option<ObjectId> {
        // for every window in the app, raise it
        let windows = self.get_app_windows(app_id);
        let mut focus_wid = None;
        for (i, window_id) in windows.iter().enumerate() {
            if let Some(we) = self.get_window_for_surface(window_id) {
                if !we.is_minimised() {
                    if i == windows.len() - 1 {
                        self.raise_element(window_id, true, false);
                        focus_wid = Some(window_id.clone());
                    } else {
                        self.raise_element(window_id, false, false);
                    }
                } else {
                    // if minimised and there is only one window in the app, unminimize it
                    if windows.len() == 1 {
                        self.unminimize_window(window_id);
                    }
                }
            }
        }
        if let Some(wid) = focus_window {
            if let Some(we) = self.get_window_for_surface(wid) {
                if !we.is_minimised() {
                    self.raise_element(wid, true, false);
                    focus_wid = Some(wid.clone());
                }
            }
        }

        focus_wid
    }

    /// Raise all the windows of a given app
    /// returns the window id of the last window raised, and set to active, if any
    pub fn focus_app(&mut self, app_id: &str) -> Option<ObjectId> {
        tracing::trace!("workspaces::focus_app: {:?}", app_id);

        // Hide show desktop when focusing an app (use -2.0 to force dismiss)
        if self.get_show_desktop() {
            self.expose_show_desktop(-2.0, true);
        }

        let wid = self.raise_app_elements(app_id, None);
        if wid.is_none() {
            // return early
            return wid;
        }
        let wid = wid.unwrap();
        let current_space_index = self.with_model(|m| m.current_workspace);
        let index = self
            .spaces
            .iter()
            .position(|s| s.elements().any(|e| e.id() == wid))
            .unwrap_or(current_space_index);

        self.set_current_workspace_index(index, None);

        Some(wid)
    }

    pub fn focus_app_with_window(&mut self, wid: &ObjectId) -> Option<ObjectId> {
        let app_id = self
            .get_window_for_surface(wid)
            .map(|w| w.xdg_app_id())
            .unwrap_or_default();
        tracing::trace!("workspaces::focus_app_with_window {:?}", app_id);

        // Hide show desktop when focusing an app (use -2.0 to force dismiss)
        if self.get_show_desktop() {
            self.expose_show_desktop(-2.0, true);
        }

        let wid = self.raise_app_elements(&app_id, Some(wid));
        if wid.is_none() {
            // return early
            return wid;
        }
        let wid = wid.unwrap();
        let current_space_index = self.with_model(|m| m.current_workspace);
        let index = self
            .spaces
            .iter()
            .position(|s| s.elements().any(|e| e.id() == wid))
            .unwrap_or(current_space_index);

        self.set_current_workspace_index(index, None);

        Some(wid)
    }

    /// Update the workspace model using elements from Space: windows_list, app_windows_map, zindex_application_list
    /// - app_windows_map: is a map of app_id to a list of toplevel surfaces
    /// - applications_list: is the list of app_id in the order they are opened
    /// - zindex_application_list: is the list of app_id in the order they are in the zindex
    pub(crate) fn update_workspace_model(&self) {
        let windows: Vec<(ObjectId, WindowElement)> = self
            .spaces_elements()
            .map(|we| (we.wl_surface().unwrap().id(), we.clone()))
            .collect();

        {
            // reset the model
            if let Ok(mut model_mut) = self.model.write() {
                model_mut.zindex_application_list = Vec::new();
                model_mut.app_windows_map = HashMap::new();
            } else {
                return;
            }
        }

        let mut app_set = HashSet::new();
        for (window_id, we) in windows.iter() {
            let raw_app_id = we.xdg_app_id();
            let display_app_id = we.display_app_id(&self.display_handle);

            // Skip only if both raw and display app_id are empty
            if raw_app_id.is_empty() && display_app_id.is_empty() {
                tracing::warn!("[update_workspace_model] Skipping window with no app_id");
                continue;
            }
            if let Ok(mut model_mut) = self.model.write() {
                // Use raw_app_id for window mapping if available, otherwise use display_app_id
                let map_key = if !raw_app_id.is_empty() {
                    raw_app_id.clone()
                } else {
                    display_app_id.clone()
                };

                model_mut
                    .app_windows_map
                    .entry(map_key)
                    .or_default()
                    .push(window_id.clone());

                // Use display_app_id for UI lists (shows actual programs)
                if !model_mut.application_list.contains(&display_app_id) {
                    model_mut
                        .application_list
                        .push_front(display_app_id.clone());
                }
                if app_set.insert(display_app_id.clone()) {
                    model_mut
                        .zindex_application_list
                        .push(display_app_id.clone());
                }
            }
        }

        // keep only app in application_list that are in zindex_application_list
        {
            let mut model: std::sync::RwLockWriteGuard<'_, WorkspacesModel> =
                self.model.write().unwrap();

            let app_list = model.zindex_application_list.clone();
            {
                // update app list
                model
                    .application_list
                    .retain(|app_id| app_list.contains(app_id));
            }

            {
                // update minimized windows
                model
                    .minimized_windows
                    .retain(|(id, _)| windows.iter().any(|(wid, _)| wid == id));
            }
        }

        let model = self.model.read().unwrap();
        let event = model.clone();

        self.notify_observers(&event);
    }

    /// Returns all the window elements from all the spaces
    /// starting from current space
    pub fn spaces_elements(&self) -> impl DoubleEndedIterator<Item = &WindowElement> {
        let current_space = self.space();

        self.spaces
            .iter()
            .filter(move |s| s != &current_space)
            .chain(std::iter::once(current_space))
            .flat_map(|space| space.elements())
    }

    // Outputs Management

    /// Returns the list of outputs associated with the current workspace
    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.space().outputs()
    }

    /// Attach a new output to every workspace
    pub fn map_output(
        &mut self,
        output: &Output,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
    ) {
        self.map_output_with_primary(output, location, false);
    }

    /// Attach a new output to every workspace, optionally marking it as primary.
    pub fn map_output_with_primary(
        &mut self,
        output: &Output,
        location: impl Into<smithay::utils::Point<i32, smithay::utils::Logical>>,
        is_primary: bool,
    ) {
        let location = location.into();

        self.outputs.push(output.clone());
        if is_primary || self.primary_output.is_none() {
            self.primary_output = Some(output.clone());
        }
        // add the new output to every space
        for space in self.spaces.iter_mut() {
            space.map_output(output, location);
        }
    }

    /// Returns the primary output (used for dock placement and hot zone).
    pub fn primary_output(&self) -> Option<&Output> {
        self.primary_output.as_ref()
    }

    /// Detach an output from every workspace
    pub fn unmap_output(&mut self, output: &Output) {
        self.outputs.retain(|o| o != output);
        if self.primary_output.as_ref() == Some(output) {
            self.primary_output = self.outputs.first().cloned();
        }
        // remove the new output from every space
        for space in self.spaces.iter_mut() {
            space.unmap_output(output);
        }
    }

    // Workspaces Management

    pub fn add_workspace(&mut self) -> (usize, Arc<WorkspaceView>) {
        let mut new_space = Space::default();

        if !self.spaces.is_empty() {
            // init the space with the current outputs
            let current_space = self.space();
            for output in self.outputs.iter() {
                let geo = current_space.output_geometry(output).unwrap();
                new_space.map_output(output, geo.loc);
            }
        }

        self.spaces.push(new_space);
        let (index, workspace) = self.with_model_mut(|m| {
            m.workspace_counter += 1;

            let workspace = Arc::new(WorkspaceView::new(
                m.workspace_counter,
                self.layers_engine.clone(),
                &self.workspaces_layer,
                self.overlay_layer.clone(),
            ));
            self.expose_layer
                .add_sublayer(&workspace.window_selector_view.layer);

            m.workspaces.push(workspace.clone());
            self.notify_observers(m);
            (m.workspaces.len() - 1, workspace)
        });
        self.update_workspaces_layout();
        (index, workspace)
    }

    pub fn get_next_free_workspace(&mut self) -> (usize, Arc<WorkspaceView>) {
        let current_workspace = self.get_current_workspace_index();
        if current_workspace < self.spaces.len() - 1 {
            for i in current_workspace + 1..self.spaces.len() {
                if self.spaces[i].elements().count() == 0 {
                    return (i, self.with_model(|m| m.workspaces[i].clone()));
                }
            }
        }
        self.add_workspace()
    }

    pub fn remove_workspace_at(&mut self, n: usize) {
        if self.spaces.len() == 1 {
            return;
        }

        let workspace_model = self.with_model_mut(|m| {
            if m.workspaces.len() == 1 {
                return m.clone();
            }

            if n < m.workspaces.len() {
                m.workspaces.remove(n);
                if m.current_workspace >= m.workspaces.len() {
                    m.current_workspace = m.workspaces.len() - 1;
                }
            }
            m.clone()
        });

        if n < self.spaces.len() {
            if let Some(ws) = self.get_workspace_at(n) {
                // Allow removal of fullscreen workspaces only if they have no windows (dangling state)
                let window_count = self.spaces[n].elements().count();
                if ws.get_fullscreen_mode() && window_count > 0 {
                    // Do not remove a fullscreen workspace that still has windows
                    return;
                }
            }
            // move all windows to previous workspace
            let space_to_remove = self.spaces.remove(n);
            for e in space_to_remove.elements() {
                let location = space_to_remove.element_location(e).unwrap_or_default();
                // Drop fullscreen state so the window restores to its normal size on the target workspace
                if e.is_fullscreen() {
                    e.set_fullscreen(false, workspace_model.current_workspace);

                    if let Some(ws) = self.get_workspace_at(workspace_model.current_workspace) {
                        ws.set_fullscreen_mode(false);
                        ws.set_fullscreen_animating(false);
                        ws.set_name(None);
                    }
                }
                self.move_window_to_workspace(e, workspace_model.current_workspace, location);
            }

            // If expose is visible, rebuild its layout to reflect the moved windows
            if self.get_show_all() {
                self.expose_update_if_needed_workspace(workspace_model.current_workspace);
            }
        }
        self.update_workspaces_layout();
        self.notify_observers(&workspace_model);
    }

    pub fn get_workspace_at(&self, i: usize) -> Option<Arc<WorkspaceView>> {
        self.with_model(|m| m.workspaces.get(i).cloned())
    }

    pub fn get_current_workspace(&self) -> Arc<WorkspaceView> {
        self.with_model(|m| m.workspaces[m.current_workspace].clone())
    }

    pub fn get_current_workspace_index(&self) -> usize {
        self.with_model(|m| m.current_workspace)
    }

    /// Get the top (non-minimized) window of a workspace, or None if the workspace is empty.
    pub fn get_top_window_of_workspace(&self, workspace_index: usize) -> Option<ObjectId> {
        if workspace_index >= self.spaces.len() {
            return None;
        }
        self.spaces[workspace_index].elements().rev().find_map(|e| {
            let id = e.id();
            if let Some(window) = self.windows_map.get(&id) {
                if window.is_minimised() {
                    return None;
                }
            }
            Some(id)
        })
    }

    /// Given a workspace view index (WorkspaceView.index), return its current
    /// position in the workspaces vector (zero-based). Useful when external
    /// components keep the view index while the internal ordering may change.
    pub fn workspace_position_by_view_index(&self, workspace_index: usize) -> Option<usize> {
        self.with_model(|m| {
            m.workspaces
                .iter()
                .position(|ws| ws.index == workspace_index)
        })
    }
    pub fn set_current_workspace_index(
        &mut self,
        i: usize,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        if i > self.spaces.len() - 1 {
            return None;
        }
        self.with_model_mut(|m| {
            if i > m.workspaces.len() - 1 {
                return;
            }
            m.current_workspace = i;
        });
        self.update_workspace_model();
        self.scroll_to_workspace_index(i, transition)
    }
    /// Scroll to the workspace at index i, default transition is 1.0s spring
    fn scroll_to_workspace_index(
        &self,
        i: usize,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        let transition = transition.unwrap_or(Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(Spring::with_duration_and_bounce(1.0, 0.1)),
        });
        let mut x = 0.0;
        if let Some(workspace) = self.get_workspace_at(i) {
            // Control dock visibility based on workspace fullscreen state
            // Only skip dock control when actively IN expose mode (show_all)

            // Don't use hide/show during:
            // - expose mode or expose transitions (let expose system control position)
            // - fullscreen animations (let fullscreen transition complete smoothly)
            if !self.get_show_all() {
                if workspace.get_fullscreen_mode() {
                    self.dock.hide(Some(transition));
                } else if !self.dock.is_autohide_enabled() {
                    // With autohide on, dock visibility is managed by the hot zone /
                    // show_autohide(); calling show() would hide it (see DockView::show).
                    self.dock.show(Some(transition));
                }
            }

            // Animate layer_shell_overlay based on target workspace fullscreen state
            let target_opacity = if workspace.get_fullscreen_mode() {
                0.0
            } else {
                1.0
            };
            self.layer_shell_overlay
                .set_opacity(target_opacity, Some(transition));

            if self.get_show_all() {
                // In expose mode, ensure the target workspace has its layout calculated
                // and windows positioned, but don't animate dock (it's shared across workspaces)
                self.expose_update_if_needed_workspace(i);
            }

            let (workspace_width, scale) = self.with_model(|m| (m.width as f32, m.scale as f32));
            if workspace_width > 0.0 {
                // Account for spacing between workspaces (WORKSPACE_SPACING is logical, scale to physical)
                x = i as f32 * (workspace_width + WORKSPACE_SPACING * scale);
            } else {
                x = workspace
                    .workspace_layer
                    .render_layer()
                    .local_transformed_bounds
                    .left();
            }
        }

        self.apply_scroll_offset(x, Some(transition))
    }

    /// Update workspace position during 3-finger horizontal swipe gesture.
    /// Applies delta immediately (no animation) with rubber-band resistance at edges.
    pub fn workspace_swipe_update(&self, delta_x: f32) {
        let (num_workspaces, workspace_width, scale) =
            self.with_model(|m| (m.workspaces.len(), m.width as f32, m.scale as f32));

        if num_workspaces == 0 || workspace_width <= 0.0 {
            return;
        }

        // Get current scroll position (negated because layer position is negative of scroll offset)
        let current_pos = self.workspaces_layer.render_position();
        let current_offset = -current_pos.x;

        // Calculate new offset - delta is in logical pixels, convert to physical
        // Apply dampening factor to reduce sensitivity and prevent overshooting
        const SWIPE_DAMPENING: f32 = 0.6;
        let physical_delta = delta_x * scale * SWIPE_DAMPENING;

        // Calculate bounds - account for spacing between workspaces (WORKSPACE_SPACING is logical, scale to physical)
        let max_offset = (num_workspaces - 1) as f32 * (workspace_width + WORKSPACE_SPACING * scale);

        // Apply rubber-band resistance when already past boundaries
        // Resistance increases progressively the further past the edge we are
        let new_offset = if current_offset < 0.0 {
            // Already past first workspace - apply progressive resistance to delta
            let distance_past_edge = -current_offset;
            let resistance_factor = 1.0 / (1.0 + distance_past_edge / 100.0);
            current_offset - (physical_delta * resistance_factor)
        } else if current_offset > max_offset {
            // Already past last workspace - apply progressive resistance to delta
            let distance_past_edge = current_offset - max_offset;
            let resistance_factor = 1.0 / (1.0 + distance_past_edge / 100.0);
            current_offset - (physical_delta * resistance_factor)
        } else {
            // Within bounds - normal scrolling
            current_offset - physical_delta
        };

        // Apply immediately without animation
        self.workspaces_layer.set_position((-new_offset, 0.0), None);
        self.expose_layer.set_position((-new_offset, 0.0), None);
    }

    /// End workspace swipe gesture and snap to nearest workspace.
    /// Uses velocity to determine target workspace for natural momentum-based snapping.
    /// Returns the target workspace index.
    pub fn workspace_swipe_end(&mut self, velocity: f32) -> usize {
        let (num_workspaces, workspace_width, current_index, scale) = self.with_model(|m| {
            (
                m.workspaces.len(),
                m.width as f32,
                m.current_workspace,
                m.scale as f32,
            )
        });

        if num_workspaces == 0 || workspace_width <= 0.0 {
            // Just snap to current
            let _ = self.set_current_workspace_index(current_index, None);
            return current_index;
        }

        // Get current scroll position
        let current_pos = self.workspaces_layer.render_position();
        let current_offset = -current_pos.x;

        // Convert velocity to physical units
        let physical_velocity = velocity * scale;

        // Velocity threshold for momentum-based switching (pixels per update event)
        // Typical trackpad sends ~60 events/sec, so 15px/event ≈ 900px/sec
        const VELOCITY_THRESHOLD: f32 = 15.0;

        let target_index = if physical_velocity.abs() > VELOCITY_THRESHOLD {
            // Velocity-based: switch in direction of swipe
            if physical_velocity > 0.0 {
                // Swiping right (moving content left) -> go to previous workspace
                current_index.saturating_sub(1)
            } else {
                // Swiping left (moving content right) -> go to next workspace
                (current_index + 1).min(num_workspaces - 1)
            }
        } else {
            // Position-based: snap to nearest workspace
            // Account for spacing when calculating progress (WORKSPACE_SPACING is logical, scale to physical)
            let progress = current_offset / (workspace_width + WORKSPACE_SPACING * scale);
            let nearest = progress.round() as usize;
            nearest.min(num_workspaces - 1)
        };

        // Use a snappy spring transition for the final animation
        let transition = Transition {
            delay: 0.0,
            timing: TimingFunction::Spring(Spring::with_duration_and_bounce(0.5, 0.05)),
        };

        let _ = self.set_current_workspace_index(target_index, Some(transition));
        target_index
    }

    // Space management

    pub fn outputs_for_element(&self, element: &WindowElement) -> Vec<Output> {
        self.space().outputs_for_element(element)
    }

    fn apply_scroll_offset(
        &self,
        offset: f32,
        transition: Option<Transition>,
    ) -> Option<TransactionRef> {
        if !offset.is_finite() {
            return None;
        }
        if let Some(transition) = &transition {
            // Mark as animating
            self.is_animating
                .store(true, std::sync::atomic::Ordering::Relaxed);

            let animation = self
                .workspaces_layer
                .engine
                .add_animation_from_transition(transition, true);
            let change1 = self.workspaces_layer.change_position((-offset, 0.0));
            let change2 = self.expose_layer.change_position((-offset, 0.0));
            let changes = vec![change1, change2];
            let tr = self
                .workspaces_layer
                .engine
                .schedule_changes(&changes, animation)
                .into_iter()
                .next();

            // Clear animating flag when animation completes
            if let Some(tr) = &tr {
                let is_animating = self.is_animating.clone();

                let workspace_view = self.get_current_workspace();
                let window_selector_overlay =
                    workspace_view.window_selector_view.overlay_layer.clone();
                let show_all = self.get_show_all();
                tr.on_finish(
                    move |_: &Layer, _: f32| {
                        is_animating.store(false, std::sync::atomic::Ordering::Relaxed);

                        // Ensure overlay is visible after workspace scroll animation
                        if show_all {
                            window_selector_overlay.set_opacity(1.0, None);
                        }
                    },
                    true,
                );
            }

            return tr;
        }
        None
    }

    pub fn element_under(
        &self,
        point: impl Into<smithay::utils::Point<f64, smithay::utils::Logical>>,
    ) -> Option<(
        &WindowElement,
        smithay::utils::Point<i32, smithay::utils::Logical>,
    )> {
        self.space().element_under(point)
    }

    pub fn output_geometry(
        &self,
        output: &Output,
    ) -> Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> {
        self.space().output_geometry(output)
    }

    pub fn refresh_space(&mut self) {
        self.space_mut().refresh();
    }

    pub fn element_location(
        &self,
        we: &WindowElement,
    ) -> Option<smithay::utils::Point<i32, smithay::utils::Logical>> {
        self.space().element_location(we)
    }

    pub fn output_under<P: Into<smithay::utils::Point<f64, smithay::utils::Logical>>>(
        &self,
        point: P,
    ) -> impl Iterator<Item = &Output> {
        self.space().output_under(point)
    }

    pub fn element_geometry(
        &self,
        we: &WindowElement,
    ) -> Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> {
        self.space().element_geometry(we)
    }

    // Add these helper methods
    #[allow(dead_code)]
    fn find_space_for_element(&self, element: &WindowElement) -> Option<&Space<WindowElement>> {
        self.spaces
            .iter()
            .find(|space| space.elements().any(|e| e.id() == element.id()))
    }

    #[allow(dead_code)]
    fn find_space_index_for_element(&self, element: &WindowElement) -> Option<usize> {
        self.spaces
            .iter()
            .position(|space| space.elements().any(|e| e.id() == element.id()))
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Layer Shell Support
    // ─────────────────────────────────────────────────────────────────────────────

    /// Create a new lay_rs layer for a layer shell surface and add it to the appropriate container.
    /// Returns the new layer.
    pub fn create_layer_shell_layer(
        &self,
        wlr_layer: smithay::wayland::shell::wlr_layer::Layer,
        namespace: &str,
    ) -> Layer {
        use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;

        let layer = self.layers_engine.new_layer();
        layer.set_key(format!(
            "layer_shell_{}_{}",
            wlr_layer_to_str(wlr_layer),
            namespace
        ));
        layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        // Layer shell surfaces handle their own pointer events
        layer.set_pointer_events(true);

        // Add to appropriate container based on layer
        match wlr_layer {
            WlrLayer::Background => {
                self.layer_shell_background.add_sublayer(&layer);
            }
            WlrLayer::Bottom => {
                self.layer_shell_background.add_sublayer(&layer);
            }
            WlrLayer::Top => {
                self.layer_shell_top.add_sublayer(&layer);
            }
            WlrLayer::Overlay => {
                self.layer_shell_overlay.add_sublayer(&layer);
            }
        }

        layer
    }

    /// Remove a layer shell layer from the scene graph.
    pub fn remove_layer_shell_layer(&self, layer: &Layer) {
        layer.remove();
    }
}

/// Helper to convert WlrLayer to string for layer keys
fn wlr_layer_to_str(layer: smithay::wayland::shell::wlr_layer::Layer) -> &'static str {
    use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
    match layer {
        WlrLayer::Background => "background",
        WlrLayer::Bottom => "bottom",
        WlrLayer::Top => "top",
        WlrLayer::Overlay => "overlay",
    }
}

#[derive(Clone)]
struct UnminimizeContext {
    wid: ObjectId,
    workspace: Arc<WorkspaceView>,
    window: WindowElement,
    view: WindowView,
    dock: Arc<DockView>,
    layers_engine: Arc<Engine>,
    expose_layer: Layer,
    model: Arc<RwLock<WorkspacesModel>>,
    observers: Vec<Weak<dyn Observer<WorkspacesModel>>>,
    layer_pos: (f32, f32),
    pos_logical: (i32, i32),
}

impl UnminimizeContext {
    fn run(&self) {
        let wid = self.wid.clone();
        let workspace = self.workspace.clone();
        let window = self.window.clone();
        let view = self.view.clone();
        let dock = self.dock.clone();
        let layers_engine = self.layers_engine.clone();
        let expose_layer = self.expose_layer.clone();
        let model = self.model.clone();
        let observers = self.observers.clone();
        let layer_pos = self.layer_pos;
        let pos_logical = self.pos_logical;

        let event = {
            let mut model = model.write().unwrap();
            model.minimized_windows.retain(|(w, _title)| w != &wid);
            model.clone()
        };

        window.set_is_minimised(false);

        if let Some(drawer) = dock.remove_window_element(&wid) {
            let windows_layer_ref = workspace.windows_layer.clone();
            let expose_windows_ref = expose_layer.clone();
            let layer_ref = view.window_layer.clone();
            let mirror_ref = view.mirror_layer.clone();
            let target_pos = layer_pos;
            layer_ref.set_hidden(true);
            mirror_ref.set_hidden(true);

            // Clear any color filter that might have been applied during dock interaction
            layer_ref.set_color_filter(None);
            mirror_ref.set_color_filter(None);

            layers_engine.update(0.0);

            let drawer_bounds = drawer.render_bounds_transformed();
            drawer.clear_on_change_size_handlers();
            drawer
                .set_size(
                    Size::points(0.0, 130.0),
                    Transition {
                        delay: 0.2,
                        timing: TimingFunction::ease_out_quad(0.3),
                    },
                )
                .on_start(
                    move |_layer: &Layer, _| {
                        layer_ref.remove_draw_content();
                        windows_layer_ref.add_sublayer(&layer_ref);
                        expose_windows_ref.add_sublayer(&mirror_ref);
                        layer_ref.set_position(target_pos, None);
                    },
                    true,
                )
                .then(move |layer: &Layer, _| {
                    layer.remove();
                });

            view.unminimize(drawer_bounds);

            // Make sure the mirror layer is visible again for expose
            view.mirror_layer.set_hidden(false);
        }

        workspace.map_window(&window, (pos_logical.0, pos_logical.1).into(), None);

        crate::utils::notify_observers(&observers, &event);
    }
}

impl Observable<WorkspacesModel> for Workspaces {
    fn add_listener(&mut self, observer: std::sync::Arc<dyn Observer<WorkspacesModel>>) {
        let observer = std::sync::Arc::downgrade(&observer);
        self.observers.push(observer);
    }

    fn observers<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = std::sync::Weak<dyn Observer<WorkspacesModel>>> + 'a> {
        Box::new(self.observers.iter().cloned())
    }
}
